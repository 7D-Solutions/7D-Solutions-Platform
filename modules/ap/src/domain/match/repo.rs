//! Repository layer for the 3-way match engine.
//!
//! All database access for matching lives here: row types, guard queries,
//! mutation inserts, and status updates. Read functions take `&PgPool`;
//! write functions take a mutable transaction reference for atomicity.

use chrono::{DateTime, Utc};
use sqlx::PgPool;
use uuid::Uuid;

use super::{MatchError, MatchLineResult};

// ============================================================================
// DB row types (internal to the match domain)
// ============================================================================

#[derive(sqlx::FromRow)]
pub(super) struct BillRow {
    pub vendor_id: Uuid,
    pub status: String,
}

#[derive(sqlx::FromRow)]
pub(super) struct BillLineRow {
    pub line_id: Uuid,
    pub quantity: f64,
    pub unit_price_minor: i64,
    pub po_line_id: Option<Uuid>,
}

#[derive(sqlx::FromRow)]
pub(super) struct PoLineRow {
    pub line_id: Uuid,
    pub quantity: f64,
    pub unit_price_minor: i64,
}

/// Aggregated receipt totals for one PO line.
pub(super) struct ReceiptAgg {
    pub po_line_id: Uuid,
    pub total_received: f64,
    pub first_receipt_id: Uuid,
}

// ============================================================================
// Guard queries (reads)
// ============================================================================

/// Load bill header, scoped to tenant.
pub(super) async fn load_bill(
    pool: &PgPool,
    bill_id: Uuid,
    tenant_id: &str,
) -> Result<BillRow, MatchError> {
    let row: Option<BillRow> = sqlx::query_as(
        "SELECT vendor_id, status FROM vendor_bills WHERE bill_id = $1 AND tenant_id = $2",
    )
    .bind(bill_id)
    .bind(tenant_id)
    .fetch_optional(pool)
    .await?;

    row.ok_or(MatchError::BillNotFound(bill_id))
}

/// Verify that a PO exists for this tenant.
pub(super) async fn verify_po_exists(
    pool: &PgPool,
    po_id: Uuid,
    tenant_id: &str,
) -> Result<(), MatchError> {
    let exists: Option<(Uuid,)> =
        sqlx::query_as("SELECT po_id FROM purchase_orders WHERE po_id = $1 AND tenant_id = $2")
            .bind(po_id)
            .bind(tenant_id)
            .fetch_optional(pool)
            .await?;

    if exists.is_none() {
        return Err(MatchError::PoNotFound(po_id));
    }
    Ok(())
}

/// Load all bill lines for a bill, ordered by creation time.
pub(super) async fn load_bill_lines(
    pool: &PgPool,
    bill_id: Uuid,
) -> Result<Vec<BillLineRow>, MatchError> {
    let lines: Vec<BillLineRow> = sqlx::query_as(
        "SELECT line_id, quantity, unit_price_minor, po_line_id \
         FROM bill_lines WHERE bill_id = $1 ORDER BY created_at ASC",
    )
    .bind(bill_id)
    .fetch_all(pool)
    .await?;

    if lines.is_empty() {
        return Err(MatchError::NoMatchableLines);
    }
    Ok(lines)
}

/// Load PO lines for a purchase order.
pub(super) async fn load_po_lines(
    pool: &PgPool,
    po_id: Uuid,
) -> Result<Vec<PoLineRow>, MatchError> {
    let lines: Vec<PoLineRow> = sqlx::query_as(
        "SELECT line_id, quantity::FLOAT8 AS quantity, unit_price_minor \
         FROM po_lines WHERE po_id = $1",
    )
    .bind(po_id)
    .fetch_all(pool)
    .await?;
    Ok(lines)
}

/// Aggregate received quantities per PO line from po_receipt_links.
pub(super) async fn load_receipt_aggs(
    pool: &PgPool,
    po_line_ids: &[Uuid],
) -> Result<Vec<ReceiptAgg>, MatchError> {
    let mut aggs = Vec::new();
    for &po_line_id in po_line_ids {
        let row: Option<(f64, Uuid)> = sqlx::query_as(
            r#"
            SELECT
                SUM(quantity_received::FLOAT8)     AS total_received,
                MIN(receipt_id::TEXT)::UUID        AS first_receipt_id
            FROM po_receipt_links
            WHERE po_line_id = $1
            GROUP BY po_line_id
            "#,
        )
        .bind(po_line_id)
        .fetch_optional(pool)
        .await?;

        if let Some((total_received, first_receipt_id)) = row {
            aggs.push(ReceiptAgg {
                po_line_id,
                total_received,
                first_receipt_id,
            });
        }
    }
    Ok(aggs)
}

// ============================================================================
// Mutation (writes — within caller-owned transaction)
// ============================================================================

/// Insert a single match record. ON CONFLICT (bill_line_id) DO NOTHING
/// guarantees idempotency on re-runs.
pub(super) async fn insert_match_record(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    bill_id: Uuid,
    po_id: Option<Uuid>,
    line: &MatchLineResult,
    matched_by: &str,
    matched_at: DateTime<Utc>,
) -> Result<(), MatchError> {
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
    .bind(po_id)
    .bind(line.po_line_id)
    .bind(line.receipt_id)
    .bind(&line.match_type)
    .bind(line.matched_quantity)
    .bind(line.matched_amount_minor)
    .bind(line.within_tolerance)
    .bind(line.price_variance_minor)
    .bind(line.qty_variance)
    .bind(&line.match_status)
    .bind(matched_by)
    .bind(matched_at)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Update bill status to 'matched' (idempotent: WHERE status = 'open').
pub(super) async fn update_bill_status_matched(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    bill_id: Uuid,
) -> Result<(), MatchError> {
    sqlx::query(
        "UPDATE vendor_bills SET status = 'matched' \
         WHERE bill_id = $1 AND status = 'open'",
    )
    .bind(bill_id)
    .execute(&mut **tx)
    .await?;
    Ok(())
}
