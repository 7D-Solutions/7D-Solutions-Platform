//! Bill void lifecycle: Guard → Mutation → Outbox atomicity.
//!
//! void_bill: transitions a bill to 'voided', preventing further allocations.
//!
//! Void is an explicit, append-only operation:
//!   - void_reason is required and becomes the immutable audit justification.
//!   - The outbox event carries reverses_event_id referencing the original
//!     vendor_bill_created event, enabling downstream GL reversal.
//!
//! Permitted void transitions: open | matched | approved | partially_paid → voided
//! Paid bills cannot be voided (reversal requires a credit memo).
//!
//! Idempotency contract:
//!   - If bill is already 'voided', returns current state (no re-emit).
//!   - Concurrency: row locked with SELECT … FOR UPDATE before mutation.

use chrono::Utc;
use sqlx::PgPool;
use uuid::Uuid;

use tax_core::TaxProvider;

use crate::events::{
    build_vendor_bill_voided_envelope, VendorBillVoidedPayload,
    EVENT_TYPE_VENDOR_BILL_VOIDED, EVENT_TYPE_VENDOR_BILL_CREATED,
};
use crate::outbox::enqueue_event_tx;

use super::{BillError, VendorBill, VoidBillRequest};

// ============================================================================
// Internal DB row type
// ============================================================================

#[derive(sqlx::FromRow)]
struct BillHeaderRow {
    vendor_id: Uuid,
    vendor_invoice_ref: String,
    total_minor: i64,
    currency: String,
    status: String,
}

// ============================================================================
// Public API
// ============================================================================

/// Void a vendor bill, preventing further allocations.
///
/// Guard:    Lock bill row; verify status permits voiding.
/// Mutation: UPDATE status = 'voided'.
/// Outbox:   ap.vendor_bill_voided enqueued atomically with reversal linkage.
///
/// Idempotent: if already 'voided', returns the current bill state (no re-emit).
pub async fn void_bill(
    pool: &PgPool,
    tax_provider: &(impl TaxProvider + ?Sized),
    tenant_id: &str,
    bill_id: Uuid,
    req: &VoidBillRequest,
    correlation_id: String,
) -> Result<VendorBill, BillError> {
    req.validate()?;

    let mut tx = pool.begin().await?;

    // Guard: lock the bill row to prevent concurrent mutations
    let row: Option<BillHeaderRow> = sqlx::query_as(
        r#"
        SELECT vendor_id, vendor_invoice_ref, total_minor, currency, status
        FROM vendor_bills
        WHERE bill_id = $1 AND tenant_id = $2
        FOR UPDATE
        "#,
    )
    .bind(bill_id)
    .bind(tenant_id)
    .fetch_optional(&mut *tx)
    .await?;

    let row = row.ok_or(BillError::NotFound(bill_id))?;

    // Idempotency: already voided → commit and return current state
    if row.status == "voided" {
        tx.commit().await?;
        let bill: VendorBill = sqlx::query_as(
            r#"
            SELECT bill_id, tenant_id, vendor_id, vendor_invoice_ref, currency,
                   total_minor, tax_minor, invoice_date, due_date, status, fx_rate_id,
                   entered_by, entered_at
            FROM vendor_bills
            WHERE bill_id = $1 AND tenant_id = $2
            "#,
        )
        .bind(bill_id)
        .bind(tenant_id)
        .fetch_one(pool)
        .await?;
        return Ok(bill);
    }

    // Guard: only voidable statuses are permitted
    if !matches!(row.status.as_str(), "open" | "matched" | "approved" | "partially_paid") {
        return Err(BillError::InvalidTransition {
            from: row.status.clone(),
            to: "voided".to_string(),
        });
    }

    // Tax: void any active tax snapshot for this bill.
    // If no snapshot exists, the bill is non-taxable — proceed normally.
    crate::domain::tax::void_bill_tax(
        pool,
        tax_provider,
        tenant_id,
        bill_id,
        req.void_reason.trim(),
        &correlation_id,
    )
    .await
    .map_err(|e| BillError::TaxError(format!("tax void failed: {}", e)))?;

    let now = Utc::now();
    let event_id = Uuid::new_v4();

    // Look up the original vendor_bill_created event_id for reversal linkage.
    // Queried inside the transaction so it's consistent with the locked state.
    let original_event_id: Option<Uuid> = sqlx::query_scalar(
        r#"
        SELECT event_id
        FROM events_outbox
        WHERE aggregate_type = 'bill'
          AND aggregate_id    = $1
          AND event_type      = $2
        LIMIT 1
        "#,
    )
    .bind(bill_id.to_string())
    .bind(EVENT_TYPE_VENDOR_BILL_CREATED)
    .fetch_optional(&mut *tx)
    .await?;

    // Mutation: advance status to voided
    let voided: VendorBill = sqlx::query_as(
        r#"
        UPDATE vendor_bills
        SET status = 'voided'
        WHERE bill_id = $1 AND tenant_id = $2
        RETURNING
            bill_id, tenant_id, vendor_id, vendor_invoice_ref, currency,
            total_minor, tax_minor, invoice_date, due_date, status, fx_rate_id,
            entered_by, entered_at
        "#,
    )
    .bind(bill_id)
    .bind(tenant_id)
    .fetch_one(&mut *tx)
    .await?;

    // Outbox: ap.vendor_bill_voided (REVERSAL class, reverses original creation event)
    let payload = VendorBillVoidedPayload {
        bill_id,
        tenant_id: tenant_id.to_string(),
        vendor_id: row.vendor_id,
        vendor_invoice_ref: row.vendor_invoice_ref.clone(),
        original_total_minor: row.total_minor,
        currency: row.currency.clone(),
        void_reason: req.void_reason.trim().to_string(),
        voided_by: req.voided_by.trim().to_string(),
        voided_at: now,
    };

    let envelope = build_vendor_bill_voided_envelope(
        event_id,
        tenant_id.to_string(),
        correlation_id,
        None,
        original_event_id,
        payload,
    );

    enqueue_event_tx(
        &mut tx,
        event_id,
        EVENT_TYPE_VENDOR_BILL_VOIDED,
        "bill",
        &bill_id.to_string(),
        &envelope,
    )
    .await?;

    tx.commit().await?;

    Ok(voided)
}

