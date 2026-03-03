//! Lot expiry assignment and alert scanning.

use chrono::{DateTime, Duration, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use thiserror::Error;
use uuid::Uuid;

use crate::events::{
    build_expiry_alert_envelope, build_expiry_set_envelope, ExpiryAlertPayload, ExpirySetPayload,
    EVENT_TYPE_EXPIRY_ALERT, EVENT_TYPE_EXPIRY_SET,
};

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct LotExpiryRecord {
    pub lot_id: Uuid,
    pub tenant_id: String,
    pub item_id: Uuid,
    pub lot_code: String,
    pub expires_on: NaiveDate,
    pub expiry_source: String,
    pub expiry_set_at: DateTime<Utc>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SetLotExpiryRequest {
    pub tenant_id: String,
    pub lot_id: Uuid,
    #[serde(default)]
    pub expires_on: Option<NaiveDate>,
    #[serde(default)]
    pub compute_from_policy: bool,
    #[serde(default)]
    pub reference_at: Option<DateTime<Utc>>,
    pub idempotency_key: String,
    pub correlation_id: Option<String>,
    pub causation_id: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct RunExpiryAlertScanRequest {
    pub tenant_id: String,
    #[serde(default)]
    pub as_of_date: Option<NaiveDate>,
    pub expiring_within_days: i32,
    pub idempotency_key: String,
    pub correlation_id: Option<String>,
    pub causation_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
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

#[derive(sqlx::FromRow)]
struct IdempotencyRecord {
    response_body: String,
    request_hash: String,
}

#[derive(sqlx::FromRow)]
struct LotRow {
    item_id: Uuid,
    created_at: DateTime<Utc>,
}

#[derive(sqlx::FromRow)]
struct LotCandidate {
    id: Uuid,
    item_id: Uuid,
    lot_code: String,
    expires_on: NaiveDate,
}

pub async fn compute_expiry_from_policy(
    pool: &PgPool,
    tenant_id: &str,
    item_id: Uuid,
    reference_at: DateTime<Utc>,
) -> Result<Option<NaiveDate>, ExpiryError> {
    let shelf_life_days: Option<i32> = sqlx::query_scalar(
        r#"
        SELECT shelf_life_days
        FROM item_revisions
        WHERE tenant_id = $1 AND item_id = $2
          AND effective_from IS NOT NULL
          AND effective_from <= $3
          AND (effective_to IS NULL OR effective_to > $3)
        ORDER BY effective_from DESC
        LIMIT 1
        "#,
    )
    .bind(tenant_id)
    .bind(item_id)
    .bind(reference_at)
    .fetch_optional(pool)
    .await?
    .flatten();

    Ok(shelf_life_days.map(|days| reference_at.date_naive() + Duration::days(days as i64)))
}

pub async fn set_lot_expiry(
    pool: &PgPool,
    req: &SetLotExpiryRequest,
) -> Result<(LotExpiryRecord, bool), ExpiryError> {
    validate_set_request(req)?;

    let request_hash = serde_json::to_string(req)?;
    if let Some(record) = find_idempotency_key(pool, &req.tenant_id, &req.idempotency_key).await? {
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
    let lot = sqlx::query_as::<_, LotRow>(
        r#"
        SELECT item_id, created_at
        FROM inventory_lots
        WHERE id = $1 AND tenant_id = $2
        FOR UPDATE
        "#,
    )
    .bind(req.lot_id)
    .bind(&req.tenant_id)
    .fetch_optional(&mut *tx)
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

    let updated = sqlx::query_as::<_, LotExpiryRecord>(
        r#"
        UPDATE inventory_lots
        SET expires_on = $1,
            expiry_source = $2,
            expiry_set_at = $3
        WHERE id = $4 AND tenant_id = $5
        RETURNING
            id AS lot_id,
            tenant_id,
            item_id,
            lot_code,
            expires_on,
            COALESCE(expiry_source, '') AS expiry_source,
            COALESCE(expiry_set_at, NOW()) AS expiry_set_at
        "#,
    )
    .bind(expires_on)
    .bind(expiry_source)
    .bind(now)
    .bind(req.lot_id)
    .bind(&req.tenant_id)
    .fetch_one(&mut *tx)
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

    sqlx::query(
        r#"
        INSERT INTO inv_outbox
            (event_id, event_type, aggregate_type, aggregate_id, tenant_id,
             payload, correlation_id, causation_id, schema_version)
        VALUES ($1, $2, 'inventory_lot', $3, $4, $5::JSONB, $6, $7, '1.0.0')
        "#,
    )
    .bind(event_id)
    .bind(EVENT_TYPE_EXPIRY_SET)
    .bind(updated.lot_id.to_string())
    .bind(&req.tenant_id)
    .bind(&envelope_json)
    .bind(&correlation_id)
    .bind(&req.causation_id)
    .execute(&mut *tx)
    .await?;

    let response_json = serde_json::to_string(&updated)?;
    let expires_at = now + Duration::days(7);
    sqlx::query(
        r#"
        INSERT INTO inv_idempotency_keys
            (tenant_id, idempotency_key, request_hash, response_body, status_code, expires_at)
        VALUES ($1, $2, $3, $4::JSONB, 200, $5)
        "#,
    )
    .bind(&req.tenant_id)
    .bind(&req.idempotency_key)
    .bind(&request_hash)
    .bind(&response_json)
    .bind(expires_at)
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;
    Ok((updated, false))
}

pub async fn run_expiry_alert_scan(
    pool: &PgPool,
    req: &RunExpiryAlertScanRequest,
) -> Result<(RunExpiryAlertScanResult, bool), ExpiryError> {
    validate_scan_request(req)?;

    let request_hash = serde_json::to_string(req)?;
    if let Some(record) = find_idempotency_key(pool, &req.tenant_id, &req.idempotency_key).await? {
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

    let expiring = sqlx::query_as::<_, LotCandidate>(
        r#"
        SELECT id, item_id, lot_code, expires_on
        FROM inventory_lots
        WHERE tenant_id = $1
          AND expires_on IS NOT NULL
          AND expires_on > $2
          AND expires_on <= $3
        "#,
    )
    .bind(&req.tenant_id)
    .bind(as_of_date)
    .bind(as_of_date + Duration::days(req.expiring_within_days as i64))
    .fetch_all(pool)
    .await?;

    let expired = sqlx::query_as::<_, LotCandidate>(
        r#"
        SELECT id, item_id, lot_code, expires_on
        FROM inventory_lots
        WHERE tenant_id = $1
          AND expires_on IS NOT NULL
          AND expires_on <= $2
        "#,
    )
    .bind(&req.tenant_id)
    .bind(as_of_date)
    .fetch_all(pool)
    .await?;

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
    sqlx::query(
        r#"
        INSERT INTO inv_idempotency_keys
            (tenant_id, idempotency_key, request_hash, response_body, status_code, expires_at)
        VALUES ($1, $2, $3, $4::JSONB, 200, $5)
        "#,
    )
    .bind(&req.tenant_id)
    .bind(&req.idempotency_key)
    .bind(&request_hash)
    .bind(&response_json)
    .bind(expires_at)
    .execute(pool)
    .await?;

    Ok((result, false))
}

async fn emit_alert_if_new(
    pool: &PgPool,
    tenant_id: &str,
    correlation_id: &str,
    causation_id: Option<String>,
    lot: &LotCandidate,
    alert_type: &str,
    alert_date: NaiveDate,
    window_days: i32,
) -> Result<bool, ExpiryError> {
    let mut tx = pool.begin().await?;

    let marker: Option<Uuid> = sqlx::query_scalar(
        r#"
        INSERT INTO inv_lot_expiry_alert_state
            (tenant_id, lot_id, alert_type, alert_date, window_days)
        VALUES ($1, $2, $3, $4, $5)
        ON CONFLICT (tenant_id, lot_id, alert_type, alert_date, window_days)
        DO NOTHING
        RETURNING id
        "#,
    )
    .bind(tenant_id)
    .bind(lot.id)
    .bind(alert_type)
    .bind(alert_date)
    .bind(window_days)
    .fetch_optional(&mut *tx)
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

    sqlx::query(
        r#"
        INSERT INTO inv_outbox
            (event_id, event_type, aggregate_type, aggregate_id, tenant_id,
             payload, correlation_id, causation_id, schema_version)
        VALUES ($1, $2, 'inventory_lot', $3, $4, $5::JSONB, $6, $7, '1.0.0')
        "#,
    )
    .bind(event_id)
    .bind(EVENT_TYPE_EXPIRY_ALERT)
    .bind(lot.id.to_string())
    .bind(tenant_id)
    .bind(&envelope_json)
    .bind(correlation_id)
    .bind(causation_id)
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;
    Ok(true)
}

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

async fn find_idempotency_key(
    pool: &PgPool,
    tenant_id: &str,
    idempotency_key: &str,
) -> Result<Option<IdempotencyRecord>, sqlx::Error> {
    sqlx::query_as::<_, IdempotencyRecord>(
        r#"
        SELECT response_body::TEXT AS response_body, request_hash
        FROM inv_idempotency_keys
        WHERE tenant_id = $1 AND idempotency_key = $2
        "#,
    )
    .bind(tenant_id)
    .bind(idempotency_key)
    .fetch_optional(pool)
    .await
}
