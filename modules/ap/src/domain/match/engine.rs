//! 3-way match engine: deterministic comparison of bill ↔ PO ↔ receipts.
//!
//! Algorithm (per bill line):
//!   If po_line_id is set:
//!     • Lookup PO line (ordered qty + unit price)
//!     • Sum receipt links for po_line_id (if any)
//!     • If receipts exist → three_way, else → two_way
//!     • matched_qty = min(bill_qty, received_qty)  for three_way
//!                      min(bill_qty, po_qty)        for two_way
//!     • price_variance = (bill_price - po_price) × matched_qty
//!     • qty_variance   = bill_qty - comparison_qty
//!     • within_tolerance when |price_var| ≤ tolerance AND |qty_var| < 1e-6
//!   Otherwise → non_po (no comparison; always within_tolerance)
//!
//! Idempotency: INSERT … ON CONFLICT (bill_line_id) DO NOTHING.
//! Re-running the engine for the same bill does not duplicate match records.
//!
//! Outbox: vendor_bill_matched enqueued in the same transaction as match records.

use chrono::Utc;
use sqlx::PgPool;
use uuid::Uuid;

use crate::events::{
    build_vendor_bill_matched_envelope, BillMatchLine, MatchType, VendorBillMatchedPayload,
    EVENT_TYPE_VENDOR_BILL_MATCHED,
};
use crate::outbox::enqueue_event_tx;

use super::{MatchError, MatchLineResult, MatchOutcome, MatchStatus, RunMatchRequest};

// ============================================================================
// Internal DB row types
// ============================================================================

#[derive(sqlx::FromRow)]
struct BillRow {
    vendor_id: Uuid,
    status: String,
}

#[derive(sqlx::FromRow)]
struct BillLineRow {
    line_id: Uuid,
    quantity: f64,
    unit_price_minor: i64,
    po_line_id: Option<Uuid>,
}

#[derive(sqlx::FromRow)]
struct PoLineRow {
    line_id: Uuid,
    quantity: f64,
    unit_price_minor: i64,
}

/// Aggregated receipt totals for one PO line.
struct ReceiptAgg {
    po_line_id: Uuid,
    total_received: f64,
    first_receipt_id: Uuid,
}

// ============================================================================
// Public API
// ============================================================================

