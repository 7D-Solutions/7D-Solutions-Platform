//! PO service — Guard → Mutation → Outbox DB operations.
//!
//! create_po:        creates a draft PO with lines; emits ap.po_created
//! update_po_lines:  replaces all lines on a draft PO (idempotent)
//! get_po:           fetches PO + lines by id (tenant-scoped)
//! list_pos:         lists PO headers with optional vendor/status filters

use chrono::{DateTime, Utc};
use sqlx::PgPool;
use uuid::Uuid;

use crate::events::{
    build_po_created_envelope, PoCreatedPayload, PoLine as EventPoLine, EVENT_TYPE_PO_CREATED,
};
use crate::outbox::enqueue_event_tx;

use super::{
    CreatePoLineRequest, CreatePoRequest, PoError, PoLineRecord, PurchaseOrder,
    PurchaseOrderWithLines, UpdatePoLinesRequest,
};

// ============================================================================
// Reads
// ============================================================================

/// Fetch a single PO with its lines. Returns None if not found for this tenant.
pub async fn get_po(
    pool: &PgPool,
    tenant_id: &str,
    po_id: Uuid,
) -> Result<Option<PurchaseOrderWithLines>, PoError> {
    let po: Option<PurchaseOrder> = sqlx::query_as(
        r#"
        SELECT po_id, tenant_id, vendor_id, po_number, currency,
               total_minor, status, created_by, created_at, expected_delivery_date
        FROM purchase_orders
        WHERE po_id = $1 AND tenant_id = $2
        "#,
    )
    .bind(po_id)
    .bind(tenant_id)
    .fetch_optional(pool)
    .await?;

    let Some(po) = po else {
        return Ok(None);
    };

    let lines = fetch_lines(pool, po_id).await?;
    Ok(Some(PurchaseOrderWithLines { po, lines }))
}

/// List PO headers for a tenant with optional vendor_id and status filters.
pub async fn list_pos(
    pool: &PgPool,
    tenant_id: &str,
    vendor_id: Option<Uuid>,
    status: Option<&str>,
) -> Result<Vec<PurchaseOrder>, PoError> {
    let pos = match (vendor_id, status) {
        (Some(vid), Some(s)) => {
            sqlx::query_as::<_, PurchaseOrder>(
                r#"SELECT po_id, tenant_id, vendor_id, po_number, currency,
                          total_minor, status, created_by, created_at, expected_delivery_date
                   FROM purchase_orders
                   WHERE tenant_id = $1 AND vendor_id = $2 AND status = $3
                   ORDER BY created_at DESC"#,
            )
            .bind(tenant_id).bind(vid).bind(s)
            .fetch_all(pool).await?
        }
        (Some(vid), None) => {
            sqlx::query_as::<_, PurchaseOrder>(
                r#"SELECT po_id, tenant_id, vendor_id, po_number, currency,
                          total_minor, status, created_by, created_at, expected_delivery_date
                   FROM purchase_orders
                   WHERE tenant_id = $1 AND vendor_id = $2
                   ORDER BY created_at DESC"#,
            )
            .bind(tenant_id).bind(vid)
            .fetch_all(pool).await?
        }
        (None, Some(s)) => {
            sqlx::query_as::<_, PurchaseOrder>(
                r#"SELECT po_id, tenant_id, vendor_id, po_number, currency,
                          total_minor, status, created_by, created_at, expected_delivery_date
                   FROM purchase_orders
                   WHERE tenant_id = $1 AND status = $2
                   ORDER BY created_at DESC"#,
            )
            .bind(tenant_id).bind(s)
            .fetch_all(pool).await?
        }
        (None, None) => {
            sqlx::query_as::<_, PurchaseOrder>(
                r#"SELECT po_id, tenant_id, vendor_id, po_number, currency,
                          total_minor, status, created_by, created_at, expected_delivery_date
                   FROM purchase_orders
                   WHERE tenant_id = $1
                   ORDER BY created_at DESC"#,
            )
            .bind(tenant_id)
            .fetch_all(pool).await?
        }
    };
    Ok(pos)
}

async fn fetch_lines(pool: &PgPool, po_id: Uuid) -> Result<Vec<PoLineRecord>, PoError> {
    let lines = sqlx::query_as::<_, PoLineRecord>(
        r#"
        SELECT line_id, po_id, description,
               quantity::FLOAT8 AS quantity,
               unit_of_measure, unit_price_minor, line_total_minor,
               gl_account_code, created_at
        FROM po_lines
        WHERE po_id = $1
        ORDER BY created_at ASC
        "#,
    )
    .bind(po_id)
    .fetch_all(pool)
    .await?;
    Ok(lines)
}

