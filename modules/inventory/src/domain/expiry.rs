//! Lot expiry assignment and alert scanning.

use chrono::{Duration, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use thiserror::Error;
use utoipa::ToSchema;
use uuid::Uuid;

use crate::domain::expiry_repo;
use crate::events::{
    build_expiry_alert_envelope, build_expiry_set_envelope, ExpiryAlertPayload, ExpirySetPayload,
    EVENT_TYPE_EXPIRY_ALERT, EVENT_TYPE_EXPIRY_SET,
};

// ============================================================================
// Domain model (defined in expiry_repo; re-exported here for API compatibility)
// ============================================================================

pub use crate::domain::expiry_repo::LotExpiryRecord;

// ============================================================================
// Request / result types
// ============================================================================

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct SetLotExpiryRequest {
    pub tenant_id: String,
    pub lot_id: Uuid,
    #[serde(default)]
    pub expires_on: Option<NaiveDate>,
    #[serde(default)]
    pub compute_from_policy: bool,
    #[serde(default)]
    pub reference_at: Option<chrono::DateTime<Utc>>,
    pub idempotency_key: String,
    pub correlation_id: Option<String>,
    pub causation_id: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct RunExpiryAlertScanRequest {
    pub tenant_id: String,
    #[serde(default)]
    pub as_of_date: Option<NaiveDate>,
    pub expiring_within_days: i32,
    pub idempotency_key: String,
    pub correlation_id: Option<String>,
    pub causation_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct RunExpiryAlertScanResult {
    pub tenant_id: String,
    pub as_of_date: NaiveDate,
    pub expiring_within_days: i32,
    pub expiring_soon_emitted: usize,
    pub expired_emitted: usize,
}

#[derive(Debug, Error)]
pub enum ExpiryError {
    #[error("Lot not found")]
    LotNotFound,
    #[error("Expiry date is required when compute_from_policy is false")]
    ExpiryDateRequired,
    #[error("No effective revision with shelf_life_days found for expiry computation")]
    MissingShelfLifePolicy,
    #[error("Validation error: {0}")]
    Validation(String),
    #[error("Idempotency key conflict: same key used with a different request")]
    ConflictingIdempotencyKey,
    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),
    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),
}

// ============================================================================
// Policy helpers
// ============================================================================

pub async fn compute_expiry_from_policy(
    pool: &PgPool,
    tenant_id: &str,
    item_id: Uuid,
    reference_at: chrono::DateTime<Utc>,
) -> Result<Option<NaiveDate>, ExpiryError> {
    let shelf_life_days =
        expiry_repo::fetch_shelf_life_days(pool, tenant_id, item_id, reference_at).await?;

    Ok(shelf_life_days
        .map(|days| reference_at.date_naive() + Duration::days(days as i64)))
}

// ============================================================================
// Set lot expiry service
// ============================================================================

pub async fn set_lot_expiry(
    pool: &PgPool,
    req: &SetLotExpiryRequest,
) -> Result<(LotExpiryRecord, bool), ExpiryError> {
    validate_set_request(req)?;

    let request_hash = serde_json::to_string(req)?;
    if let Some(record) =
        expiry_repo::find_idempotency_key(pool, &req.tenant_id, &req.idempotency_key).await?
    {
        if record.request_hash != request_hash {
            return Err(ExpiryError::ConflictingIdempotencyKey);
        }
        let result: LotExpiryRecord = serde_json::from_str(&record.response_body)?;
        return Ok((result, true));
    }

    let now = Utc::now();
    let correlation_id = req
        .correlation_id
        .clone()
        .unwrap_or_else(|| Uuid::new_v4().to_string());

    let mut tx = pool.begin().await?;

    let lot = expiry_repo::lock_lot(&mut tx, req.lot_id, &req.tenant_id)
        .await?
        .ok_or(ExpiryError::LotNotFound)?;

    let (expires_on, expiry_source) = if req.compute_from_policy {
        let reference_at = req.reference_at.unwrap_or(lot.created_at);
        let computed =
            compute_expiry_from_policy(pool, &req.tenant_id, lot.item_id, reference_at).await?;
        (
            computed.ok_or(ExpiryError::MissingShelfLifePolicy)?,
            "policy",
        )
    } else {
        (
            req.expires_on.ok_or(ExpiryError::ExpiryDateRequired)?,
            "manual",
        )
    };

    let updated = expiry_repo::update_lot_expiry(
        &mut tx,
        expires_on,
        expiry_source,
        now,
        req.lot_id,
        &req.tenant_id,
    )
    .await?;

    let event_id = Uuid::new_v4();
    let payload = ExpirySetPayload {
        lot_id: updated.lot_id,
        tenant_id: updated.tenant_id.clone(),
        item_id: updated.item_id,
        lot_code: updated.lot_code.clone(),
        expiry_date: updated.expires_on,
        source: updated.expiry_source.clone(),
        set_at: updated.expiry_set_at,
    };
    let envelope = build_expiry_set_envelope(
        event_id,
        req.tenant_id.clone(),
        correlation_id.clone(),
        req.causation_id.clone(),
        payload,
    );
    let envelope_json = serde_json::to_string(&envelope)?;

    expiry_repo::insert_lot_outbox_event(
        &mut tx,
        event_id,
        EVENT_TYPE_EXPIRY_SET,
        &updated.lot_id.to_string(),
        &req.tenant_id,
        &envelope_json,
        &correlation_id,
        req.causation_id.as_deref(),
    )
    .await?;

    let response_json = serde_json::to_string(&updated)?;
    let expires_at = now + Duration::days(7);
    expiry_repo::store_idempotency_key(
        &mut tx,
        &req.tenant_id,
        &req.idempotency_key,
        &request_hash,
        &response_json,
        200,
        expires_at,
    )
    .await?;

    tx.commit().await?;
    Ok((updated, false))
}

// ============================================================================
// Expiry alert scan service
// ============================================================================

pub async fn run_expiry_alert_scan(
    pool: &PgPool,
    req: &RunExpiryAlertScanRequest,
) -> Result<(RunExpiryAlertScanResult, bool), ExpiryError> {
    validate_scan_request(req)?;

    let request_hash = serde_json::to_string(req)?;
    if let Some(record) =
        expiry_repo::find_idempotency_key(pool, &req.tenant_id, &req.idempotency_key).await?
    {
        if record.request_hash != request_hash {
            return Err(ExpiryError::ConflictingIdempotencyKey);
        }
        let result: RunExpiryAlertScanResult = serde_json::from_str(&record.response_body)?;
        return Ok((result, true));
    }

    let as_of_date = req.as_of_date.unwrap_or_else(|| Utc::now().date_naive());
    let correlation_id = req
        .correlation_id
        .clone()
        .unwrap_or_else(|| Uuid::new_v4().to_string());

    let expiring =
        expiry_repo::fetch_expiring_lots(pool, &req.tenant_id, as_of_date, req.expiring_within_days)
            .await?;

    let expired = expiry_repo::fetch_expired_lots(pool, &req.tenant_id, as_of_date).await?;

    let mut expiring_soon_emitted = 0usize;
    for lot in expiring {
        if emit_alert_if_new(
            pool,
            &req.tenant_id,
            &correlation_id,
            req.causation_id.clone(),
            &lot,
            "expiring_soon",
            as_of_date,
            req.expiring_within_days,
        )
        .await?
        {
            expiring_soon_emitted += 1;
        }
    }

    let mut expired_emitted = 0usize;
    for lot in expired {
        if emit_alert_if_new(
            pool,
            &req.tenant_id,
            &correlation_id,
            req.causation_id.clone(),
            &lot,
            "expired",
            as_of_date,
            0,
        )
        .await?
        {
            expired_emitted += 1;
        }
    }

    let result = RunExpiryAlertScanResult {
        tenant_id: req.tenant_id.clone(),
        as_of_date,
        expiring_within_days: req.expiring_within_days,
        expiring_soon_emitted,
        expired_emitted,
    };

    let now = Utc::now();
    let response_json = serde_json::to_string(&result)?;
    let expires_at = now + Duration::days(7);
    expiry_repo::store_idempotency_key_pool(
        pool,
        &req.tenant_id,
        &req.idempotency_key,
        &request_hash,
        &response_json,
        200,
        expires_at,
    )
    .await?;

    Ok((result, false))
}

// ============================================================================
// Alert emission helper
// ============================================================================

async fn emit_alert_if_new(
    pool: &PgPool,
    tenant_id: &str,
    correlation_id: &str,
    causation_id: Option<String>,
    lot: &expiry_repo::LotCandidate,
    alert_type: &str,
    alert_date: NaiveDate,
    window_days: i32,
) -> Result<bool, ExpiryError> {
    let mut tx = pool.begin().await?;

    let marker = expiry_repo::insert_alert_state_if_new(
        &mut tx,
        tenant_id,
        lot.id,
        alert_type,
        alert_date,
        window_days,
    )
    .await?;

    if marker.is_none() {
        tx.commit().await?;
        return Ok(false);
    }

    let event_id = Uuid::new_v4();
    let payload = ExpiryAlertPayload {
        lot_id: lot.id,
        tenant_id: tenant_id.to_string(),
        item_id: lot.item_id,
        lot_code: lot.lot_code.clone(),
        expiry_date: lot.expires_on,
        alert_kind: alert_type.to_string(),
        window_start: alert_date,
        window_end: alert_date + Duration::days(window_days as i64),
        emitted_at: Utc::now(),
    };
    let envelope = build_expiry_alert_envelope(
        event_id,
        tenant_id.to_string(),
        correlation_id.to_string(),
        causation_id.clone(),
        payload,
    );
    let envelope_json = serde_json::to_string(&envelope)?;

    expiry_repo::insert_alert_outbox_event(
        &mut tx,
        event_id,
        EVENT_TYPE_EXPIRY_ALERT,
        &lot.id.to_string(),
        tenant_id,
        &envelope_json,
        correlation_id,
        causation_id.as_deref(),
    )
    .await?;

    tx.commit().await?;
    Ok(true)
}

// ============================================================================
// Validation
// ============================================================================

fn validate_set_request(req: &SetLotExpiryRequest) -> Result<(), ExpiryError> {
    if req.tenant_id.trim().is_empty() {
        return Err(ExpiryError::Validation("tenant_id is required".to_string()));
    }
    if req.idempotency_key.trim().is_empty() {
        return Err(ExpiryError::Validation(
            "idempotency_key is required".to_string(),
        ));
    }
    if !req.compute_from_policy && req.expires_on.is_none() {
        return Err(ExpiryError::ExpiryDateRequired);
    }
    Ok(())
}

fn validate_scan_request(req: &RunExpiryAlertScanRequest) -> Result<(), ExpiryError> {
    if req.tenant_id.trim().is_empty() {
        return Err(ExpiryError::Validation("tenant_id is required".to_string()));
    }
    if req.idempotency_key.trim().is_empty() {
        return Err(ExpiryError::Validation(
            "idempotency_key is required".to_string(),
        ));
    }
    if req.expiring_within_days < 0 {
        return Err(ExpiryError::Validation(
            "expiring_within_days must be >= 0".to_string(),
        ));
    }
    Ok(())
}
