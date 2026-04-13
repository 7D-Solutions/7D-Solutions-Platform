//! Bill service — Guard → Mutation → Outbox orchestration.
//!
//! All writes follow the pattern:
//!   1. Guard: validate business preconditions (vendor exists, no duplicate invoice)
//!   2. Mutation: insert bill + lines in a transaction
//!   3. Outbox: enqueue vendor_bill_created event atomically
//!
//! Raw SQL lives in `super::repo`; this module owns business logic only.

use chrono::Utc;
use sqlx::PgPool;
use uuid::Uuid;

use platform_audit::schema::{MutationClass, WriteAuditRequest};
use platform_audit::writer::AuditWriter;

use crate::domain::vendors::compute_due_date;
use crate::events::{
    build_vendor_bill_created_envelope, BillLine as EventBillLine, VendorBillCreatedPayload,
    EVENT_TYPE_VENDOR_BILL_CREATED,
};
use crate::outbox::enqueue_event_tx;

use super::repo;
use super::{BillError, CreateBillRequest, VendorBillWithLines};

// ============================================================================
// Reads
// ============================================================================

/// Fetch a single bill with its lines. Returns None if not found for this tenant.
pub async fn get_bill(
    pool: &PgPool,
    tenant_id: &str,
    bill_id: Uuid,
) -> Result<Option<VendorBillWithLines>, BillError> {
    let Some(bill) = repo::fetch_bill(pool, tenant_id, bill_id).await? else {
        return Ok(None);
    };

    let lines = repo::fetch_bill_lines(pool, bill_id).await?;

    Ok(Some(VendorBillWithLines { bill, lines }))
}

/// List bills for a tenant. Excludes voided by default.
/// Pass `vendor_id` to filter to a single vendor.
pub async fn list_bills(
    pool: &PgPool,
    tenant_id: &str,
    vendor_id: Option<Uuid>,
    include_voided: bool,
) -> Result<Vec<super::VendorBill>, BillError> {
    repo::list_bills(pool, tenant_id, vendor_id, include_voided).await
}

// ============================================================================
// Writes
// ============================================================================

/// Create a vendor bill with lines.
///
/// - Due date is derived from vendor payment terms when not explicitly provided.
/// - Bill total is computed as the sum of line totals (quantity × unit_price).
/// - Emits `ap.vendor_bill_created` via the outbox atomically in the same transaction.
pub async fn create_bill(
    pool: &PgPool,
    tenant_id: &str,
    req: &CreateBillRequest,
    correlation_id: String,
) -> Result<VendorBillWithLines, BillError> {
    req.validate()?;

    // Guard: vendor must exist and be active for this tenant
    let vendor_row = repo::fetch_active_vendor(pool, req.vendor_id, tenant_id).await?;
    let (_, payment_terms_days) = vendor_row.ok_or(BillError::VendorNotFound(req.vendor_id))?;

    // Derive due_date deterministically from vendor terms when not provided
    let due_date = match req.due_date {
        Some(d) => d,
        None => {
            let invoice_naive = req.invoice_date.date_naive();
            let due_naive = compute_due_date(invoice_naive, payment_terms_days);
            due_naive
                .and_hms_opt(0, 0, 0)
                .expect("valid time components")
                .and_utc()
        }
    };

    let bill_id = Uuid::new_v4();
    let event_id = Uuid::new_v4();
    let now = Utc::now();

    // Compute total from lines (sum of qty × unit_price)
    let total_minor: i64 = req.lines.iter().map(|l| l.line_total_minor()).sum();

    let mut tx = pool.begin().await?;

    // Mutation: insert bill header
    let bill = repo::insert_bill(
        &mut *tx,
        bill_id,
        tenant_id,
        req.vendor_id,
        req.vendor_invoice_ref.trim(),
        &req.currency.to_uppercase(),
        total_minor,
        req.tax_minor,
        req.invoice_date,
        due_date,
        req.fx_rate_id,
        req.entered_by.trim(),
        now,
    )
    .await?;

    // Mutation: insert bill lines
    let mut bill_lines = Vec::with_capacity(req.lines.len());
    let mut event_lines = Vec::with_capacity(req.lines.len());

    for line_req in &req.lines {
        let line_id = Uuid::new_v4();
        let description = line_req.description.as_deref().unwrap_or("").to_string();
        let gl_account = line_req
            .gl_account_code
            .as_deref()
            .unwrap_or("")
            .to_string();
        let line_total = line_req.line_total_minor();

        let line = repo::insert_bill_line(
            &mut *tx,
            line_id,
            bill_id,
            &description,
            line_req.quantity,
            line_req.unit_price_minor,
            line_total,
            &gl_account,
            line_req.po_line_id,
            now,
        )
        .await?;

        event_lines.push(EventBillLine {
            line_id,
            description: description.clone(),
            quantity: line_req.quantity,
            unit_price_minor: line_req.unit_price_minor,
            line_total_minor: line_total,
            gl_account_code: gl_account,
            po_line_id: line_req.po_line_id,
        });
        bill_lines.push(line);
    }

    // Audit: record bill creation inside the same transaction
    let audit_req = WriteAuditRequest::new(
        Uuid::nil(),
        "system".to_string(),
        "CreateVendorBill".to_string(),
        MutationClass::Create,
        "VendorBill".to_string(),
        bill_id.to_string(),
    );
    AuditWriter::write_in_tx(&mut tx, audit_req).await
        .map_err(|e| match e {
            platform_audit::writer::AuditWriterError::Database(db) => BillError::Database(db),
            platform_audit::writer::AuditWriterError::InvalidRequest(msg) => {
                BillError::Database(sqlx::Error::Protocol(msg))
            }
        })?;

    // Outbox: enqueue vendor_bill_created event
    let payload = VendorBillCreatedPayload {
        bill_id,
        tenant_id: tenant_id.to_string(),
        vendor_id: req.vendor_id,
        vendor_invoice_ref: req.vendor_invoice_ref.trim().to_string(),
        currency: req.currency.to_uppercase(),
        lines: event_lines,
        total_minor,
        tax_minor: req.tax_minor,
        invoice_date: req.invoice_date,
        due_date,
        entered_by: req.entered_by.trim().to_string(),
        entered_at: now,
    };

    let envelope = build_vendor_bill_created_envelope(
        event_id,
        tenant_id.to_string(),
        correlation_id,
        None,
        payload,
    );

    enqueue_event_tx(
        &mut tx,
        event_id,
        EVENT_TYPE_VENDOR_BILL_CREATED,
        "bill",
        &bill_id.to_string(),
        &envelope,
    )
    .await?;

    tx.commit().await?;

    Ok(VendorBillWithLines {
        bill,
        lines: bill_lines,
    })
}