// ============================================================================
// Writes
// ============================================================================

/// Create a draft PO with line items. Emits `ap.po_created` via the outbox.
///
/// Guard:    vendor must exist and be active for this tenant.
/// Mutation: purchase_orders header + po_lines + po_status audit row.
/// Outbox:   ap.po_created envelope enqueued atomically.
pub async fn create_po(
    pool: &PgPool,
    tenant_id: &str,
    req: &CreatePoRequest,
    correlation_id: String,
) -> Result<PurchaseOrderWithLines, PoError> {
    req.validate()?;

    // Guard: vendor must exist and be active
    let vendor_exists: Option<(Uuid,)> = sqlx::query_as(
        "SELECT vendor_id FROM vendors WHERE vendor_id = $1 AND tenant_id = $2 AND is_active = TRUE",
    )
    .bind(req.vendor_id)
    .bind(tenant_id)
    .fetch_optional(pool)
    .await?;

    if vendor_exists.is_none() {
        return Err(PoError::VendorNotFound(req.vendor_id));
    }

    let po_id = Uuid::new_v4();
    let event_id = Uuid::new_v4();
    let now = Utc::now();

    // PO number: PO-YYYYMMDD-{first 8 hex chars of UUID}
    let po_number = format!(
        "PO-{}-{}",
        now.format("%Y%m%d"),
        &po_id.simple().to_string()[..8].to_uppercase()
    );

    let total_minor: i64 = req.lines.iter().map(|l| l.line_total_minor()).sum();

    let mut tx = pool.begin().await?;

    // Mutation: insert PO header (status = draft)
    let po: PurchaseOrder = sqlx::query_as(
        r#"
        INSERT INTO purchase_orders (
            po_id, tenant_id, vendor_id, po_number, currency,
            total_minor, status, created_by, created_at, expected_delivery_date
        )
        VALUES ($1, $2, $3, $4, $5, $6, 'draft', $7, $8, $9)
        RETURNING
            po_id, tenant_id, vendor_id, po_number, currency,
            total_minor, status, created_by, created_at, expected_delivery_date
        "#,
    )
    .bind(po_id)
    .bind(tenant_id)
    .bind(req.vendor_id)
    .bind(&po_number)
    .bind(req.currency.to_uppercase())
    .bind(total_minor)
    .bind(req.created_by.trim())
    .bind(now)
    .bind(req.expected_delivery_date)
    .fetch_one(&mut *tx)
    .await?;

    // Mutation: append draft entry to status audit log
    sqlx::query(
        "INSERT INTO po_status (po_id, status, changed_by, changed_at) VALUES ($1, 'draft', $2, $3)",
    )
    .bind(po_id)
    .bind(req.created_by.trim())
    .bind(now)
    .execute(&mut *tx)
    .await?;

    // Mutation: insert lines
    let (po_lines, event_lines) = insert_lines(&mut tx, po_id, &req.lines, now).await?;

    // Outbox: ap.po_created
    let payload = PoCreatedPayload {
        po_id,
        tenant_id: tenant_id.to_string(),
        vendor_id: req.vendor_id,
        po_number: po_number.clone(),
        currency: req.currency.to_uppercase(),
        lines: event_lines,
        total_minor,
        created_by: req.created_by.trim().to_string(),
        created_at: now,
        expected_delivery_date: req.expected_delivery_date,
    };

    let envelope = build_po_created_envelope(
        event_id,
        tenant_id.to_string(),
        correlation_id,
        None,
        payload,
    );

    enqueue_event_tx(&mut tx, event_id, EVENT_TYPE_PO_CREATED, "po", &po_id.to_string(), &envelope)
        .await?;

    tx.commit().await?;

    Ok(PurchaseOrderWithLines { po, lines: po_lines })
}

