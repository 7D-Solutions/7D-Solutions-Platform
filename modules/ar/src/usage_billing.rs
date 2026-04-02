//! Usage-to-Invoice Billing (bd-n9j)
//!
//! Converts unbilled metered usage records into invoice line items.
//!
//! ## Exactly-once guarantee
//!
//! Each usage record can be billed at most once. The `billed_at` column serves as
//! the sentinel; `FOR UPDATE SKIP LOCKED` prevents concurrent bill runs from
//! selecting the same rows.
//!
//! ## Transaction pattern
//!
//! ```text
//! BEGIN
//!   SELECT ar_metered_usage WHERE billed_at IS NULL ... FOR UPDATE SKIP LOCKED
//!   INSERT INTO ar_invoice_line_items (one per usage row)
//!   UPDATE ar_metered_usage SET billed_at = NOW(), invoice_id, line_item_id
//!   INSERT INTO events_outbox (ar.usage_invoiced per row)
//! COMMIT
//! ```
//!
//! ## Idempotency
//!
//! Calling bill_usage_for_invoice when all usage is already billed is a no-op
//! (returns Ok with billed_count = 0).

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use uuid::Uuid;

use crate::events::contracts::{
    build_usage_invoiced_envelope, UsageInvoicedPayload, EVENT_TYPE_USAGE_INVOICED,
};
use crate::events::outbox::enqueue_event_tx;

// ============================================================================
// Request / Response types
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BillUsageRequest {
    pub app_id: String,
    pub invoice_id: i32,
    pub customer_id: i32,
    pub period_start: DateTime<Utc>,
    pub period_end: DateTime<Utc>,
    /// Correlation ID for outbox event tracing
    pub correlation_id: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct BillUsageResult {
    /// Number of usage records that were billed in this call
    pub billed_count: usize,
    /// Total amount billed in minor units (cents)
    pub total_amount_minor: i64,
}

// ============================================================================
// Internal row type
// ============================================================================

#[derive(Debug, sqlx::FromRow)]
struct UnbilledUsageRow {
    pub id: i32,
    pub metric_name: String,
    pub quantity: String, // NUMERIC(10,2) → String (no bigdecimal feature)
    pub unit_price_cents: i32,
    pub period_start: DateTime<Utc>,
    pub period_end: DateTime<Utc>,
    pub usage_uuid: Option<Uuid>,
}

// ============================================================================
// Core function
// ============================================================================

/// Select unbilled usage for the given billing window, create invoice line items,
/// mark usage as billed, and emit `ar.usage_invoiced` events — all in one transaction.
///
/// Returns `BillUsageResult` with the number of records billed and total amount.
/// If no unbilled usage exists for the window, returns `billed_count = 0`.
pub async fn bill_usage_for_invoice(
    db: &PgPool,
    req: BillUsageRequest,
) -> Result<BillUsageResult, sqlx::Error> {
    let mut tx = db.begin().await?;

    // Step 1: Lock and select unbilled usage rows for this customer + billing window.
    // FOR UPDATE SKIP LOCKED ensures concurrent bill runs cannot double-bill.
    let unbilled: Vec<UnbilledUsageRow> = sqlx::query_as::<_, UnbilledUsageRow>(
        r#"
        SELECT id, metric_name, quantity::TEXT AS quantity, unit_price_cents,
               period_start::TIMESTAMPTZ AS period_start,
               period_end::TIMESTAMPTZ AS period_end, usage_uuid
        FROM ar_metered_usage
        WHERE app_id = $1
          AND customer_id = $2
          AND billed_at IS NULL
          AND period_start >= $3
          AND period_end <= $4
        FOR UPDATE SKIP LOCKED
        "#,
    )
    .bind(&req.app_id)
    .bind(req.customer_id)
    .bind(req.period_start)
    .bind(req.period_end)
    .fetch_all(&mut *tx)
    .await?;

    if unbilled.is_empty() {
        tx.rollback().await?;
        return Ok(BillUsageResult {
            billed_count: 0,
            total_amount_minor: 0,
        });
    }

    let mut total_amount_minor: i64 = 0;

    for row in &unbilled {
        let qty: f64 = row.quantity.parse().unwrap_or(0.0);
        let amount_cents = (qty * row.unit_price_cents as f64).round() as i64;
        total_amount_minor += amount_cents as i64;

        // Step 2a: Insert invoice line item
        // Cast quantity string to NUMERIC via SQL
        let line_item_id: i32 = sqlx::query_scalar(
            r#"
            INSERT INTO ar_invoice_line_items (
                app_id, invoice_id, line_item_type, description,
                quantity, unit_price_cents, amount_cents, created_at
            )
            VALUES ($1, $2, 'metered_usage', $3, $4, $5, $6, NOW())
            RETURNING id
            "#,
        )
        .bind(&req.app_id)
        .bind(req.invoice_id)
        .bind(format!(
            "{} — {}/{}",
            row.metric_name,
            row.period_start.format("%Y-%m-%d"),
            row.period_end.format("%Y-%m-%d")
        ))
        .bind(qty)
        .bind(row.unit_price_cents)
        .bind(amount_cents)
        .fetch_one(&mut *tx)
        .await?;

        // Step 2b: Mark usage as billed (atomic with line item creation)
        sqlx::query(
            r#"
            UPDATE ar_metered_usage
            SET billed_at = NOW(),
                invoice_id = $1,
                line_item_id = $2
            WHERE id = $3
              AND billed_at IS NULL
            "#,
        )
        .bind(req.invoice_id)
        .bind(line_item_id)
        .bind(row.id)
        .execute(&mut *tx)
        .await?;

        // Step 2c: Emit ar.usage_invoiced outbox event (one per usage record)
        let event_id = row.usage_uuid.unwrap_or_else(Uuid::new_v4);
        let total_minor = amount_cents as i64;
        let payload = UsageInvoicedPayload {
            usage_id: event_id,
            invoice_id: req.invoice_id.to_string(),
            tenant_id: req.app_id.clone(),
            customer_id: req.customer_id.to_string(),
            metric_name: row.metric_name.clone(),
            quantity: qty,
            unit: "units".to_string(), // unit stored in metric_name context
            unit_price_minor: row.unit_price_cents as i64,
            total_minor,
            currency: "usd".to_string(),
            invoiced_at: Utc::now(),
        };

        let envelope = build_usage_invoiced_envelope(
            Uuid::new_v4(), // unique event_id per emission
            req.app_id.clone(),
            req.correlation_id.clone(),
            None,
            payload,
        );

        enqueue_event_tx(
            &mut tx,
            EVENT_TYPE_USAGE_INVOICED,
            "usage_billing",
            &row.id.to_string(),
            &envelope,
        )
        .await?;
    }

    tx.commit().await?;

    Ok(BillUsageResult {
        billed_count: unbilled.len(),
        total_amount_minor,
    })
}