/// Run the 3-way match engine for a vendor bill against a PO.
///
/// Guard → Mutation → Outbox:
///   Guard:    Load bill, PO, receipt aggregates; validate status.
///   Mutation: Insert match records (ON CONFLICT DO NOTHING); update bill status.
///   Outbox:   Enqueue vendor_bill_matched atomically in the same transaction.
pub async fn run_match(
    pool: &PgPool,
    tenant_id: &str,
    bill_id: Uuid,
    req: &RunMatchRequest,
    correlation_id: String,
) -> Result<MatchOutcome, MatchError> {
    req.validate()?;

    // Guard: load bill header (tenant-scoped)
    let bill: Option<BillRow> = sqlx::query_as(
        "SELECT vendor_id, status FROM vendor_bills WHERE bill_id = $1 AND tenant_id = $2",
    )
    .bind(bill_id)
    .bind(tenant_id)
    .fetch_optional(pool)
    .await?;

    let bill = bill.ok_or(MatchError::BillNotFound(bill_id))?;

    if !matches!(bill.status.as_str(), "open" | "matched") {
        return Err(MatchError::InvalidBillStatus(bill.status));
    }

    // Guard: verify PO belongs to this tenant
    let po_exists: Option<(Uuid,)> = sqlx::query_as(
        "SELECT po_id FROM purchase_orders WHERE po_id = $1 AND tenant_id = $2",
    )
    .bind(req.po_id)
    .bind(tenant_id)
    .fetch_optional(pool)
    .await?;

    if po_exists.is_none() {
        return Err(MatchError::PoNotFound(req.po_id));
    }

    // Guard: load bill lines
    let bill_lines: Vec<BillLineRow> = sqlx::query_as(
        "SELECT line_id, quantity, unit_price_minor, po_line_id \
         FROM bill_lines WHERE bill_id = $1 ORDER BY created_at ASC",
    )
    .bind(bill_id)
    .fetch_all(pool)
    .await?;

    if bill_lines.is_empty() {
        return Err(MatchError::NoMatchableLines);
    }

    // Guard: load PO lines for this PO
    let po_lines: Vec<PoLineRow> = sqlx::query_as(
        "SELECT line_id, quantity::FLOAT8 AS quantity, unit_price_minor \
         FROM po_lines WHERE po_id = $1",
    )
    .bind(req.po_id)
    .fetch_all(pool)
    .await?;

    // Guard: aggregate receipt quantities per po_line_id referenced by bill lines
    let ref_po_line_ids: Vec<Uuid> = bill_lines
        .iter()
        .filter_map(|bl| bl.po_line_id)
        .collect::<std::collections::HashSet<_>>()
        .into_iter()
        .collect();

    let receipt_aggs = load_receipt_aggs(pool, &ref_po_line_ids).await?;

    let now = Utc::now();
    let matched_by = req.matched_by.trim().to_string();
    let event_id = Uuid::new_v4();

    // Compute deterministic match results per line
    let match_lines: Vec<MatchLineResult> = bill_lines
        .iter()
        .map(|bl| compute_line_match(bl, &po_lines, &receipt_aggs, req.price_tolerance_pct))
        .collect();

    let fully_matched = match_lines.iter().all(|l| l.within_tolerance);

    // Build event match lines (PO-linked lines only)
    let event_match_lines: Vec<BillMatchLine> = match_lines
        .iter()
        .filter_map(|l| {
            l.po_line_id.map(|po_line_id| BillMatchLine {
                bill_line_id: l.bill_line_id,
                po_line_id,
                receipt_id: l.receipt_id,
                matched_quantity: l.matched_quantity,
                matched_amount_minor: l.matched_amount_minor,
                within_tolerance: l.within_tolerance,
            })
        })
        .collect();

    // Determine overall match type for the event
    let overall_match_type = if match_lines.iter().any(|l| l.match_type == "three_way") {
        MatchType::ThreeWay
    } else if match_lines.iter().any(|l| l.match_type == "two_way") {
        MatchType::TwoWay
    } else {
        MatchType::NonPo
    };

    // Mutation + Outbox (atomic transaction)
    let mut tx = pool.begin().await?;

    for line in &match_lines {
        sqlx::query(
            r#"
            INSERT INTO three_way_match (
                bill_id, bill_line_id, po_id, po_line_id, receipt_id,
                match_type, matched_quantity, matched_amount_minor, within_tolerance,
                price_variance_minor, qty_variance, match_status,
                matched_by, matched_at
            )
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14)
            ON CONFLICT (bill_line_id) DO NOTHING
            "#,
        )
        .bind(bill_id)
        .bind(line.bill_line_id)
        .bind(if line.po_line_id.is_some() { Some(req.po_id) } else { None::<Uuid> })
        .bind(line.po_line_id)
        .bind(line.receipt_id)
        .bind(&line.match_type)
        .bind(line.matched_quantity)
        .bind(line.matched_amount_minor)
        .bind(line.within_tolerance)
        .bind(line.price_variance_minor)
        .bind(line.qty_variance)
        .bind(&line.match_status)
        .bind(&matched_by)
        .bind(now)
        .execute(&mut *tx)
        .await?;
    }

    // Update bill status to 'matched' (idempotent: WHERE status = 'open')
    sqlx::query(
        "UPDATE vendor_bills SET status = 'matched' \
         WHERE bill_id = $1 AND status = 'open'",
    )
    .bind(bill_id)
    .execute(&mut *tx)
    .await?;

    // Outbox: enqueue vendor_bill_matched
    let payload = VendorBillMatchedPayload {
        bill_id,
        tenant_id: tenant_id.to_string(),
        vendor_id: bill.vendor_id,
        po_id: req.po_id,
        match_type: overall_match_type,
        match_lines: event_match_lines,
        fully_matched,
        matched_by: matched_by.clone(),
        matched_at: now,
    };

    let envelope = build_vendor_bill_matched_envelope(
        event_id,
        tenant_id.to_string(),
        correlation_id,
        None,
        payload,
    );

    enqueue_event_tx(
        &mut tx,
        event_id,
        EVENT_TYPE_VENDOR_BILL_MATCHED,
        "bill",
        &bill_id.to_string(),
        &envelope,
    )
    .await?;

    tx.commit().await?;

    Ok(MatchOutcome {
        bill_id,
        po_id: req.po_id,
        lines: match_lines,
        fully_matched,
        matched_by,
        matched_at: now,
    })
}

