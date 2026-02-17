use chrono::{DateTime, Utc};
use sqlx::PgPool;
use uuid::Uuid;

use crate::events::contracts::{
    FxRateUpdatedPayload, EVENT_TYPE_FX_RATE_UPDATED, MUTATION_CLASS_DATA_MUTATION,
};
use crate::repos::fx_rate_repo::{self, FxRate};
use crate::repos::outbox_repo;

#[derive(Debug)]
pub struct CreateFxRateRequest {
    pub tenant_id: String,
    pub base_currency: String,
    pub quote_currency: String,
    pub rate: f64,
    pub effective_at: DateTime<Utc>,
    pub source: String,
    pub idempotency_key: String,
}

#[derive(Debug)]
pub struct CreateFxRateResponse {
    pub rate_id: Uuid,
    pub was_inserted: bool,
}

/// Create a new FX rate and emit fx.rate_updated atomically via the outbox.
///
/// If the idempotency_key already exists, returns the existing rate_id with
/// `was_inserted = false`. No event is emitted for duplicates.
pub async fn create_fx_rate(
    pool: &PgPool,
    req: CreateFxRateRequest,
) -> Result<CreateFxRateResponse, String> {
    // Validate currencies
    if req.base_currency.len() != 3
        || !req.base_currency.chars().all(|c| c.is_ascii_uppercase())
    {
        return Err(format!(
            "Invalid base_currency '{}': must be 3 uppercase ASCII letters",
            req.base_currency
        ));
    }
    if req.quote_currency.len() != 3
        || !req.quote_currency.chars().all(|c| c.is_ascii_uppercase())
    {
        return Err(format!(
            "Invalid quote_currency '{}': must be 3 uppercase ASCII letters",
            req.quote_currency
        ));
    }
    if req.base_currency == req.quote_currency {
        return Err("base_currency and quote_currency must differ".to_string());
    }
    if req.rate <= 0.0 || !req.rate.is_finite() {
        return Err("rate must be a positive finite number".to_string());
    }
    if req.idempotency_key.is_empty() {
        return Err("idempotency_key cannot be empty".to_string());
    }

    let rate_id = Uuid::new_v4();
    let inverse_rate = 1.0 / req.rate;
    let now = Utc::now();

    let fx_rate = FxRate {
        id: rate_id,
        tenant_id: req.tenant_id.clone(),
        base_currency: req.base_currency.clone(),
        quote_currency: req.quote_currency.clone(),
        rate: req.rate,
        inverse_rate,
        effective_at: req.effective_at,
        source: req.source.clone(),
        idempotency_key: req.idempotency_key.clone(),
        created_at: now,
    };

    // Begin transaction for atomicity: insert rate + outbox event
    let mut tx = pool
        .begin()
        .await
        .map_err(|e| format!("Failed to begin transaction: {}", e))?;

    let was_inserted = fx_rate_repo::insert_fx_rate(&mut tx, &fx_rate)
        .await
        .map_err(|e| format!("Failed to insert FX rate: {}", e))?;

    if was_inserted {
        // Emit fx.rate_updated event to outbox (same transaction)
        let event_id = Uuid::new_v4();
        let payload = FxRateUpdatedPayload {
            rate_id,
            base_currency: req.base_currency.clone(),
            quote_currency: req.quote_currency.clone(),
            rate: req.rate,
            inverse_rate,
            effective_at: req.effective_at,
            source: req.source.clone(),
        };
        let payload_json = serde_json::to_value(&payload)
            .map_err(|e| format!("Failed to serialize FX rate payload: {}", e))?;

        outbox_repo::insert_outbox_event(
            &mut tx,
            event_id,
            EVENT_TYPE_FX_RATE_UPDATED,
            "fx_rate",
            &rate_id.to_string(),
            payload_json,
            MUTATION_CLASS_DATA_MUTATION,
        )
        .await
        .map_err(|e| format!("Failed to insert outbox event: {}", e))?;
    }

    tx.commit()
        .await
        .map_err(|e| format!("Failed to commit transaction: {}", e))?;

    Ok(CreateFxRateResponse {
        rate_id,
        was_inserted,
    })
}

/// Get the latest FX rate as-of a given timestamp.
pub async fn get_latest_rate(
    pool: &PgPool,
    tenant_id: &str,
    base_currency: &str,
    quote_currency: &str,
    as_of: DateTime<Utc>,
) -> Result<Option<FxRate>, String> {
    fx_rate_repo::get_latest_rate(pool, tenant_id, base_currency, quote_currency, as_of)
        .await
        .map_err(|e| format!("Failed to query FX rate: {}", e))
}
