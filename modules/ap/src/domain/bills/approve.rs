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
//! Event: ap.vendor_bill_approved carries full actor attribution, GL line allocations,
//!        and the FX rate identifier for multi-currency GL posting.

use chrono::Utc;
use sqlx::PgPool;
use uuid::Uuid;

use tax_core::{TaxCommitRequest, TaxProvider};

use crate::events::{
    build_vendor_bill_approved_envelope, ApprovedGlLine, VendorBillApprovedPayload,
    EVENT_TYPE_VENDOR_BILL_APPROVED,
};
use crate::outbox::enqueue_event_tx;

use super::models::check_match_policy;
use super::{ApproveBillRequest, BillError, VendorBill};

// ============================================================================
// Public API
// ============================================================================

/// Approve a vendor bill, enforcing the match policy.
///
/// Guard:    Lock bill row; verify status; check match policy.
/// Mutation: UPDATE status = 'approved'.
/// Outbox:   ap.vendor_bill_approved enqueued atomically. Payload includes
///           per-line GL account allocations and the FX rate identifier.
///
/// Idempotent: if already 'approved', returns the current bill state (no re-emit).
pub async fn approve_bill(
    pool: &PgPool,
    tax_provider: &(impl TaxProvider + ?Sized),
    tenant_id: &str,
    bill_id: Uuid,
    req: &ApproveBillRequest,
    correlation_id: String,
) -> Result<VendorBill, BillError> {
    req.validate()?;

    let mut tx = pool.begin().await?;

    // Guard: lock the bill row to prevent concurrent approvals
    let row = super::repo::lock_bill_header(&mut *tx, bill_id, tenant_id)
        .await?
        .ok_or(BillError::NotFound(bill_id))?;

    // Idempotency: already approved → commit and return current state
    if row.status == "approved" {
        tx.commit().await?;
        let bill = super::repo::fetch_bill(pool, tenant_id, bill_id)
            .await?
            .ok_or(BillError::NotFound(bill_id))?;
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

    // Tax: commit any quoted tax snapshot for this bill.
    // If no snapshot exists, the bill is non-taxable — proceed normally.
    // If a snapshot is already committed, this is idempotent.
    let tax_snap = crate::domain::tax::repo::find_active_snapshot_tx(&mut *tx, bill_id).await?;

    if let Some(snap) = &tax_snap {
        if snap.status == "quoted" {
            let commit_req = TaxCommitRequest {
                tenant_id: tenant_id.to_string(),
                invoice_id: bill_id.to_string(),
                provider_quote_ref: snap.provider_quote_ref.clone(),
                correlation_id: correlation_id.clone(),
            };
            let commit_resp = tax_provider
                .commit_tax(commit_req)
                .await
                .map_err(|e| BillError::TaxError(format!("tax commit failed: {}", e)))?;

            crate::domain::tax::repo::commit_snapshot_tx(
                &mut *tx,
                snap.id,
                &commit_resp.provider_commit_ref,
                commit_resp.committed_at,
            )
            .await?;
        }
    }

    let now = Utc::now();
    let event_id = Uuid::new_v4();

    // Mutation: advance status to approved
    let approved = super::repo::approve_bill_status(&mut *tx, bill_id, tenant_id).await?;

    // Fetch bill lines for GL posting allocations (replay-safe event payload)
    let gl_line_rows = super::repo::fetch_bill_gl_lines(&mut *tx, bill_id).await?;

    let gl_lines: Vec<ApprovedGlLine> = gl_line_rows
        .into_iter()
        .map(|r| ApprovedGlLine {
            line_id: r.line_id,
            gl_account_code: r.gl_account_code,
            amount_minor: r.line_total_minor,
            po_line_id: r.po_line_id,
        })
        .collect();

    // Outbox: ap.vendor_bill_approved (self-contained for GL posting)
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
        fx_rate_id: row.fx_rate_id,
        gl_lines,
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
// Integrated Tests (real DB, no mocks)
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::bills::models::test_fixtures::{
        approve_req, cleanup, create_bill_with_line, create_vendor, insert_match_record, make_pool,
    };
    use crate::domain::tax::ZeroTaxProvider;
    use serial_test::serial;

    const TEST_TENANT: &str = "test-tenant-approve-bill";

    #[tokio::test]
    #[serial]
    async fn test_approve_matched_within_tolerance_succeeds() {
        let db = make_pool().await;
        cleanup(&db, TEST_TENANT).await;
        let vendor_id = create_vendor(&db, TEST_TENANT).await;
        let bill_id = create_bill_with_line(&db, TEST_TENANT, vendor_id, "matched").await;
        insert_match_record(&db, bill_id, true).await;

        let result = approve_bill(
            &db,
            &ZeroTaxProvider,
            TEST_TENANT,
            bill_id,
            &approve_req(None),
            "corr-1".to_string(),
        )
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

        cleanup(&db, TEST_TENANT).await;
    }

    #[tokio::test]
    #[serial]
    async fn test_approve_event_carries_gl_lines_and_fx_rate() {
        let db = make_pool().await;
        cleanup(&db, TEST_TENANT).await;
        let vendor_id = create_vendor(&db, TEST_TENANT).await;
        let bill_id = create_bill_with_line(&db, TEST_TENANT, vendor_id, "matched").await;
        insert_match_record(&db, bill_id, true).await;

        approve_bill(
            &db,
            &ZeroTaxProvider,
            TEST_TENANT,
            bill_id,
            &approve_req(None),
            "corr-gllines".to_string(),
        )
        .await
        .expect("approve failed");

        // Verify the outbox event payload contains gl_lines
        let (payload_json,): (serde_json::Value,) = sqlx::query_as(
            "SELECT payload FROM events_outbox WHERE aggregate_type = 'bill' \
             AND aggregate_id = $1 AND event_type = $2",
        )
        .bind(bill_id.to_string())
        .bind(EVENT_TYPE_VENDOR_BILL_APPROVED)
        .fetch_one(&db)
        .await
        .expect("outbox payload");

        let gl_lines = payload_json["payload"]["gl_lines"]
            .as_array()
            .expect("gl_lines field");
        // We inserted one line in create_bill_with_line (50000 minor, '6100')
        assert_eq!(gl_lines.len(), 1, "one GL line expected");
        assert_eq!(gl_lines[0]["gl_account_code"].as_str(), Some("6100"));
        assert_eq!(gl_lines[0]["amount_minor"].as_i64(), Some(50000));

        cleanup(&db, TEST_TENANT).await;
    }

    #[tokio::test]
    #[serial]
    async fn test_approve_open_without_override_fails() {
        let db = make_pool().await;
        cleanup(&db, TEST_TENANT).await;
        let vendor_id = create_vendor(&db, TEST_TENANT).await;
        let bill_id = create_bill_with_line(&db, TEST_TENANT, vendor_id, "open").await;

        let result = approve_bill(
            &db,
            &ZeroTaxProvider,
            TEST_TENANT,
            bill_id,
            &approve_req(None),
            "corr-2".to_string(),
        )
        .await;

        assert!(
            matches!(result, Err(BillError::MatchPolicyViolation(_))),
            "expected MatchPolicyViolation, got {:?}",
            result
        );

        cleanup(&db, TEST_TENANT).await;
    }

    #[tokio::test]
    #[serial]
    async fn test_approve_open_with_override_succeeds() {
        let db = make_pool().await;
        cleanup(&db, TEST_TENANT).await;
        let vendor_id = create_vendor(&db, TEST_TENANT).await;
        let bill_id = create_bill_with_line(&db, TEST_TENANT, vendor_id, "open").await;

        let result = approve_bill(
            &db,
            &ZeroTaxProvider,
            TEST_TENANT,
            bill_id,
            &approve_req(Some("spot purchase, no PO required")),
            "corr-3".to_string(),
        )
        .await
        .expect("approve with override failed");

        assert_eq!(result.status, "approved");

        cleanup(&db, TEST_TENANT).await;
    }

    #[tokio::test]
    #[serial]
    async fn test_approve_tolerance_violation_without_override_fails() {
        let db = make_pool().await;
        cleanup(&db, TEST_TENANT).await;
        let vendor_id = create_vendor(&db, TEST_TENANT).await;
        let bill_id = create_bill_with_line(&db, TEST_TENANT, vendor_id, "matched").await;
        insert_match_record(&db, bill_id, false).await; // within_tolerance = false

        let result = approve_bill(
            &db,
            &ZeroTaxProvider,
            TEST_TENANT,
            bill_id,
            &approve_req(None),
            "corr-4".to_string(),
        )
        .await;

        assert!(
            matches!(result, Err(BillError::MatchPolicyViolation(_))),
            "expected MatchPolicyViolation for tolerance violation, got {:?}",
            result
        );

        cleanup(&db, TEST_TENANT).await;
    }

    #[tokio::test]
    #[serial]
    async fn test_approve_tolerance_violation_with_override_succeeds() {
        let db = make_pool().await;
        cleanup(&db, TEST_TENANT).await;
        let vendor_id = create_vendor(&db, TEST_TENANT).await;
        let bill_id = create_bill_with_line(&db, TEST_TENANT, vendor_id, "matched").await;
        insert_match_record(&db, bill_id, false).await;

        let result = approve_bill(
            &db,
            &ZeroTaxProvider,
            TEST_TENANT,
            bill_id,
            &approve_req(Some("price variance pre-approved by CFO")),
            "corr-5".to_string(),
        )
        .await
        .expect("override approve failed");

        assert_eq!(result.status, "approved");

        cleanup(&db, TEST_TENANT).await;
    }

    #[tokio::test]
    #[serial]
    async fn test_approve_idempotent_no_double_event() {
        let db = make_pool().await;
        cleanup(&db, TEST_TENANT).await;
        let vendor_id = create_vendor(&db, TEST_TENANT).await;
        let bill_id = create_bill_with_line(&db, TEST_TENANT, vendor_id, "matched").await;
        insert_match_record(&db, bill_id, true).await;

        approve_bill(
            &db,
            &ZeroTaxProvider,
            TEST_TENANT,
            bill_id,
            &approve_req(None),
            "corr-6a".to_string(),
        )
        .await
        .expect("first approve");

        let second = approve_bill(
            &db,
            &ZeroTaxProvider,
            TEST_TENANT,
            bill_id,
            &approve_req(None),
            "corr-6b".to_string(),
        )
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
        assert_eq!(
            count, 1,
            "idempotent second approve must not produce a second event"
        );

        cleanup(&db, TEST_TENANT).await;
    }

    #[tokio::test]
    #[serial]
    async fn test_approve_invalid_transition_from_paid() {
        let db = make_pool().await;
        cleanup(&db, TEST_TENANT).await;
        let vendor_id = create_vendor(&db, TEST_TENANT).await;
        let bill_id = create_bill_with_line(&db, TEST_TENANT, vendor_id, "paid").await;

        let result = approve_bill(
            &db,
            &ZeroTaxProvider,
            TEST_TENANT,
            bill_id,
            &approve_req(None),
            "corr-7".to_string(),
        )
        .await;

        assert!(
            matches!(result, Err(BillError::InvalidTransition { .. })),
            "expected InvalidTransition from paid, got {:?}",
            result
        );

        cleanup(&db, TEST_TENANT).await;
    }

    #[tokio::test]
    #[serial]
    async fn test_approve_wrong_tenant_returns_not_found() {
        let db = make_pool().await;
        cleanup(&db, TEST_TENANT).await;
        let vendor_id = create_vendor(&db, TEST_TENANT).await;
        let bill_id = create_bill_with_line(&db, TEST_TENANT, vendor_id, "matched").await;
        insert_match_record(&db, bill_id, true).await;

        let result = approve_bill(
            &db,
            &ZeroTaxProvider,
            "wrong-tenant",
            bill_id,
            &approve_req(None),
            "corr-8".to_string(),
        )
        .await;

        assert!(
            matches!(result, Err(BillError::NotFound(_))),
            "expected NotFound for wrong tenant, got {:?}",
            result
        );

        cleanup(&db, TEST_TENANT).await;
    }
}