// ============================================================================
// Integrated Tests (real DB, no mocks)
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use serial_test::serial;

    const TEST_TENANT: &str = "test-tenant-bills";

    fn test_db_url() -> String {
        std::env::var("DATABASE_URL")
            .unwrap_or_else(|_| "postgres://ap_user:ap_pass@localhost:5443/ap_db".to_string())
    }

    async fn test_pool() -> PgPool {
        PgPool::connect(&test_db_url())
            .await
            .expect("Failed to connect to AP test database")
    }

    /// Insert a minimal vendor for testing; returns its vendor_id.
    async fn create_test_vendor(pool: &PgPool, terms_days: i32) -> Uuid {
        let vendor_id = Uuid::new_v4();
        sqlx::query(
            r#"
            INSERT INTO vendors (
                vendor_id, tenant_id, name, currency, payment_terms_days,
                is_active, created_at, updated_at
            )
            VALUES ($1, $2, $3, 'USD', $4, TRUE, NOW(), NOW())
            "#,
        )
        .bind(vendor_id)
        .bind(TEST_TENANT)
        .bind(format!("Test Vendor {}", vendor_id))
        .bind(terms_days)
        .execute(pool)
        .await
        .expect("insert test vendor failed");
        vendor_id
    }

    /// Remove all TEST_TENANT data in dependency order.
    async fn cleanup(pool: &PgPool) {
        sqlx::query(
            "DELETE FROM events_outbox WHERE aggregate_type = 'bill' \
             AND aggregate_id IN (SELECT bill_id::TEXT FROM vendor_bills WHERE tenant_id = $1)",
        )
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

        sqlx::query(
            "DELETE FROM bill_lines WHERE bill_id IN \
             (SELECT bill_id FROM vendor_bills WHERE tenant_id = $1)",
        )
        .bind(TEST_TENANT)
        .execute(pool)
        .await
        .ok();

        sqlx::query("DELETE FROM vendor_bills WHERE tenant_id = $1")
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

    fn sample_req(vendor_id: Uuid, inv_ref: &str) -> CreateBillRequest {
        CreateBillRequest {
            vendor_id,
            vendor_invoice_ref: inv_ref.to_string(),
            currency: "USD".to_string(),
            invoice_date: Utc::now(),
            due_date: None,
            tax_minor: Some(500),
            entered_by: "user-ap".to_string(),
            fx_rate_id: None,
            lines: vec![
                super::super::CreateBillLineRequest {
                    description: Some("Consulting services".to_string()),
                    item_id: None,
                    quantity: 10.0,
                    unit_price_minor: 5000,
                    gl_account_code: Some("6100".to_string()),
                    po_line_id: None,
                },
                super::super::CreateBillLineRequest {
                    description: Some("Expenses".to_string()),
                    item_id: None,
                    quantity: 1.0,
                    unit_price_minor: 1000,
                    gl_account_code: None,
                    po_line_id: None,
                },
            ],
        }
    }

    #[tokio::test]
    #[serial]
    async fn test_create_bill_derives_due_date_from_terms() {
        let pool = test_pool().await;
        cleanup(&pool).await;
        let vendor_id = create_test_vendor(&pool, 30).await; // Net-30

        let req = sample_req(vendor_id, "INV-2026-001");
        let result = create_bill(&pool, TEST_TENANT, &req, "corr-1".to_string())
            .await
            .expect("create_bill failed");

        assert_eq!(result.bill.vendor_id, vendor_id);
        assert_eq!(result.bill.status, "open");
        // total = 10*5000 + 1*1000 = 51000
        assert_eq!(result.bill.total_minor, 51_000);
        assert_eq!(result.bill.currency, "USD");
        assert_eq!(result.lines.len(), 2);

        // Due date = invoice_date + 30 days
        let invoice_naive = req.invoice_date.date_naive();
        let expected_due = compute_due_date(invoice_naive, 30);
        assert_eq!(result.bill.due_date.date_naive(), expected_due);

        cleanup(&pool).await;
    }

    #[tokio::test]
    #[serial]
    async fn test_create_bill_explicit_due_date() {
        let pool = test_pool().await;
        cleanup(&pool).await;
        let vendor_id = create_test_vendor(&pool, 30).await;

        let explicit_due = Utc::now() + chrono::Duration::days(60);
        let mut req = sample_req(vendor_id, "INV-2026-002");
        req.due_date = Some(explicit_due);

        let result = create_bill(&pool, TEST_TENANT, &req, "corr-2".to_string())
            .await
            .expect("create_bill with explicit due_date failed");

        // Due date must match what was supplied (same day)
        assert_eq!(result.bill.due_date.date_naive(), explicit_due.date_naive());

        cleanup(&pool).await;
    }

    #[tokio::test]
    #[serial]
    async fn test_get_bill_returns_with_lines() {
        let pool = test_pool().await;
        cleanup(&pool).await;
        let vendor_id = create_test_vendor(&pool, 15).await;

        let req = sample_req(vendor_id, "INV-2026-003");
        let created = create_bill(&pool, TEST_TENANT, &req, "corr-3".to_string())
            .await
            .expect("create failed");

        let fetched = get_bill(&pool, TEST_TENANT, created.bill.bill_id)
            .await
            .expect("get_bill failed");

        assert!(fetched.is_some());
        let bwl = fetched.expect("bill must be present");
        assert_eq!(bwl.bill.bill_id, created.bill.bill_id);
        assert_eq!(bwl.lines.len(), 2);

        cleanup(&pool).await;
    }

    #[tokio::test]
    #[serial]
    async fn test_get_bill_wrong_tenant_returns_none() {
        let pool = test_pool().await;
        cleanup(&pool).await;
        let vendor_id = create_test_vendor(&pool, 30).await;

        let req = sample_req(vendor_id, "INV-2026-004");
        let created = create_bill(&pool, TEST_TENANT, &req, "corr-4".to_string())
            .await
            .expect("create failed");

        let result = get_bill(&pool, "other-tenant", created.bill.bill_id)
            .await
            .expect("get_bill error");
        assert!(result.is_none());

        cleanup(&pool).await;
    }

    #[tokio::test]
    #[serial]
    async fn test_duplicate_invoice_rejected() {
        let pool = test_pool().await;
        cleanup(&pool).await;
        let vendor_id = create_test_vendor(&pool, 30).await;

        let req = sample_req(vendor_id, "INV-DUP-001");
        create_bill(&pool, TEST_TENANT, &req, "corr-a".to_string())
            .await
            .expect("first create failed");

        let result = create_bill(&pool, TEST_TENANT, &req, "corr-b".to_string()).await;
        assert!(
            matches!(result, Err(BillError::DuplicateInvoice(_))),
            "expected DuplicateInvoice, got {:?}",
            result
        );

        cleanup(&pool).await;
    }

    #[tokio::test]
    #[serial]
    async fn test_vendor_not_found_returns_error() {
        let pool = test_pool().await;
        cleanup(&pool).await;

        let req = sample_req(Uuid::new_v4(), "INV-NOVENDOR-001");
        let result = create_bill(&pool, TEST_TENANT, &req, "corr-c".to_string()).await;
        assert!(
            matches!(result, Err(BillError::VendorNotFound(_))),
            "expected VendorNotFound, got {:?}",
            result
        );
    }

    #[tokio::test]
    #[serial]
    async fn test_vendor_bill_created_event_enqueued() {
        let pool = test_pool().await;
        cleanup(&pool).await;
        let vendor_id = create_test_vendor(&pool, 30).await;

        let req = sample_req(vendor_id, "INV-OUTBOX-001");
        let result = create_bill(&pool, TEST_TENANT, &req, "corr-outbox".to_string())
            .await
            .expect("create failed");

        let count: (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM events_outbox \
             WHERE aggregate_type = 'bill' AND aggregate_id = $1",
        )
        .bind(result.bill.bill_id.to_string())
        .fetch_one(&pool)
        .await
        .expect("outbox query failed");

        assert!(count.0 >= 1, "expected >=1 outbox event, got {}", count.0);

        cleanup(&pool).await;
    }

    #[tokio::test]
    #[serial]
    async fn test_list_bills_excludes_voided_by_default() {
        let pool = test_pool().await;
        cleanup(&pool).await;
        let vendor_id = create_test_vendor(&pool, 30).await;

        let req = sample_req(vendor_id, "INV-VOID-001");
        let created = create_bill(&pool, TEST_TENANT, &req, "corr-v".to_string())
            .await
            .expect("create failed");

        // Manually void the bill
        sqlx::query("UPDATE vendor_bills SET status = 'voided' WHERE bill_id = $1 AND tenant_id = $2")
            .bind(created.bill.bill_id)
            .bind(TEST_TENANT)
            .execute(&pool)
            .await
            .expect("void update failed");

        let active = list_bills(&pool, TEST_TENANT, None, false)
            .await
            .expect("list failed");
        assert!(
            active.iter().all(|b| b.bill_id != created.bill.bill_id),
            "voided bill should be excluded from default list"
        );

        let all = list_bills(&pool, TEST_TENANT, None, true)
            .await
            .expect("list all failed");
        assert!(
            all.iter().any(|b| b.bill_id == created.bill.bill_id),
            "voided bill must appear with include_voided=true"
        );

        cleanup(&pool).await;
    }

    #[tokio::test]
    #[serial]
    async fn test_list_bills_filter_by_vendor() {
        let pool = test_pool().await;
        cleanup(&pool).await;

        let vendor_a = create_test_vendor(&pool, 30).await;
        let vendor_b = create_test_vendor(&pool, 30).await;

        create_bill(
            &pool,
            TEST_TENANT,
            &sample_req(vendor_a, "INV-A-001"),
            "ca".to_string(),
        )
        .await
        .expect("vendor A bill failed");
        create_bill(
            &pool,
            TEST_TENANT,
            &sample_req(vendor_b, "INV-B-001"),
            "cb".to_string(),
        )
        .await
        .expect("vendor B bill failed");

        let a_bills = list_bills(&pool, TEST_TENANT, Some(vendor_a), false)
            .await
            .expect("list vendor_a failed");
        assert_eq!(a_bills.len(), 1);
        assert_eq!(a_bills[0].vendor_id, vendor_a);

        cleanup(&pool).await;
    }

    #[tokio::test]
    #[serial]
    async fn test_multi_currency_bill_stores_fx_rate_id() {
        let pool = test_pool().await;
        cleanup(&pool).await;
        let vendor_id = create_test_vendor(&pool, 30).await;

        let fx_rate_id = Uuid::new_v4(); // pretend GL rate UUID
        let mut req = sample_req(vendor_id, "INV-EUR-001");
        req.currency = "EUR".to_string();
        req.fx_rate_id = Some(fx_rate_id);

        let result = create_bill(&pool, TEST_TENANT, &req, "corr-fx".to_string())
            .await
            .expect("EUR bill create failed");

        assert_eq!(result.bill.currency, "EUR");
        assert_eq!(result.bill.fx_rate_id, Some(fx_rate_id));

        cleanup(&pool).await;
    }
}