/// Replace all lines on a draft PO (idempotent full replacement).
///
/// Only permitted when PO is in 'draft' status — returns PoError::NotDraft otherwise.
/// Recalculates and stores the new total_minor after replacement.
pub async fn update_po_lines(
    pool: &PgPool,
    tenant_id: &str,
    po_id: Uuid,
    req: &UpdatePoLinesRequest,
) -> Result<PurchaseOrderWithLines, PoError> {
    req.validate()?;

    let mut tx = pool.begin().await?;

    // Guard: PO must exist for this tenant and be in draft
    let po: Option<PurchaseOrder> = sqlx::query_as(
        r#"
        SELECT po_id, tenant_id, vendor_id, po_number, currency,
               total_minor, status, created_by, created_at, expected_delivery_date
        FROM purchase_orders
        WHERE po_id = $1 AND tenant_id = $2
        FOR UPDATE
        "#,
    )
    .bind(po_id)
    .bind(tenant_id)
    .fetch_optional(&mut *tx)
    .await?;

    let po = po.ok_or(PoError::NotFound(po_id))?;

    if po.status != "draft" {
        return Err(PoError::NotDraft(po.status.clone()));
    }

    let now = Utc::now();
    let total_minor: i64 = req.lines.iter().map(|l| l.line_total_minor()).sum();

    // Mutation: replace all lines (delete + re-insert = idempotent)
    sqlx::query("DELETE FROM po_lines WHERE po_id = $1")
        .bind(po_id)
        .execute(&mut *tx)
        .await?;

    let (new_lines, _) = insert_lines(&mut tx, po_id, &req.lines, now).await?;

    // Mutation: update PO total to reflect new lines
    let updated_po: PurchaseOrder = sqlx::query_as(
        r#"
        UPDATE purchase_orders
        SET total_minor = $1
        WHERE po_id = $2 AND tenant_id = $3
        RETURNING
            po_id, tenant_id, vendor_id, po_number, currency,
            total_minor, status, created_by, created_at, expected_delivery_date
        "#,
    )
    .bind(total_minor)
    .bind(po_id)
    .bind(tenant_id)
    .fetch_one(&mut *tx)
    .await?;

    tx.commit().await?;

    Ok(PurchaseOrderWithLines { po: updated_po, lines: new_lines })
}

// ============================================================================
// Helpers
// ============================================================================

/// Insert PO lines within a caller-owned transaction.
/// Returns (DB records, event lines).
async fn insert_lines(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    po_id: Uuid,
    lines: &[CreatePoLineRequest],
    now: DateTime<Utc>,
) -> Result<(Vec<PoLineRecord>, Vec<EventPoLine>), PoError> {
    let mut db_lines = Vec::with_capacity(lines.len());
    let mut event_lines = Vec::with_capacity(lines.len());

    for line_req in lines {
        let line_id = Uuid::new_v4();
        let description = line_req.effective_description();
        let line_total = line_req.line_total_minor();

        let line: PoLineRecord = sqlx::query_as(
            r#"
            INSERT INTO po_lines (
                line_id, po_id, description, quantity, unit_of_measure,
                unit_price_minor, line_total_minor, gl_account_code, created_at
            )
            VALUES ($1, $2, $3, $4::NUMERIC, $5, $6, $7, $8, $9)
            RETURNING
                line_id, po_id, description,
                quantity::FLOAT8 AS quantity,
                unit_of_measure, unit_price_minor, line_total_minor,
                gl_account_code, created_at
            "#,
        )
        .bind(line_id)
        .bind(po_id)
        .bind(&description)
        .bind(line_req.quantity)
        .bind(&line_req.unit_of_measure)
        .bind(line_req.unit_price_minor)
        .bind(line_total)
        .bind(&line_req.gl_account_code)
        .bind(now)
        .fetch_one(&mut **tx)
        .await?;

        event_lines.push(EventPoLine {
            line_id,
            description: description.clone(),
            quantity: line_req.quantity,
            unit_of_measure: line_req.unit_of_measure.clone(),
            unit_price_minor: line_req.unit_price_minor,
            gl_account_code: line_req.gl_account_code.clone(),
        });
        db_lines.push(line);
    }

    Ok((db_lines, event_lines))
}

