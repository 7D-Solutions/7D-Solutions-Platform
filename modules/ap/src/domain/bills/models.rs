//! Shared model types for the bills domain.
//!
//! Contains internal DB row types, policy helpers, and test fixtures
//! shared across the bill domain modules (approve.rs, void.rs, etc.).

use chrono::{DateTime, Utc};
use sqlx::PgPool;
use uuid::Uuid;

use super::BillError;

// ============================================================================
// Internal DB row types
// ============================================================================

/// Internal bill header row used during approval.
///
/// Fetched with SELECT … FOR UPDATE to prevent concurrent mutations.
#[derive(sqlx::FromRow)]
pub(crate) struct BillHeaderRow {
    pub vendor_id: Uuid,
    pub vendor_invoice_ref: String,
    pub total_minor: i64,
    pub currency: String,
    pub due_date: DateTime<Utc>,
    pub status: String,
    /// Phase 23a FX identifier — None when bill currency == functional currency.
    pub fx_rate_id: Option<Uuid>,
}

/// Internal bill line row fetched during approval for GL posting allocations.
///
/// Used to build `ApprovedGlLine` entries inside the approved event payload,
/// making the event self-contained / replay-safe for the GL consumer.
#[derive(Debug, sqlx::FromRow)]
pub(crate) struct BillLineGlRow {
    pub line_id: Uuid,
    pub gl_account_code: String,
    pub line_total_minor: i64,
    pub po_line_id: Option<Uuid>,
}

// ============================================================================
// Policy helpers
// ============================================================================

/// Enforce the bill match policy for approval.
///
/// - `'open'` status (never matched): `override_reason` is required.
/// - `'matched'` status: all three_way_match lines must be `within_tolerance`,
///   or `override_reason` must be provided.
pub(crate) async fn check_match_policy(
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
// Test fixtures (shared across bill domain tests)
// ============================================================================

#[cfg(test)]
pub mod test_fixtures {
    use super::*;
    use crate::domain::bills::ApproveBillRequest;

    pub fn db_url() -> String {
        std::env::var("DATABASE_URL")
            .unwrap_or_else(|_| "postgres://ap_user:ap_pass@localhost:5443/ap_db".to_string())
    }

    pub async fn make_pool() -> PgPool {
        PgPool::connect(&db_url()).await.expect("DB connect failed")
    }

    pub async fn create_vendor(db: &PgPool, tenant_id: &str) -> Uuid {
        let vendor_id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO vendors (vendor_id, tenant_id, name, currency, payment_terms_days, \
             is_active, created_at, updated_at) VALUES ($1, $2, $3, 'USD', 30, TRUE, NOW(), NOW())",
        )
        .bind(vendor_id)
        .bind(tenant_id)
        .bind(format!("Vendor-{}", vendor_id))
        .execute(db)
        .await
        .expect("insert vendor");
        vendor_id
    }

    pub async fn create_bill_with_line(
        db: &PgPool,
        tenant_id: &str,
        vendor_id: Uuid,
        status: &str,
    ) -> Uuid {
        let bill_id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO vendor_bills (bill_id, tenant_id, vendor_id, vendor_invoice_ref, \
             currency, total_minor, invoice_date, due_date, status, entered_by, entered_at) \
             VALUES ($1, $2, $3, $4, 'USD', 50000, NOW(), NOW() + interval '30 days', \
             $5, 'system', NOW())",
        )
        .bind(bill_id)
        .bind(tenant_id)
        .bind(vendor_id)
        .bind(format!("INV-{}", &bill_id.to_string()[..8]))
        .bind(status)
        .execute(db)
        .await
        .expect("insert bill");
        // Insert a single bill line for GL account routing
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
        bill_id
    }

    pub async fn insert_match_record(db: &PgPool, bill_id: Uuid, within_tol: bool) {
        // Fetch the first bill line for this bill
        let (line_id,): (Uuid,) =
            sqlx::query_as("SELECT line_id FROM bill_lines WHERE bill_id = $1 LIMIT 1")
                .bind(bill_id)
                .fetch_one(db)
                .await
                .expect("fetch bill_line for match");

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

    pub async fn cleanup(db: &PgPool, tenant_id: &str) {
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
            sqlx::query(q).bind(tenant_id).execute(db).await.ok();
        }
    }

    pub fn approve_req(override_reason: Option<&str>) -> ApproveBillRequest {
        ApproveBillRequest {
            approved_by: "approver-1".to_string(),
            override_reason: override_reason.map(|s| s.to_string()),
        }
    }
}
