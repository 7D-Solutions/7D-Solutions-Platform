//! AP event ingestion for the reporting module.
//!
//! Subscribes to AP domain events and populates `rpt_ap_aging_cache` with
//! aging bucket snapshots per vendor.
//!
//! ## Events consumed
//!
//! - `ap.events.ap.vendor_bill_created`  — add bill to appropriate bucket
//! - `ap.events.ap.vendor_bill_voided`   — subtract voided amount
//! - `ap.events.ap.payment_executed`     — subtract payment amount
//!
//! ## Bucket assignment (bill_created)
//!
//! Relative to `as_of = today`:
//! - **current**: due_date >= as_of
//! - **1-30**: as_of - 30d <= due_date < as_of
//! - **31-60**: as_of - 60d <= due_date < as_of - 30d
//! - **61-90**: as_of - 90d <= due_date < as_of - 60d
//! - **over_90**: due_date < as_of - 90d

use std::sync::Arc;

use async_trait::async_trait;
use chrono::{NaiveDate, Utc};
use serde::Deserialize;
use sqlx::PgPool;

use event_bus::EventBus;

use crate::ingest::{start_consumer, IngestConsumer, StreamHandler};

// ── Constants ────────────────────────────────────────────────────────────────

pub const SUBJECT_BILL_CREATED: &str = "ap.events.ap.vendor_bill_created";
pub const SUBJECT_BILL_VOIDED: &str = "ap.events.ap.vendor_bill_voided";
pub const SUBJECT_PAYMENT_EXECUTED: &str = "ap.events.ap.payment_executed";

pub const CONSUMER_AP_AGING_BILLS: &str = "reporting.ap_aging_bills";
pub const CONSUMER_AP_AGING_VOIDS: &str = "reporting.ap_aging_voids";
pub const CONSUMER_AP_AGING_PAYMENTS: &str = "reporting.ap_aging_payments";

// ── Local payload mirrors ───────────────────────────────────────────────────
//
// Reporting must not depend on the AP crate. Mirror only required fields.

#[derive(Debug, Deserialize)]
struct BillCreatedPayload {
    vendor_id: String,
    total_minor: i64,
    due_date: chrono::DateTime<Utc>,
    currency: String,
}

#[derive(Debug, Deserialize)]
struct BillVoidedPayload {
    vendor_id: String,
    original_total_minor: i64,
    currency: String,
}

#[derive(Debug, Deserialize)]
struct PaymentExecutedPayload {
    vendor_id: String,
    amount_minor: i64,
    currency: String,
}

// ── Bucket computation ──────────────────────────────────────────────────────

enum AgingBucket {
    Current,
    Days1_30,
    Days31_60,
    Days61_90,
    Over90,
}

fn compute_bucket(due_date: NaiveDate, as_of: NaiveDate) -> AgingBucket {
    let days_past_due = (as_of - due_date).num_days();
    if days_past_due <= 0 {
        AgingBucket::Current
    } else if days_past_due <= 30 {
        AgingBucket::Days1_30
    } else if days_past_due <= 60 {
        AgingBucket::Days31_60
    } else if days_past_due <= 90 {
        AgingBucket::Days61_90
    } else {
        AgingBucket::Over90
    }
}

// ── Bill Created Handler ────────────────────────────────────────────────────

pub struct ApBillCreatedHandler;

#[async_trait]
impl StreamHandler for ApBillCreatedHandler {
    async fn handle(
        &self,
        pool: &PgPool,
        tenant_id: &str,
        _event_id: &str,
        payload: &serde_json::Value,
    ) -> Result<(), anyhow::Error> {
        let p: BillCreatedPayload = serde_json::from_value(payload.clone())
            .map_err(|e| anyhow::anyhow!("Failed to parse bill created payload: {}", e))?;

        let as_of = Utc::now().date_naive();
        let due_date = p.due_date.date_naive();

        let (current, b1_30, b31_60, b61_90, over_90) = match compute_bucket(due_date, as_of) {
            AgingBucket::Current => (p.total_minor, 0, 0, 0, 0),
            AgingBucket::Days1_30 => (0, p.total_minor, 0, 0, 0),
            AgingBucket::Days31_60 => (0, 0, p.total_minor, 0, 0),
            AgingBucket::Days61_90 => (0, 0, 0, p.total_minor, 0),
            AgingBucket::Over90 => (0, 0, 0, 0, p.total_minor),
        };

        upsert_aging_add(
            pool,
            tenant_id,
            as_of,
            &p.vendor_id,
            &p.currency,
            current,
            b1_30,
            b31_60,
            b61_90,
            over_90,
            p.total_minor,
        )
        .await
    }
}

// ── Bill Voided Handler ─────────────────────────────────────────────────────

pub struct ApBillVoidedHandler;

#[async_trait]
impl StreamHandler for ApBillVoidedHandler {
    async fn handle(
        &self,
        pool: &PgPool,
        tenant_id: &str,
        _event_id: &str,
        payload: &serde_json::Value,
    ) -> Result<(), anyhow::Error> {
        let p: BillVoidedPayload = serde_json::from_value(payload.clone())
            .map_err(|e| anyhow::anyhow!("Failed to parse bill voided payload: {}", e))?;

        let as_of = Utc::now().date_naive();
        upsert_aging_subtract(
            pool,
            tenant_id,
            as_of,
            &p.vendor_id,
            &p.currency,
            p.original_total_minor,
        )
        .await
    }
}