// ============================================================================
// Integrated Tests (real DB, no mocks)
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;

    const TEST_TENANT: &str = "test-tenant-pos";

    fn test_db_url() -> String {
        std::env::var("DATABASE_URL")
            .unwrap_or_else(|_| "postgres://ap_user:ap_pass@localhost:5443/ap_db".to_string())
    }

    async fn test_pool() -> PgPool {
        PgPool::connect(&test_db_url())
            .await
            .expect("Failed to connect to AP test database")
    }

    async fn create_test_vendor(pool: &PgPool) -> Uuid {
        let vendor_id = Uuid::new_v4();
        sqlx::query(
            r#"INSERT INTO vendors (vendor_id, tenant_id, name, currency, payment_terms_days,
               is_active, created_at, updated_at)
               VALUES ($1, $2, $3, 'USD', 30, TRUE, NOW(), NOW())"#,
        )
        .bind(vendor_id)
        .bind(TEST_TENANT)
        .bind(format!("Test Vendor PO {}", vendor_id))
        .execute(pool)
        .await
        .expect("insert test vendor failed");
        vendor_id
    }

    async fn cleanup(pool: &PgPool) {
        sqlx::query(
            "DELETE FROM events_outbox WHERE aggregate_type = 'po' \
             AND aggregate_id IN (SELECT po_id::TEXT FROM purchase_orders WHERE tenant_id = $1)",
        )
        .bind(TEST_TENANT).execute(pool).await.ok();

        sqlx::query(
            "DELETE FROM po_status WHERE po_id IN \
             (SELECT po_id FROM purchase_orders WHERE tenant_id = $1)",
        )
        .bind(TEST_TENANT).execute(pool).await.ok();

        sqlx::query(
            "DELETE FROM po_lines WHERE po_id IN \
             (SELECT po_id FROM purchase_orders WHERE tenant_id = $1)",
        )
        .bind(TEST_TENANT).execute(pool).await.ok();

        sqlx::query("DELETE FROM purchase_orders WHERE tenant_id = $1")
            .bind(TEST_TENANT).execute(pool).await.ok();

        sqlx::query(
            "DELETE FROM events_outbox WHERE aggregate_type = 'vendor' \
             AND aggregate_id IN (SELECT vendor_id::TEXT FROM vendors WHERE tenant_id = $1)",
        )
        .bind(TEST_TENANT).execute(pool).await.ok();

        sqlx::query("DELETE FROM vendors WHERE tenant_id = $1")
            .bind(TEST_TENANT).execute(pool).await.ok();
    }

    fn sample_req(vendor_id: Uuid) -> CreatePoRequest {
        CreatePoRequest {
            vendor_id,
            currency: "USD".to_string(),
            created_by: "user-ap".to_string(),
            expected_delivery_date: None,
            lines: vec![
                CreatePoLineRequest {
                    item_id: None,
                    description: Some("Office chairs".to_string()),
                    quantity: 10.0,
                    unit_of_measure: "each".to_string(),
                    unit_price_minor: 45_000,
                    gl_account_code: "6100".to_string(),
                },
                CreatePoLineRequest {
                    item_id: Some(Uuid::new_v4()),
                    description: None,
                    quantity: 5.0,
                    unit_of_measure: "each".to_string(),
                    unit_price_minor: 10_000,
                    gl_account_code: "6200".to_string(),
                },
            ],
        }
    }

    #[tokio::test]
    #[serial]
    async fn test_create_po_draft_with_lines() {
        let pool = test_pool().await;
        cleanup(&pool).await;
        let vendor_id = create_test_vendor(&pool).await;

        let req = sample_req(vendor_id);
        let result = create_po(&pool, TEST_TENANT, &req, "corr-1".to_string())
            .await
            .expect("create_po failed");

        assert_eq!(result.po.vendor_id, vendor_id);
        assert_eq!(result.po.status, "draft");
        assert_eq!(result.po.currency, "USD");
        // total = 10*45000 + 5*10000 = 500000
        assert_eq!(result.po.total_minor, 500_000);
        assert_eq!(result.lines.len(), 2);
        assert!(result.po.po_number.starts_with("PO-"));

        // second line stored as item:{uuid}
        assert!(result.lines[1].description.starts_with("item:"));

        cleanup(&pool).await;
    }

    #[tokio::test]
    #[serial]
    async fn test_create_po_vendor_not_found() {
        let pool = test_pool().await;
        cleanup(&pool).await;

        let req = sample_req(Uuid::new_v4());
        let result = create_po(&pool, TEST_TENANT, &req, "corr-x".to_string()).await;
        assert!(
            matches!(result, Err(PoError::VendorNotFound(_))),
            "expected VendorNotFound, got {:?}",
            result
        );
    }

    #[tokio::test]
    #[serial]
    async fn test_get_po_returns_with_lines() {
        let pool = test_pool().await;
        cleanup(&pool).await;
        let vendor_id = create_test_vendor(&pool).await;

        let created = create_po(&pool, TEST_TENANT, &sample_req(vendor_id), "corr-2".to_string())
            .await
            .expect("create failed");

        let fetched = get_po(&pool, TEST_TENANT, created.po.po_id)
            .await
            .expect("get_po failed");

        assert!(fetched.is_some());
        let powi = fetched.unwrap();
        assert_eq!(powi.po.po_id, created.po.po_id);
        assert_eq!(powi.lines.len(), 2);

        cleanup(&pool).await;
    }

    #[tokio::test]
    #[serial]
    async fn test_get_po_wrong_tenant_returns_none() {
        let pool = test_pool().await;
        cleanup(&pool).await;
        let vendor_id = create_test_vendor(&pool).await;

        let created = create_po(&pool, TEST_TENANT, &sample_req(vendor_id), "corr-3".to_string())
            .await
            .expect("create failed");

        let result = get_po(&pool, "other-tenant", created.po.po_id)
            .await
            .expect("get_po error");
        assert!(result.is_none());

        cleanup(&pool).await;
    }

    #[tokio::test]
    #[serial]
    async fn test_update_po_lines_replaces_all_idempotent() {
        let pool = test_pool().await;
        cleanup(&pool).await;
        let vendor_id = create_test_vendor(&pool).await;

        let created = create_po(&pool, TEST_TENANT, &sample_req(vendor_id), "corr-4".to_string())
            .await
            .expect("create failed");

        let update_req = UpdatePoLinesRequest {
            updated_by: "user-ap".to_string(),
            lines: vec![CreatePoLineRequest {
                item_id: None,
                description: Some("Replacement item".to_string()),
                quantity: 2.0,
                unit_of_measure: "each".to_string(),
                unit_price_minor: 20_000,
                gl_account_code: "6300".to_string(),
            }],
        };

        let updated = update_po_lines(&pool, TEST_TENANT, created.po.po_id, &update_req)
            .await
            .expect("update_po_lines failed");

        assert_eq!(updated.lines.len(), 1);
        assert_eq!(updated.po.total_minor, 40_000); // 2 * 20000
        assert_eq!(updated.lines[0].description, "Replacement item");

        // Calling again with same request is idempotent
        let updated2 = update_po_lines(&pool, TEST_TENANT, created.po.po_id, &update_req)
            .await
            .expect("second update failed");
        assert_eq!(updated2.lines.len(), 1);
        assert_eq!(updated2.po.total_minor, 40_000);

        cleanup(&pool).await;
    }

    #[tokio::test]
    #[serial]
    async fn test_update_po_lines_rejected_for_non_draft() {
        let pool = test_pool().await;
        cleanup(&pool).await;
        let vendor_id = create_test_vendor(&pool).await;

        let created = create_po(&pool, TEST_TENANT, &sample_req(vendor_id), "corr-5".to_string())
            .await
            .expect("create failed");

        // Manually advance status to approved
        sqlx::query("UPDATE purchase_orders SET status = 'approved' WHERE po_id = $1")
            .bind(created.po.po_id)
            .execute(&pool)
            .await
            .expect("status update failed");

        let update_req = UpdatePoLinesRequest {
            updated_by: "user-ap".to_string(),
            lines: vec![CreatePoLineRequest {
                item_id: None,
                description: Some("New item".to_string()),
                quantity: 1.0,
                unit_of_measure: "each".to_string(),
                unit_price_minor: 5_000,
                gl_account_code: "6100".to_string(),
            }],
        };

        let result = update_po_lines(&pool, TEST_TENANT, created.po.po_id, &update_req).await;
        assert!(
            matches!(result, Err(PoError::NotDraft(_))),
            "expected NotDraft, got {:?}",
            result
        );

        cleanup(&pool).await;
    }

    #[tokio::test]
    #[serial]
    async fn test_po_created_event_enqueued_in_outbox() {
        let pool = test_pool().await;
        cleanup(&pool).await;
        let vendor_id = create_test_vendor(&pool).await;

        let created = create_po(&pool, TEST_TENANT, &sample_req(vendor_id), "corr-outbox".to_string())
            .await
            .expect("create failed");

        let count: (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM events_outbox WHERE aggregate_type = 'po' AND aggregate_id = $1",
        )
        .bind(created.po.po_id.to_string())
        .fetch_one(&pool)
        .await
        .expect("outbox query failed");

        assert!(count.0 >= 1, "expected >=1 outbox event, got {}", count.0);

        cleanup(&pool).await;
    }
}
