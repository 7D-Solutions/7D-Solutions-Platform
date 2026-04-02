//! Match service: orchestrates the 3-way matching algorithm.
//!
//! Pure matching logic (`compute_line_match`) is separated from the
//! orchestration layer (`run_match`) which coordinates repo queries,
//! computation, persistence, and outbox emission.
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

use chrono::Utc;
use sqlx::PgPool;
use uuid::Uuid;

use crate::events::{
    build_vendor_bill_matched_envelope, BillMatchLine, MatchType, VendorBillMatchedPayload,
    EVENT_TYPE_VENDOR_BILL_MATCHED,
};
use crate::outbox::enqueue_event_tx;

use super::repo::{self, BillLineRow, PoLineRow, ReceiptAgg};
use super::{MatchError, MatchLineResult, MatchOutcome, MatchStatus, RunMatchRequest};

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

    // Guard: load and validate
    let bill = repo::load_bill(pool, bill_id, tenant_id).await?;

    if !matches!(bill.status.as_str(), "open" | "matched") {
        return Err(MatchError::InvalidBillStatus(bill.status));
    }

    repo::verify_po_exists(pool, req.po_id, tenant_id).await?;

    let bill_lines = repo::load_bill_lines(pool, bill_id).await?;
    let po_lines = repo::load_po_lines(pool, req.po_id).await?;

    let ref_po_line_ids: Vec<Uuid> = bill_lines
        .iter()
        .filter_map(|bl| bl.po_line_id)
        .collect::<std::collections::HashSet<_>>()
        .into_iter()
        .collect();

    let receipt_aggs = repo::load_receipt_aggs(pool, &ref_po_line_ids).await?;

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
        let po_id = if line.po_line_id.is_some() {
            Some(req.po_id)
        } else {
            None
        };
        repo::insert_match_record(&mut tx, bill_id, po_id, line, &matched_by, now).await?;
    }

    repo::update_bill_status_matched(&mut tx, bill_id).await?;

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
// Pure matching logic
// ============================================================================

/// Compute match result for a single bill line against PO lines and receipts.
/// This is a pure function — no I/O, deterministic for the same inputs.
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
    let matched_amount_minor = (po_line.unit_price_minor as f64 * matched_qty).round() as i64;

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