// ============================================================================
// Internal helpers
// ============================================================================

/// Aggregate received quantities per PO line from po_receipt_links.
async fn load_receipt_aggs(
    pool: &PgPool,
    po_line_ids: &[Uuid],
) -> Result<Vec<ReceiptAgg>, MatchError> {
    let mut aggs = Vec::new();
    for &po_line_id in po_line_ids {
        let row: Option<(f64, Uuid)> = sqlx::query_as(
            r#"
            SELECT
                SUM(quantity_received::FLOAT8) AS total_received,
                MIN(receipt_id)               AS first_receipt_id
            FROM po_receipt_links
            WHERE po_line_id = $1
            GROUP BY po_line_id
            "#,
        )
        .bind(po_line_id)
        .fetch_optional(pool)
        .await?;

        if let Some((total_received, first_receipt_id)) = row {
            aggs.push(ReceiptAgg { po_line_id, total_received, first_receipt_id });
        }
    }
    Ok(aggs)
}

/// Compute match result for a single bill line against PO lines and receipts.
fn compute_line_match(
    bl: &BillLineRow,
    po_lines: &[PoLineRow],
    receipt_aggs: &[ReceiptAgg],
    price_tolerance_pct: f64,
) -> MatchLineResult {
    let Some(po_line_id) = bl.po_line_id else {
        // non_po: no PO reference — treat as fully matched
        return MatchLineResult {
            bill_line_id: bl.line_id,
            po_line_id: None,
            receipt_id: None,
            match_type: "non_po".to_string(),
            matched_quantity: bl.quantity,
            matched_amount_minor: (bl.quantity * bl.unit_price_minor as f64).round() as i64,
            price_variance_minor: 0,
            qty_variance: 0.0,
            within_tolerance: true,
            match_status: MatchStatus::Matched.as_str().to_string(),
        };
    };

    let Some(po_line) = po_lines.iter().find(|pl| pl.line_id == po_line_id) else {
        // PO line not found in this PO — treat defensively as non_po
        return MatchLineResult {
            bill_line_id: bl.line_id,
            po_line_id: Some(po_line_id),
            receipt_id: None,
            match_type: "non_po".to_string(),
            matched_quantity: bl.quantity,
            matched_amount_minor: (bl.quantity * bl.unit_price_minor as f64).round() as i64,
            price_variance_minor: 0,
            qty_variance: 0.0,
            within_tolerance: true,
            match_status: MatchStatus::Matched.as_str().to_string(),
        };
    };

    let receipt_agg = receipt_aggs.iter().find(|ra| ra.po_line_id == po_line_id);

    let (match_type, comparison_qty, receipt_id) = match receipt_agg {
        Some(agg) => ("three_way", agg.total_received, Some(agg.first_receipt_id)),
        None => ("two_way", po_line.quantity, None),
    };

    let matched_qty = f64::min(bl.quantity, comparison_qty);
    let qty_variance = bl.quantity - comparison_qty;

    // Price variance in minor currency units (signed)
    let price_variance_minor =
        ((bl.unit_price_minor - po_line.unit_price_minor) as f64 * matched_qty).round() as i64;

    // Amount matched valued at PO price
    let matched_amount_minor =
        (po_line.unit_price_minor as f64 * matched_qty).round() as i64;

    // Tolerance checks
    let price_tolerance_minor =
        (po_line.unit_price_minor as f64 * matched_qty * price_tolerance_pct).round() as i64;
    let price_ok = price_variance_minor.abs() <= price_tolerance_minor;
    let qty_ok = qty_variance.abs() < 1e-6;

    let within_tolerance = price_ok && qty_ok;
    let match_status = match (price_ok, qty_ok) {
        (true, true) => MatchStatus::Matched,
        (false, true) => MatchStatus::PriceVariance,
        (true, false) => MatchStatus::QtyVariance,
        (false, false) => MatchStatus::PriceAndQtyVariance,
    };

    MatchLineResult {
        bill_line_id: bl.line_id,
        po_line_id: Some(po_line_id),
        receipt_id,
        match_type: match_type.to_string(),
        matched_quantity: matched_qty,
        matched_amount_minor,
        price_variance_minor,
        qty_variance,
        within_tolerance,
        match_status: match_status.as_str().to_string(),
    }
}

