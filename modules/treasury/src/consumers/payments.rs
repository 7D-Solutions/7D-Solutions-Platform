//! Treasury consumer for Payments execution and settlement events.
//!
//! Subscribes to:
//! - `payments.events.payment.succeeded` — AR collection succeeded (money IN, +amount)
//! - `ap.events.ap.payment_executed`    — AP vendor payment executed (money OUT, -amount)
//!
//! For each event, a normalized `treasury_bank_transactions` row is written
//! against the tenant's first active bank account.
//!
//! ## Idempotency
//! Two layers:
//! 1. `processed_events` table guard (pre-flight check + atomic insert)
//! 2. `treasury_bank_transactions(account_id, external_id)` unique constraint
//!
//! Both operations run inside a single DB transaction, so replay or concurrent
//! delivery of the same event is always safe.
//!
//! ## No Bank Account Configured
//! If a tenant has no active bank account, the event is still recorded in
//! `processed_events` with a warning. This prevents infinite retry spam while
//! the operator configures an account.

use chrono::{DateTime, Utc};
use event_bus::{BusMessage, EventBus, EventEnvelope};
use futures::StreamExt;
use sqlx::PgPool;
use std::sync::Arc;
use uuid::Uuid;

use crate::domain::txns::{
    models::InsertBankTxnRequest,
    service::{
        default_account_id, insert_bank_txn_tx, is_event_processed, record_processed_event_tx,
    },
};

// ============================================================================
// Local payload mirrors (anti-corruption layer)
// ============================================================================

/// Mirror of payments::models::PaymentSucceededPayload
#[derive(Debug, Clone, serde::Deserialize)]
pub struct PaymentSucceededPayload {
    pub payment_id: String,
    pub invoice_id: String,
    pub amount_minor: i32,
    pub currency: String,
}

/// Mirror of ap::events::payment::ApPaymentExecutedPayload
#[derive(Debug, Clone, serde::Deserialize)]
pub struct ApPaymentExecutedPayload {
    pub payment_id: Uuid,
    pub run_id: Uuid,
    pub tenant_id: String,
    pub vendor_id: Uuid,
    pub amount_minor: i64,
    pub currency: String,
    pub payment_method: String,
    pub bank_reference: Option<String>,
    pub executed_at: DateTime<Utc>,
}

// ============================================================================
// Core business logic (testable without NATS)
// ============================================================================

/// Process a `payment.succeeded` event: insert a credit bank transaction.
///
/// Returns `Ok(true)` if a new row was written, `Ok(false)` if duplicate.
pub async fn handle_payment_succeeded(
    pool: &PgPool,
    event_id: Uuid,
    tenant_id: &str,
    payload: &PaymentSucceededPayload,
) -> Result<bool, Box<dyn std::error::Error + Send + Sync>> {
    if is_event_processed(pool, event_id).await? {
        tracing::debug!(
            event_id = %event_id,
            "treasury: payment.succeeded already processed — skipping"
        );
        return Ok(false);
    }

    let account_id = default_account_id(pool, tenant_id).await?;

    let mut tx = pool.begin().await?;

    record_processed_event_tx(
        &mut tx,
        event_id,
        "payment.succeeded",
        "treasury:payments-consumer",
    )
    .await?;

    let inserted = match account_id {
        None => {
            tracing::warn!(
                tenant_id = %tenant_id,
                event_id = %event_id,
                "treasury: no active bank account for tenant — payment.succeeded recorded but not ingested as txn"
            );
            false
        }
        Some(acct_id) => {
            let req = InsertBankTxnRequest {
                app_id: tenant_id.to_string(),
                account_id: acct_id,
                amount_minor: payload.amount_minor as i64,
                currency: payload.currency.clone(),
                transaction_date: Utc::now().date_naive(),
                description: Some(format!("Payment received — invoice {}", payload.invoice_id)),
                reference: Some(payload.payment_id.clone()),
                external_id: event_id.to_string(),
                auth_date: None,
                settle_date: None,
                merchant_name: None,
                merchant_category_code: None,
            };
            insert_bank_txn_tx(&mut tx, &req).await?
        }
    };

    tx.commit().await?;

    if inserted {
        tracing::info!(
            event_id = %event_id,
            tenant_id = %tenant_id,
            amount_minor = payload.amount_minor,
            currency = %payload.currency,
            "treasury: bank transaction created from payment.succeeded"
        );
    }

    Ok(inserted)
}