// ============================================================================
// Integrated Tests (real DB, no mocks)
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::tax::ZeroTaxProvider;
    use serial_test::serial;

    const TEST_TENANT: &str = "test-tenant-void-bill";

    fn db_url() -> String {
        std::env::var("DATABASE_URL")
            .unwrap_or_else(|_| "postgres://ap_user:ap_pass@localhost:5443/ap_db".to_string())
    }

    async fn pool() -> PgPool {
        PgPool::connect(&db_url()).await.expect("DB connect failed")
    }

    async fn create_vendor(db: &PgPool) -> Uuid {
        let vendor_id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO vendors (vendor_id, tenant_id, name, currency, payment_terms_days, \
             is_active, created_at, updated_at) VALUES ($1, $2, $3, 'USD', 30, TRUE, NOW(), NOW())",
        )
        .bind(vendor_id)
        .bind(TEST_TENANT)
        .bind(format!("Vendor-{}", vendor_id))
        .execute(db)
        .await
        .expect("insert vendor");
        vendor_id
    }

    async fn create_bill(db: &PgPool, vendor_id: Uuid, status: &str) -> Uuid {
        let bill_id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO vendor_bills (bill_id, tenant_id, vendor_id, vendor_invoice_ref, \
             currency, total_minor, invoice_date, due_date, status, entered_by, entered_at) \
             VALUES ($1, $2, $3, $4, 'USD', 50000, NOW(), NOW() + interval '30 days', \
             $5, 'system', NOW())",
        )
        .bind(bill_id)
        .bind(TEST_TENANT)
        .bind(vendor_id)
        .bind(format!("INV-{}", &bill_id.to_string()[..8]))
        .bind(status)
        .execute(db)
        .await
        .expect("insert bill");
        bill_id
    }

    async fn cleanup(db: &PgPool) {
        for q in [
            "DELETE FROM three_way_match WHERE bill_id IN \
             (SELECT bill_id FROM vendor_bills WHERE tenant_id = $1)",
            "DELETE FROM events_outbox WHERE aggregate_type = 'bill' \
             AND aggregate_id IN (SELECT bill_id::TEXT FROM vendor_bills WHERE tenant_id = $1)",
            "DELETE FROM bill_lines WHERE bill_id IN \
             (SELECT bill_id FROM vendor_bills WHERE tenant_id = $1)",
            "DELETE FROM vendor_bills WHERE tenant_id = $1",
            "DELETE FROM vendors WHERE tenant_id = $1",
        ] {
            sqlx::query(q).bind(TEST_TENANT).execute(db).await.ok();
        }
    }

    fn void_req() -> VoidBillRequest {
        VoidBillRequest {
            voided_by: "accountant-1".to_string(),
            void_reason: "duplicate entry".to_string(),
        }
    }

    #[tokio::test]
    #[serial]
    async fn test_void_open_bill_transitions_status() {
        let db = pool().await;
        cleanup(&db).await;
        let vendor_id = create_vendor(&db).await;
        let bill_id = create_bill(&db, vendor_id, "open").await;

        let result = void_bill(&db, &ZeroTaxProvider, TEST_TENANT, bill_id, &void_req(), "corr-v1".to_string())
            .await
            .expect("void_bill failed");

        assert_eq!(result.status, "voided");

        let (count,): (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM events_outbox WHERE aggregate_type = 'bill' \
             AND aggregate_id = $1 AND event_type = $2",
        )
        .bind(bill_id.to_string())
        .bind(EVENT_TYPE_VENDOR_BILL_VOIDED)
        .fetch_one(&db)
        .await
        .expect("outbox count");
        assert_eq!(count, 1);

        cleanup(&db).await;
    }

    #[tokio::test]
    #[serial]
    async fn test_void_approved_bill() {
        let db = pool().await;
        cleanup(&db).await;
        let vendor_id = create_vendor(&db).await;
        let bill_id = create_bill(&db, vendor_id, "approved").await;

        let result = void_bill(&db, &ZeroTaxProvider, TEST_TENANT, bill_id, &void_req(), "corr-v2".to_string())
            .await
            .expect("void approved bill failed");

        assert_eq!(result.status, "voided");

        cleanup(&db).await;
    }

    #[tokio::test]
    #[serial]
    async fn test_void_paid_bill_returns_invalid_transition() {
        let db = pool().await;
        cleanup(&db).await;
        let vendor_id = create_vendor(&db).await;
        let bill_id = create_bill(&db, vendor_id, "paid").await;

        let result = void_bill(&db, &ZeroTaxProvider, TEST_TENANT, bill_id, &void_req(), "corr-v3".to_string()).await;

        assert!(
            matches!(result, Err(BillError::InvalidTransition { .. })),
            "expected InvalidTransition from paid, got {:?}",
            result
        );

        cleanup(&db).await;
    }

    #[tokio::test]
    #[serial]
    async fn test_void_idempotent_no_double_event() {
        let db = pool().await;
        cleanup(&db).await;
        let vendor_id = create_vendor(&db).await;
        let bill_id = create_bill(&db, vendor_id, "open").await;

        void_bill(&db, &ZeroTaxProvider, TEST_TENANT, bill_id, &void_req(), "corr-v4a".to_string())
            .await
            .expect("first void");

        let second = void_bill(&db, &ZeroTaxProvider, TEST_TENANT, bill_id, &void_req(), "corr-v4b".to_string())
            .await
            .expect("second void must succeed (idempotent)");

        assert_eq!(second.status, "voided");

        let (count,): (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM events_outbox WHERE aggregate_type = 'bill' \
             AND aggregate_id = $1 AND event_type = $2",
        )
        .bind(bill_id.to_string())
        .bind(EVENT_TYPE_VENDOR_BILL_VOIDED)
        .fetch_one(&db)
        .await
        .expect("outbox count");
        assert_eq!(count, 1, "idempotent second void must not produce a second event");

        cleanup(&db).await;
    }

    #[tokio::test]
    #[serial]
    async fn test_void_wrong_tenant_returns_not_found() {
        let db = pool().await;
        cleanup(&db).await;
        let vendor_id = create_vendor(&db).await;
        let bill_id = create_bill(&db, vendor_id, "open").await;

        let result = void_bill(&db, &ZeroTaxProvider, "wrong-tenant", bill_id, &void_req(), "corr-v5".to_string()).await;

        assert!(
            matches!(result, Err(BillError::NotFound(_))),
            "expected NotFound for wrong tenant, got {:?}",
            result
        );

        cleanup(&db).await;
    }

    #[tokio::test]
    #[serial]
    async fn test_void_event_carries_void_reason_and_total() {
        let db = pool().await;
        cleanup(&db).await;
        let vendor_id = create_vendor(&db).await;
        let bill_id = create_bill(&db, vendor_id, "open").await;

        void_bill(&db, &ZeroTaxProvider, TEST_TENANT, bill_id, &void_req(), "corr-v6".to_string())
            .await
            .expect("void failed");

        let payload: serde_json::Value = sqlx::query_scalar(
            "SELECT payload FROM events_outbox WHERE aggregate_type = 'bill' \
             AND aggregate_id = $1 AND event_type = $2",
        )
        .bind(bill_id.to_string())
        .bind(EVENT_TYPE_VENDOR_BILL_VOIDED)
        .fetch_one(&db)
        .await
        .expect("payload query");

        // Payload is the EventEnvelope JSON
        let void_reason = payload["payload"]["void_reason"].as_str().unwrap_or("");
        assert_eq!(void_reason, "duplicate entry");

        let total = payload["payload"]["original_total_minor"].as_i64().unwrap_or(0);
        assert_eq!(total, 50000);

        cleanup(&db).await;
    }
}