// ============================================================================
// Integrated Tests (real DB, no mocks)
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use serial_test::serial;

    const TEST_TENANT: &str = "test-tenant-match-engine";

    fn db_url() -> String {
        std::env::var("DATABASE_URL")
            .unwrap_or_else(|_| "postgres://ap_user:ap_pass@localhost:5443/ap_db".to_string())
    }

    async fn pool() -> PgPool {
        PgPool::connect(&db_url()).await.expect("DB connect failed")
    }

    /// Insert vendor → PO → PO line; returns (vendor_id, po_id, po_line_id, po_unit_price).
    async fn setup_po(db: &PgPool, po_unit_price: i64) -> (Uuid, Uuid, Uuid) {
        let vendor_id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO vendors (vendor_id, tenant_id, name, currency, payment_terms_days, \
             is_active, created_at, updated_at) VALUES ($1, $2, $3, 'USD', 30, TRUE, NOW(), NOW())",
        )
        .bind(vendor_id).bind(TEST_TENANT).bind(format!("V-{}", vendor_id))
        .execute(db).await.expect("insert vendor");

        let po_id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO purchase_orders (po_id, tenant_id, vendor_id, po_number, currency, \
             total_minor, status, created_by, created_at) \
             VALUES ($1, $2, $3, $4, 'USD', $5, 'approved', 'system', NOW())",
        )
        .bind(po_id).bind(TEST_TENANT).bind(vendor_id)
        .bind(format!("PO-{}", &po_id.to_string()[..8]))
        .bind(10 * po_unit_price)
        .execute(db).await.expect("insert PO");

        let line_id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO po_lines (line_id, po_id, description, quantity, unit_of_measure, \
             unit_price_minor, line_total_minor, gl_account_code, created_at) \
             VALUES ($1, $2, 'Widgets', 10.0, 'each', $3, $4, '6100', NOW())",
        )
        .bind(line_id).bind(po_id).bind(po_unit_price).bind(10 * po_unit_price)
        .execute(db).await.expect("insert PO line");

        (vendor_id, po_id, line_id)
    }

    /// Insert vendor bill + bill line referencing a PO line; returns (bill_id, bill_line_id).
    async fn setup_bill(
        db: &PgPool,
        vendor_id: Uuid,
        po_line_id: Uuid,
        bill_qty: f64,
        bill_unit_price: i64,
    ) -> (Uuid, Uuid) {
        let bill_id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO vendor_bills (bill_id, tenant_id, vendor_id, vendor_invoice_ref, \
             currency, total_minor, invoice_date, due_date, status, entered_by, entered_at) \
             VALUES ($1, $2, $3, $4, 'USD', $5, NOW(), NOW() + interval '30 days', \
             'open', 'system', NOW())",
        )
        .bind(bill_id).bind(TEST_TENANT).bind(vendor_id)
        .bind(format!("INV-{}", &bill_id.to_string()[..8]))
        .bind((bill_qty * bill_unit_price as f64).round() as i64)
        .execute(db).await.expect("insert bill");

        let line_id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO bill_lines (line_id, bill_id, description, quantity, unit_price_minor, \
             line_total_minor, gl_account_code, po_line_id, created_at) \
             VALUES ($1, $2, 'Widgets', $3, $4, $5, '6100', $6, NOW())",
        )
        .bind(line_id).bind(bill_id).bind(bill_qty).bind(bill_unit_price)
        .bind((bill_qty * bill_unit_price as f64).round() as i64)
        .bind(po_line_id)
        .execute(db).await.expect("insert bill line");

        (bill_id, line_id)
    }

    /// Insert a receipt link for a PO line; returns receipt_id.
    async fn setup_receipt(db: &PgPool, po_id: Uuid, po_line_id: Uuid, vendor_id: Uuid, qty: f64) -> Uuid {
        let receipt_id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO po_receipt_links (po_id, po_line_id, vendor_id, receipt_id, \
             quantity_received, unit_of_measure, unit_price_minor, currency, \
             gl_account_code, received_at, received_by) \
             VALUES ($1, $2, $3, $4, $5, 'each', 1000, 'USD', '6100', NOW(), 'system')",
        )
        .bind(po_id).bind(po_line_id).bind(vendor_id).bind(receipt_id).bind(qty)
        .execute(db).await.expect("insert receipt");
        receipt_id
    }

    async fn cleanup(db: &PgPool) {
        // Delete in FK-safe order
        for q in [
            "DELETE FROM three_way_match WHERE bill_id IN \
             (SELECT bill_id FROM vendor_bills WHERE tenant_id = $1)",
            "DELETE FROM events_outbox WHERE aggregate_type = 'bill' \
             AND aggregate_id IN (SELECT bill_id::TEXT FROM vendor_bills WHERE tenant_id = $1)",
            "DELETE FROM bill_lines WHERE bill_id IN \
             (SELECT bill_id FROM vendor_bills WHERE tenant_id = $1)",
            "DELETE FROM vendor_bills WHERE tenant_id = $1",
            "DELETE FROM po_receipt_links WHERE po_id IN \
             (SELECT po_id FROM purchase_orders WHERE tenant_id = $1)",
            "DELETE FROM po_lines WHERE po_id IN \
             (SELECT po_id FROM purchase_orders WHERE tenant_id = $1)",
            "DELETE FROM po_status WHERE po_id IN \
             (SELECT po_id FROM purchase_orders WHERE tenant_id = $1)",
            "DELETE FROM events_outbox WHERE aggregate_type = 'po' \
             AND aggregate_id IN (SELECT po_id::TEXT FROM purchase_orders WHERE tenant_id = $1)",
            "DELETE FROM purchase_orders WHERE tenant_id = $1",
            "DELETE FROM events_outbox WHERE aggregate_type = 'vendor' \
             AND aggregate_id IN (SELECT vendor_id::TEXT FROM vendors WHERE tenant_id = $1)",
            "DELETE FROM vendors WHERE tenant_id = $1",
        ] {
            sqlx::query(q).bind(TEST_TENANT).execute(db).await.ok();
        }
    }

    fn match_req(po_id: Uuid) -> RunMatchRequest {
        RunMatchRequest { po_id, matched_by: "ap-user".to_string(), price_tolerance_pct: 0.05 }
    }

    #[tokio::test]
    #[serial]
    async fn test_two_way_match_exact_produces_matched_status() {
        let db = pool().await;
        cleanup(&db).await;

        let (vendor_id, po_id, po_line_id) = setup_po(&db, 1000).await;
        let (bill_id, bill_line_id) = setup_bill(&db, vendor_id, po_line_id, 10.0, 1000).await;

        let outcome = run_match(&db, TEST_TENANT, bill_id, &match_req(po_id), "corr-1".to_string())
            .await
            .expect("match failed");

        assert!(outcome.fully_matched);
        assert_eq!(outcome.lines.len(), 1);
        let line = &outcome.lines[0];
        assert_eq!(line.bill_line_id, bill_line_id);
        assert_eq!(line.match_type, "two_way");
        assert_eq!(line.match_status, "matched");
        assert_eq!(line.price_variance_minor, 0);
        assert!(line.qty_variance.abs() < 1e-6);

        // Verify match record persisted
        let (count,): (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM three_way_match WHERE bill_id = $1",
        )
        .bind(bill_id).fetch_one(&db).await.expect("count");
        assert_eq!(count, 1);

        // Verify bill status updated to 'matched'
        let (status,): (String,) = sqlx::query_as(
            "SELECT status FROM vendor_bills WHERE bill_id = $1",
        )
        .bind(bill_id).fetch_one(&db).await.expect("status");
        assert_eq!(status, "matched");

        // Verify outbox event enqueued
        let (ev_count,): (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM events_outbox WHERE aggregate_type = 'bill' AND aggregate_id = $1",
        )
        .bind(bill_id.to_string()).fetch_one(&db).await.expect("outbox");
        assert!(ev_count >= 1);

        cleanup(&db).await;
    }

    #[tokio::test]
    #[serial]
    async fn test_three_way_match_with_receipt() {
        let db = pool().await;
        cleanup(&db).await;

        let (vendor_id, po_id, po_line_id) = setup_po(&db, 1000).await;
        setup_receipt(&db, po_id, po_line_id, vendor_id, 10.0).await;
        let (bill_id, _) = setup_bill(&db, vendor_id, po_line_id, 10.0, 1000).await;

        let outcome = run_match(&db, TEST_TENANT, bill_id, &match_req(po_id), "corr-2".to_string())
            .await
            .expect("three_way match failed");

        assert!(outcome.fully_matched);
        let line = &outcome.lines[0];
        assert_eq!(line.match_type, "three_way");
        assert!(line.receipt_id.is_some());
        assert_eq!(line.match_status, "matched");

        cleanup(&db).await;
    }

    #[tokio::test]
    #[serial]
    async fn test_idempotent_second_run_does_not_duplicate_records() {
        let db = pool().await;
        cleanup(&db).await;

        let (vendor_id, po_id, po_line_id) = setup_po(&db, 1000).await;
        let (bill_id, _) = setup_bill(&db, vendor_id, po_line_id, 10.0, 1000).await;

        run_match(&db, TEST_TENANT, bill_id, &match_req(po_id), "corr-3a".to_string())
            .await.expect("first run failed");
        run_match(&db, TEST_TENANT, bill_id, &match_req(po_id), "corr-3b".to_string())
            .await.expect("second run must not error");

        let (count,): (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM three_way_match WHERE bill_id = $1",
        )
        .bind(bill_id).fetch_one(&db).await.expect("count");
        assert_eq!(count, 1, "re-run must not create duplicate match records");

        cleanup(&db).await;
    }

    #[tokio::test]
    #[serial]
    async fn test_price_variance_detected() {
        let db = pool().await;
        cleanup(&db).await;

        // PO price = 1000; bill price = 1200 (20% over — outside 5% tolerance)
        let (vendor_id, po_id, po_line_id) = setup_po(&db, 1000).await;
        let (bill_id, _) = setup_bill(&db, vendor_id, po_line_id, 10.0, 1200).await;

        let outcome = run_match(&db, TEST_TENANT, bill_id, &match_req(po_id), "corr-4".to_string())
            .await
            .expect("match failed");

        assert!(!outcome.fully_matched);
        let line = &outcome.lines[0];
        assert_eq!(line.match_status, "price_variance");
        assert!(!line.within_tolerance);
        // price_variance = (1200 - 1000) * 10 = 2000 minor units
        assert_eq!(line.price_variance_minor, 2000);

        cleanup(&db).await;
    }
}
