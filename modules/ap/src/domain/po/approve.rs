//! PO approval: Guard → Mutation → Outbox atomicity.
//!
//! approve_po: transitions a draft PO to approved; emits ap.po_approved.
//!
//! Idempotency contract:
//!   - If PO is already 'approved', returns the current state without
//!     re-emitting or erroring (safe to call twice with same input).
//!   - If PO is cancelled or closed, returns InvalidTransition.
//!   - Concurrency: row is locked with SELECT ... FOR UPDATE before any mutation.
//!
//! Event actor attribution: approved_by is embedded in the payload.

use chrono::Utc;
use sqlx::PgPool;
use uuid::Uuid;

use crate::events::{
    build_po_approved_envelope, PoApprovedPayload, EVENT_TYPE_PO_APPROVED,
};
use crate::outbox::enqueue_event_tx;

use super::{ApprovePoRequest, PoError, PurchaseOrder};

// ============================================================================
// Public API
// ============================================================================

/// Approve a purchase order, transitioning it from draft to approved.
///
/// Guard:    PO must exist for the tenant; lock row; check status.
/// Mutation: UPDATE status = 'approved'; INSERT po_status audit row.
/// Outbox:   ap.po_approved enqueued atomically with actor attribution.
///
/// Idempotent: if PO is already 'approved', returns it unchanged (no re-emit).
pub async fn approve_po(
    pool: &PgPool,
    tenant_id: &str,
    po_id: Uuid,
    req: &ApprovePoRequest,
    correlation_id: String,
) -> Result<PurchaseOrder, PoError> {
    req.validate()?;

    let mut tx = pool.begin().await?;

    // Guard: lock the PO row to prevent concurrent approvals
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

    // Idempotency: already approved → return current state without re-emitting
    if po.status == "approved" {
        tx.commit().await?;
        return Ok(po);
    }

    // Guard: only draft → approved is valid
    if po.status != "draft" {
        return Err(PoError::InvalidTransition {
            from: po.status.clone(),
            to: "approved".to_string(),
        });
    }

    let now = Utc::now();
    let event_id = Uuid::new_v4();

    // Mutation: advance status to approved
    let approved_po: PurchaseOrder = sqlx::query_as(
        r#"
        UPDATE purchase_orders
        SET status = 'approved'
        WHERE po_id = $1 AND tenant_id = $2
        RETURNING
            po_id, tenant_id, vendor_id, po_number, currency,
            total_minor, status, created_by, created_at, expected_delivery_date
        "#,
    )
    .bind(po_id)
    .bind(tenant_id)
    .fetch_one(&mut *tx)
    .await?;

    // Mutation: append approved entry to status audit log
    sqlx::query(
        "INSERT INTO po_status (po_id, status, changed_by, changed_at) VALUES ($1, 'approved', $2, $3)",
    )
    .bind(po_id)
    .bind(req.approved_by.trim())
    .bind(now)
    .execute(&mut *tx)
    .await?;

    // Outbox: ap.po_approved — self-contained payload with actor attribution
    let payload = PoApprovedPayload {
        po_id,
        tenant_id: tenant_id.to_string(),
        vendor_id: approved_po.vendor_id,
        po_number: approved_po.po_number.clone(),
        approved_amount_minor: approved_po.total_minor,
        currency: approved_po.currency.clone(),
        approved_by: req.approved_by.trim().to_string(),
        approved_at: now,
    };

    let envelope = build_po_approved_envelope(
        event_id,
        tenant_id.to_string(),
        correlation_id,
        None,
        payload,
    );

    enqueue_event_tx(
        &mut tx,
        event_id,
        EVENT_TYPE_PO_APPROVED,
        "po",
        &po_id.to_string(),
        &envelope,
    )
    .await?;

    tx.commit().await?;

    Ok(approved_po)
}