// ── Payment Executed Handler ────────────────────────────────────────────────

pub struct ApPaymentExecutedHandler;

#[async_trait]
impl StreamHandler for ApPaymentExecutedHandler {
    async fn handle(
        &self,
        pool: &PgPool,
        tenant_id: &str,
        _event_id: &str,
        payload: &serde_json::Value,
    ) -> Result<(), anyhow::Error> {
        let p: PaymentExecutedPayload = serde_json::from_value(payload.clone())
            .map_err(|e| anyhow::anyhow!("Failed to parse payment executed payload: {}", e))?;

        let as_of = Utc::now().date_naive();
        upsert_aging_subtract(
            pool,
            tenant_id,
            as_of,
            &p.vendor_id,
            &p.currency,
            p.amount_minor,
        )
        .await
    }
}

// ── SQL helpers ─────────────────────────────────────────────────────────────

async fn upsert_aging_add(
    pool: &PgPool,
    tenant_id: &str,
    as_of: NaiveDate,
    vendor_id: &str,
    currency: &str,
    current: i64,
    b1_30: i64,
    b31_60: i64,
    b61_90: i64,
    over_90: i64,
    total: i64,
) -> Result<(), anyhow::Error> {
    sqlx::query(
        r#"
        INSERT INTO rpt_ap_aging_cache
            (tenant_id, as_of, vendor_id, currency,
             current_minor, bucket_1_30_minor, bucket_31_60_minor,
             bucket_61_90_minor, bucket_over_90_minor, total_minor,
             computed_at)
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, NOW())
        ON CONFLICT (tenant_id, as_of, vendor_id, currency) DO UPDATE SET
            current_minor        = rpt_ap_aging_cache.current_minor        + EXCLUDED.current_minor,
            bucket_1_30_minor    = rpt_ap_aging_cache.bucket_1_30_minor    + EXCLUDED.bucket_1_30_minor,
            bucket_31_60_minor   = rpt_ap_aging_cache.bucket_31_60_minor   + EXCLUDED.bucket_31_60_minor,
            bucket_61_90_minor   = rpt_ap_aging_cache.bucket_61_90_minor   + EXCLUDED.bucket_61_90_minor,
            bucket_over_90_minor = rpt_ap_aging_cache.bucket_over_90_minor + EXCLUDED.bucket_over_90_minor,
            total_minor          = rpt_ap_aging_cache.total_minor          + EXCLUDED.total_minor,
            computed_at          = NOW()
        "#,
    )
    .bind(tenant_id)
    .bind(as_of)
    .bind(vendor_id)
    .bind(currency)
    .bind(current)
    .bind(b1_30)
    .bind(b31_60)
    .bind(b61_90)
    .bind(over_90)
    .bind(total)
    .execute(pool)
    .await
    .map_err(|e| anyhow::anyhow!("Failed to upsert AP aging cache (add): {}", e))?;
    Ok(())
}

async fn upsert_aging_subtract(
    pool: &PgPool,
    tenant_id: &str,
    as_of: NaiveDate,
    vendor_id: &str,
    currency: &str,
    amount: i64,
) -> Result<(), anyhow::Error> {
    // Reduce amounts (floor at 0 to respect CHECK >= 0 constraints).
    // Best-effort bucket distribution: subtract from current first.
    let result = sqlx::query(
        r#"
        UPDATE rpt_ap_aging_cache SET
            current_minor        = GREATEST(0, current_minor - LEAST(current_minor, $5)),
            total_minor          = GREATEST(0, total_minor - $5),
            computed_at          = NOW()
        WHERE tenant_id = $1
          AND as_of     = $2
          AND vendor_id = $3
          AND currency  = $4
        "#,
    )
    .bind(tenant_id)
    .bind(as_of)
    .bind(vendor_id)
    .bind(currency)
    .bind(amount)
    .execute(pool)
    .await
    .map_err(|e| anyhow::anyhow!("Failed to update AP aging cache (subtract): {}", e))?;

    if result.rows_affected() == 0 {
        tracing::debug!(
            tenant_id,
            vendor_id,
            currency,
            "No AP aging row for today to subtract from; skipping"
        );
    }
    Ok(())
}

// ── Consumer registration ────────────────────────────────────────────────────

/// Register all AP aging ingestion consumers.
pub fn register_consumers(pool: PgPool, bus: Arc<dyn EventBus>) {
    let bill_handler = Arc::new(ApBillCreatedHandler);
    let bill_consumer = IngestConsumer::new(CONSUMER_AP_AGING_BILLS, pool.clone(), bill_handler);
    start_consumer(bill_consumer, bus.clone(), SUBJECT_BILL_CREATED);

    let void_handler = Arc::new(ApBillVoidedHandler);
    let void_consumer = IngestConsumer::new(CONSUMER_AP_AGING_VOIDS, pool.clone(), void_handler);
    start_consumer(void_consumer, bus.clone(), SUBJECT_BILL_VOIDED);

    let pay_handler = Arc::new(ApPaymentExecutedHandler);
    let pay_consumer = IngestConsumer::new(CONSUMER_AP_AGING_PAYMENTS, pool, pay_handler);
    start_consumer(pay_consumer, bus, SUBJECT_PAYMENT_EXECUTED);
}

#[cfg(test)]
mod tests;