/// Process an `ap.payment_executed` event: insert a debit bank transaction.
///
/// Returns `Ok(true)` if a new row was written, `Ok(false)` if duplicate.
pub async fn handle_ap_payment_executed(
    pool: &PgPool,
    event_id: Uuid,
    tenant_id: &str,
    payload: &ApPaymentExecutedPayload,
) -> Result<bool, Box<dyn std::error::Error + Send + Sync>> {
    if is_event_processed(pool, event_id).await? {
        tracing::debug!(
            event_id = %event_id,
            "treasury: ap.payment_executed already processed — skipping"
        );
        return Ok(false);
    }

    let account_id = default_account_id(pool, tenant_id).await?;

    let mut tx = pool.begin().await?;

    record_processed_event_tx(
        &mut tx,
        event_id,
        "ap.payment_executed",
        "treasury:payments-consumer",
    )
    .await?;

    let inserted = match account_id {
        None => {
            tracing::warn!(
                tenant_id = %tenant_id,
                event_id = %event_id,
                "treasury: no active bank account for tenant — ap.payment_executed recorded but not ingested as txn"
            );
            false
        }
        Some(acct_id) => {
            let req = InsertBankTxnRequest {
                app_id: tenant_id.to_string(),
                account_id: acct_id,
                // Negative: money leaving the account
                amount_minor: -(payload.amount_minor),
                currency: payload.currency.clone(),
                transaction_date: payload.executed_at.date_naive(),
                description: Some(format!(
                    "AP payment — vendor {} via {}",
                    payload.vendor_id, payload.payment_method
                )),
                reference: payload
                    .bank_reference
                    .clone()
                    .or_else(|| Some(payload.payment_id.to_string())),
                external_id: event_id.to_string(),
                auth_date: None,
                settle_date: None,
                merchant_name: None,
                merchant_category_code: None,
            };
            insert_bank_txn_tx(&mut tx, &req).await?
        }
    };

    tx.commit().await?;

    if inserted {
        tracing::info!(
            event_id = %event_id,
            tenant_id = %tenant_id,
            amount_minor = payload.amount_minor,
            currency = %payload.currency,
            "treasury: bank transaction created from ap.payment_executed"
        );
    }

    Ok(inserted)
}

// ============================================================================
// NATS consumer tasks (production entry points)
// ============================================================================

/// Start the treasury consumer for `payments.events.payment.succeeded`.
pub fn start_payment_succeeded_consumer(bus: Arc<dyn EventBus>, pool: PgPool) {
    tokio::spawn(async move {
        let subject = "payments.events.payment.succeeded";
        tracing::info!(subject, "treasury: starting payment.succeeded consumer");

        let mut stream = match bus.subscribe(subject).await {
            Ok(s) => s,
            Err(e) => {
                tracing::error!(subject, error = %e, "treasury: failed to subscribe");
                return;
            }
        };

        while let Some(msg) = stream.next().await {
            if let Err(e) = process_payment_succeeded_msg(&pool, &msg).await {
                tracing::error!(error = %e, "treasury: failed to process payment.succeeded");
            }
        }

        tracing::warn!(subject, "treasury: payment.succeeded consumer stopped");
    });
}

/// Start the treasury consumer for `ap.events.ap.payment_executed`.
pub fn start_ap_payment_executed_consumer(bus: Arc<dyn EventBus>, pool: PgPool) {
    tokio::spawn(async move {
        let subject = "ap.events.ap.payment_executed";
        tracing::info!(subject, "treasury: starting ap.payment_executed consumer");

        let mut stream = match bus.subscribe(subject).await {
            Ok(s) => s,
            Err(e) => {
                tracing::error!(subject, error = %e, "treasury: failed to subscribe");
                return;
            }
        };

        while let Some(msg) = stream.next().await {
            if let Err(e) = process_ap_payment_executed_msg(&pool, &msg).await {
                tracing::error!(error = %e, "treasury: failed to process ap.payment_executed");
            }
        }

        tracing::warn!(subject, "treasury: ap.payment_executed consumer stopped");
    });
}

// ============================================================================
// Internal message processing
// ============================================================================

async fn process_payment_succeeded_msg(
    pool: &PgPool,
    msg: &BusMessage,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let envelope: EventEnvelope<PaymentSucceededPayload> = serde_json::from_slice(&msg.payload)
        .map_err(|e| format!("parse payment.succeeded envelope: {e}"))?;

    tracing::info!(
        event_id = %envelope.event_id,
        tenant_id = %envelope.tenant_id,
        "treasury: processing payment.succeeded"
    );

    handle_payment_succeeded(
        pool,
        envelope.event_id,
        &envelope.tenant_id,
        &envelope.payload,
    )
    .await?;
    Ok(())
}

async fn process_ap_payment_executed_msg(
    pool: &PgPool,
    msg: &BusMessage,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let envelope: EventEnvelope<ApPaymentExecutedPayload> = serde_json::from_slice(&msg.payload)
        .map_err(|e| format!("parse ap.payment_executed envelope: {e}"))?;

    tracing::info!(
        event_id = %envelope.event_id,
        tenant_id = %envelope.tenant_id,
        "treasury: processing ap.payment_executed"
    );

    handle_ap_payment_executed(
        pool,
        envelope.event_id,
        &envelope.tenant_id,
        &envelope.payload,
    )
    .await?;
    Ok(())
}

#[cfg(test)]
#[path = "payments_tests.rs"]
mod tests;
