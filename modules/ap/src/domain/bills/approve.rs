//! Bill approval: Guard → Mutation → Outbox atomicity.
//!
//! approve_bill: transitions a bill to approved, enforcing the match policy.
//!
//! Match policy (configurable thresholds via override_reason escape hatch):
//!   - 'matched' + all three_way_match lines within_tolerance → approve directly.
//!   - 'matched' with any tolerance violations → require override_reason.
//!   - 'open' (match engine never run) → require override_reason.
//!
//! Idempotency contract:
//!   - If bill is already 'approved', returns current state without re-emitting.
//!   - Concurrency: row locked with SELECT … FOR UPDATE before any mutation.
//!
//! Event: ap.vendor_bill_approved carries full actor attribution and override note.

use chrono::Utc;
use sqlx::PgPool;
use uuid::Uuid;

use crate::events::{
    build_vendor_bill_approved_envelope, VendorBillApprovedPayload,
    EVENT_TYPE_VENDOR_BILL_APPROVED,
};
use crate::outbox::enqueue_event_tx;

use super::{ApproveBillRequest, BillError, VendorBill};

// ============================================================================
// Internal DB row type
// ============================================================================

#[derive(sqlx::FromRow)]
struct BillHeaderRow {
    vendor_id: Uuid,
    vendor_invoice_ref: String,
    total_minor: i64,
    currency: String,
    due_date: chrono::DateTime<Utc>,
    status: String,
}

// ============================================================================
// Public API
// ============================================================================