// ============================================================================
// Integrated Tests (real DB, no mocks)
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::po::{
        service::{create_po, get_po},
        ApprovePoRequest, CreatePoLineRequest, CreatePoRequest,
    };
    use serial_test::serial;

    const TEST_TENANT: &str = "test-tenant-approve";

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
        .bind(format!("Approve Vendor {}", vendor_id))
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
        .bind(TEST_TENANT)
        .execute(pool)
        .await
        .ok();

        sqlx::query(
            "DELETE FROM po_status WHERE po_id IN \
             (SELECT po_id FROM purchase_orders WHERE tenant_id = $1)",
        )
        .bind(TEST_TENANT)
        .execute(pool)
        .await
        .ok();

        sqlx::query(
            "DELETE FROM po_lines WHERE po_id IN \
             (SELECT po_id FROM purchase_orders WHERE tenant_id = $1)",
        )
        .bind(TEST_TENANT)
        .execute(pool)
        .await
        .ok();

        sqlx::query("DELETE FROM purchase_orders WHERE tenant_id = $1")
            .bind(TEST_TENANT)
            .execute(pool)
            .await
            .ok();

        sqlx::query(
            "DELETE FROM events_outbox WHERE aggregate_type = 'vendor' \
             AND aggregate_id IN (SELECT vendor_id::TEXT FROM vendors WHERE tenant_id = $1)",
        )
        .bind(TEST_TENANT)
        .execute(pool)
        .await
        .ok();

        sqlx::query("DELETE FROM vendors WHERE tenant_id = $1")
            .bind(TEST_TENANT)
            .execute(pool)
            .await
            .ok();
    }

    fn sample_create_req(vendor_id: Uuid) -> CreatePoRequest {
        CreatePoRequest {
            vendor_id,
            currency: "USD".to_string(),
            created_by: "user-ap".to_string(),
            expected_delivery_date: None,
            lines: vec![CreatePoLineRequest {
                item_id: None,
                description: Some("Widgets".to_string()),
                quantity: 5.0,
                unit_of_measure: "each".to_string(),
                unit_price_minor: 10_000,
                gl_account_code: "6100".to_string(),
            }],
        }
    }

    fn approve_req() -> ApprovePoRequest {
        ApprovePoRequest {
            approved_by: "manager-1".to_string(),
        }
    }

    // -- Tests --

    #[tokio::test]
    #[serial]
    async fn test_approve_draft_po_transitions_status() {
        let pool = test_pool().await;
        cleanup(&pool).await;
        let vendor_id = create_test_vendor(&pool).await;

        let created = create_po(&pool, TEST_TENANT, &sample_create_req(vendor_id), "corr-a1".to_string())
            .await
            .expect("create_po failed");

        assert_eq!(created.po.status, "draft");

        let approved = approve_po(&pool, TEST_TENANT, created.po.po_id, &approve_req(), "corr-a2".to_string())
            .await
            .expect("approve_po failed");

        assert_eq!(approved.status, "approved");
        assert_eq!(approved.po_id, created.po.po_id);

        // Verify via read-back
        let fetched = get_po(&pool, TEST_TENANT, created.po.po_id)
            .await
            .expect("get_po failed")
            .expect("PO not found");
        assert_eq!(fetched.po.status, "approved");

        cleanup(&pool).await;
    }

    #[tokio::test]
    #[serial]
    async fn test_approve_emits_po_approved_event() {
        let pool = test_pool().await;
        cleanup(&pool).await;
        let vendor_id = create_test_vendor(&pool).await;

        let created = create_po(&pool, TEST_TENANT, &sample_create_req(vendor_id), "corr-b1".to_string())
            .await
            .expect("create_po failed");

        approve_po(&pool, TEST_TENANT, created.po.po_id, &approve_req(), "corr-b2".to_string())
            .await
            .expect("approve_po failed");

        let count: (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM events_outbox \
             WHERE aggregate_type = 'po' AND aggregate_id = $1 AND event_type = $2",
        )
        .bind(created.po.po_id.to_string())
        .bind(EVENT_TYPE_PO_APPROVED)
        .fetch_one(&pool)
        .await
        .expect("outbox query failed");

        assert_eq!(count.0, 1, "expected exactly 1 po_approved event in outbox");

        cleanup(&pool).await;
    }

    #[tokio::test]
    #[serial]
    async fn test_approve_idempotent_no_double_event() {
        let pool = test_pool().await;
        cleanup(&pool).await;
        let vendor_id = create_test_vendor(&pool).await;

        let created = create_po(&pool, TEST_TENANT, &sample_create_req(vendor_id), "corr-c1".to_string())
            .await
            .expect("create_po failed");

        // Approve once
        approve_po(&pool, TEST_TENANT, created.po.po_id, &approve_req(), "corr-c2".to_string())
            .await
            .expect("first approve failed");

        // Approve again — must succeed without re-emitting
        let second = approve_po(&pool, TEST_TENANT, created.po.po_id, &approve_req(), "corr-c3".to_string())
            .await
            .expect("second approve failed (should be idempotent)");

        assert_eq!(second.status, "approved");

        // Only one po_approved event in the outbox
        let count: (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM events_outbox \
             WHERE aggregate_type = 'po' AND aggregate_id = $1 AND event_type = $2",
        )
        .bind(created.po.po_id.to_string())
        .bind(EVENT_TYPE_PO_APPROVED)
        .fetch_one(&pool)
        .await
        .expect("outbox query failed");

        assert_eq!(count.0, 1, "idempotent approve must not produce a second event");

        cleanup(&pool).await;
    }

    #[tokio::test]
    #[serial]
    async fn test_approve_wrong_tenant_returns_not_found() {
        let pool = test_pool().await;
        cleanup(&pool).await;
        let vendor_id = create_test_vendor(&pool).await;

        let created = create_po(&pool, TEST_TENANT, &sample_create_req(vendor_id), "corr-d1".to_string())
            .await
            .expect("create_po failed");

        let result = approve_po(&pool, "wrong-tenant", created.po.po_id, &approve_req(), "corr-d2".to_string()).await;
        assert!(
            matches!(result, Err(PoError::NotFound(_))),
            "expected NotFound for wrong tenant, got {:?}",
            result
        );

        cleanup(&pool).await;
    }

    #[tokio::test]
    #[serial]
    async fn test_approve_cancelled_po_returns_invalid_transition() {
        let pool = test_pool().await;
        cleanup(&pool).await;
        let vendor_id = create_test_vendor(&pool).await;

        let created = create_po(&pool, TEST_TENANT, &sample_create_req(vendor_id), "corr-e1".to_string())
            .await
            .expect("create_po failed");

        // Force-cancel the PO via raw SQL
        sqlx::query("UPDATE purchase_orders SET status = 'cancelled' WHERE po_id = $1")
            .bind(created.po.po_id)
            .execute(&pool)
            .await
            .expect("status update failed");

        let result = approve_po(&pool, TEST_TENANT, created.po.po_id, &approve_req(), "corr-e2".to_string()).await;
        assert!(
            matches!(result, Err(PoError::InvalidTransition { .. })),
            "expected InvalidTransition for cancelled PO, got {:?}",
            result
        );

        cleanup(&pool).await;
    }

    #[tokio::test]
    #[serial]
    async fn test_approve_records_status_audit_entry() {
        let pool = test_pool().await;
        cleanup(&pool).await;
        let vendor_id = create_test_vendor(&pool).await;

        let created = create_po(&pool, TEST_TENANT, &sample_create_req(vendor_id), "corr-f1".to_string())
            .await
            .expect("create_po failed");

        approve_po(&pool, TEST_TENANT, created.po.po_id, &approve_req(), "corr-f2".to_string())
            .await
            .expect("approve_po failed");

        let count: (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM po_status WHERE po_id = $1 AND status = 'approved'",
        )
        .bind(created.po.po_id)
        .fetch_one(&pool)
        .await
        .expect("po_status query failed");

        assert_eq!(count.0, 1, "expected one 'approved' audit row in po_status");

        cleanup(&pool).await;
    }

    #[tokio::test]
    #[serial]
    async fn test_approve_locks_editable_fields() {
        // After approval, update_po_lines must reject with NotDraft
        let pool = test_pool().await;
        cleanup(&pool).await;
        let vendor_id = create_test_vendor(&pool).await;

        let created = create_po(&pool, TEST_TENANT, &sample_create_req(vendor_id), "corr-g1".to_string())
            .await
            .expect("create_po failed");

        approve_po(&pool, TEST_TENANT, created.po.po_id, &approve_req(), "corr-g2".to_string())
            .await
            .expect("approve_po failed");

        use crate::domain::po::{service::update_po_lines, UpdatePoLinesRequest};
        let update_req = UpdatePoLinesRequest {
            updated_by: "user-ap".to_string(),
            lines: vec![CreatePoLineRequest {
                item_id: None,
                description: Some("Sneaky edit".to_string()),
                quantity: 1.0,
                unit_of_measure: "each".to_string(),
                unit_price_minor: 1_000,
                gl_account_code: "6100".to_string(),
            }],
        };

        let result = update_po_lines(&pool, TEST_TENANT, created.po.po_id, &update_req).await;
        assert!(
            matches!(result, Err(PoError::NotDraft(_))),
            "expected NotDraft after approval, got {:?}",
            result
        );

        cleanup(&pool).await;
    }
}