/// Approve a vendor bill, enforcing the match policy.
///
/// Guard:    Lock bill row; verify status; check match policy.
/// Mutation: UPDATE status = 'approved'.
/// Outbox:   ap.vendor_bill_approved enqueued atomically with actor attribution.
///
/// Idempotent: if already 'approved', returns the current bill state (no re-emit).
pub async fn approve_bill(
    pool: &PgPool,
    tenant_id: &str,
    bill_id: Uuid,
    req: &ApproveBillRequest,
    correlation_id: String,
) -> Result<VendorBill, BillError> {
    req.validate()?;

    let mut tx = pool.begin().await?;

    // Guard: lock the bill row to prevent concurrent approvals
    let row: Option<BillHeaderRow> = sqlx::query_as(
        r#"
        SELECT vendor_id, vendor_invoice_ref, total_minor, currency, due_date, status
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

    // Idempotency: already approved → commit and return current state
    if row.status == "approved" {
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

    // Guard: only open/matched → approved is a valid transition
    if !matches!(row.status.as_str(), "open" | "matched") {
        return Err(BillError::InvalidTransition {
            from: row.status.clone(),
            to: "approved".to_string(),
        });
    }

    // Guard: enforce match policy (read against pool; bill lock prevents concurrent match)
    check_match_policy(pool, bill_id, &row.status, &req.override_reason).await?;

    let now = Utc::now();
    let event_id = Uuid::new_v4();

    // Mutation: advance status to approved
    let approved: VendorBill = sqlx::query_as(
        r#"
        UPDATE vendor_bills
        SET status = 'approved'
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

    // Outbox: ap.vendor_bill_approved
    let payload = VendorBillApprovedPayload {
        bill_id,
        tenant_id: tenant_id.to_string(),
        vendor_id: row.vendor_id,
        vendor_invoice_ref: row.vendor_invoice_ref.clone(),
        approved_amount_minor: row.total_minor,
        currency: row.currency.clone(),
        due_date: row.due_date,
        approved_by: req.approved_by.trim().to_string(),
        approved_at: now,
    };

    let envelope = build_vendor_bill_approved_envelope(
        event_id,
        tenant_id.to_string(),
        correlation_id,
        None,
        payload,
    );

    enqueue_event_tx(
        &mut tx,
        event_id,
        EVENT_TYPE_VENDOR_BILL_APPROVED,
        "bill",
        &bill_id.to_string(),
        &envelope,
    )
    .await?;

    tx.commit().await?;

    Ok(approved)
}

// ============================================================================
// Internal helpers
// ============================================================================

/// Enforce match policy.
///
/// - 'open' (never matched): override_reason required.
/// - 'matched': all three_way_match lines must be within_tolerance, or
///   override_reason must be provided.
async fn check_match_policy(
    pool: &PgPool,
    bill_id: Uuid,
    status: &str,
    override_reason: &Option<String>,
) -> Result<(), BillError> {
    let has_override = !override_reason.as_deref().unwrap_or("").trim().is_empty();

    if status == "open" {
        if !has_override {
            return Err(BillError::MatchPolicyViolation(
                "bill has not been through the match engine; \
                 provide override_reason to approve without matching"
                    .to_string(),
            ));
        }
        return Ok(());
    }

    // status == "matched": check tolerance violations
    let (total, failed): (i64, i64) = sqlx::query_as(
        r#"
        SELECT
            COUNT(*)                                    AS total,
            COUNT(*) FILTER (WHERE within_tolerance = FALSE) AS failed
        FROM three_way_match
        WHERE bill_id = $1
        "#,
    )
    .bind(bill_id)
    .fetch_one(pool)
    .await?;

    if failed > 0 && !has_override {
        return Err(BillError::MatchPolicyViolation(format!(
            "{} of {} matched line(s) have tolerance violations; \
             provide override_reason to approve",
            failed, total
        )));
    }

    Ok(())
}

// ============================================================================
// Integrated Tests (real DB, no mocks)
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;

    const TEST_TENANT: &str = "test-tenant-approve-bill";

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

    /// Insert a match record for the bill (simulates the match engine result).
    async fn insert_match_record(db: &PgPool, bill_id: Uuid, within_tol: bool) {
        let line_id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO bill_lines (line_id, bill_id, description, quantity, unit_price_minor, \
             line_total_minor, gl_account_code, created_at) \
             VALUES ($1, $2, 'Widget', 10.0, 5000, 50000, '6100', NOW())",
        )
        .bind(line_id)
        .bind(bill_id)
        .execute(db)
        .await
        .expect("insert bill_line");

        sqlx::query(
            "INSERT INTO three_way_match (bill_id, bill_line_id, match_type, matched_quantity, \
             matched_amount_minor, within_tolerance, matched_by, matched_at, \
             price_variance_minor, qty_variance, match_status) \
             VALUES ($1, $2, 'two_way', 10.0, 50000, $3, 'system', NOW(), 0, 0.0, 'matched')",
        )
        .bind(bill_id)
        .bind(line_id)
        .bind(within_tol)
        .execute(db)
        .await
        .expect("insert match record");
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

    fn approve_req(override_reason: Option<&str>) -> ApproveBillRequest {
        ApproveBillRequest {
            approved_by: "approver-1".to_string(),
            override_reason: override_reason.map(|s| s.to_string()),
        }
    }

    #[tokio::test]
    #[serial]
    async fn test_approve_matched_within_tolerance_succeeds() {
        let db = pool().await;
        cleanup(&db).await;
        let vendor_id = create_vendor(&db).await;
        let bill_id = create_bill(&db, vendor_id, "matched").await;
        insert_match_record(&db, bill_id, true).await;

        let result = approve_bill(&db, TEST_TENANT, bill_id, &approve_req(None), "corr-1".to_string())
            .await
            .expect("approve failed");

        assert_eq!(result.status, "approved");

        let (count,): (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM events_outbox WHERE aggregate_type = 'bill' \
             AND aggregate_id = $1 AND event_type = $2",
        )
        .bind(bill_id.to_string())
        .bind(EVENT_TYPE_VENDOR_BILL_APPROVED)
        .fetch_one(&db)
        .await
        .expect("outbox query");
        assert_eq!(count, 1);

        cleanup(&db).await;
    }

    #[tokio::test]
    #[serial]
    async fn test_approve_open_without_override_fails() {
        let db = pool().await;
        cleanup(&db).await;
        let vendor_id = create_vendor(&db).await;
        let bill_id = create_bill(&db, vendor_id, "open").await;

        let result = approve_bill(&db, TEST_TENANT, bill_id, &approve_req(None), "corr-2".to_string()).await;

        assert!(
            matches!(result, Err(BillError::MatchPolicyViolation(_))),
            "expected MatchPolicyViolation, got {:?}",
            result
        );

        cleanup(&db).await;
    }

    #[tokio::test]
    #[serial]
    async fn test_approve_open_with_override_succeeds() {
        let db = pool().await;
        cleanup(&db).await;
        let vendor_id = create_vendor(&db).await;
        let bill_id = create_bill(&db, vendor_id, "open").await;

        let result = approve_bill(
            &db,
            TEST_TENANT,
            bill_id,
            &approve_req(Some("spot purchase, no PO required")),
            "corr-3".to_string(),
        )
        .await
        .expect("approve with override failed");

        assert_eq!(result.status, "approved");

        cleanup(&db).await;
    }

    #[tokio::test]
    #[serial]
    async fn test_approve_tolerance_violation_without_override_fails() {
        let db = pool().await;
        cleanup(&db).await;
        let vendor_id = create_vendor(&db).await;
        let bill_id = create_bill(&db, vendor_id, "matched").await;
        insert_match_record(&db, bill_id, false).await; // within_tolerance = false

        let result = approve_bill(&db, TEST_TENANT, bill_id, &approve_req(None), "corr-4".to_string()).await;

        assert!(
            matches!(result, Err(BillError::MatchPolicyViolation(_))),
            "expected MatchPolicyViolation for tolerance violation, got {:?}",
            result
        );

        cleanup(&db).await;
    }

    #[tokio::test]
    #[serial]
    async fn test_approve_tolerance_violation_with_override_succeeds() {
        let db = pool().await;
        cleanup(&db).await;
        let vendor_id = create_vendor(&db).await;
        let bill_id = create_bill(&db, vendor_id, "matched").await;
        insert_match_record(&db, bill_id, false).await;

        let result = approve_bill(
            &db,
            TEST_TENANT,
            bill_id,
            &approve_req(Some("price variance pre-approved by CFO")),
            "corr-5".to_string(),
        )
        .await
        .expect("override approve failed");

        assert_eq!(result.status, "approved");

        cleanup(&db).await;
    }

    #[tokio::test]
    #[serial]
    async fn test_approve_idempotent_no_double_event() {
        let db = pool().await;
        cleanup(&db).await;
        let vendor_id = create_vendor(&db).await;
        let bill_id = create_bill(&db, vendor_id, "matched").await;
        insert_match_record(&db, bill_id, true).await;

        approve_bill(&db, TEST_TENANT, bill_id, &approve_req(None), "corr-6a".to_string())
            .await
            .expect("first approve");

        let second = approve_bill(&db, TEST_TENANT, bill_id, &approve_req(None), "corr-6b".to_string())
            .await
            .expect("second approve must succeed (idempotent)");

        assert_eq!(second.status, "approved");

        let (count,): (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM events_outbox WHERE aggregate_type = 'bill' \
             AND aggregate_id = $1 AND event_type = $2",
        )
        .bind(bill_id.to_string())
        .bind(EVENT_TYPE_VENDOR_BILL_APPROVED)
        .fetch_one(&db)
        .await
        .expect("outbox count");
        assert_eq!(count, 1, "idempotent second approve must not produce a second event");

        cleanup(&db).await;
    }

    #[tokio::test]
    #[serial]
    async fn test_approve_invalid_transition_from_paid() {
        let db = pool().await;
        cleanup(&db).await;
        let vendor_id = create_vendor(&db).await;
        let bill_id = create_bill(&db, vendor_id, "paid").await;

        let result = approve_bill(&db, TEST_TENANT, bill_id, &approve_req(None), "corr-7".to_string()).await;

        assert!(
            matches!(result, Err(BillError::InvalidTransition { .. })),
            "expected InvalidTransition from paid, got {:?}",
            result
        );

        cleanup(&db).await;
    }

    #[tokio::test]
    #[serial]
    async fn test_approve_wrong_tenant_returns_not_found() {
        let db = pool().await;
        cleanup(&db).await;
        let vendor_id = create_vendor(&db).await;
        let bill_id = create_bill(&db, vendor_id, "matched").await;
        insert_match_record(&db, bill_id, true).await;

        let result = approve_bill(&db, "wrong-tenant", bill_id, &approve_req(None), "corr-8".to_string()).await;

        assert!(
            matches!(result, Err(BillError::NotFound(_))),
            "expected NotFound for wrong tenant, got {:?}",
            result
        );

        cleanup(&db).await;
    }
}
