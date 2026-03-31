//! Meter types and meter readings domain model with rollover validation.
//!
//! Invariants:
//! - meter_type name is unique per tenant
//! - reading_value is monotonically increasing per (tenant, asset, meter_type)
//! - Rollover exception: accepted when prev near rollover_value and new near zero
//! - Validation against highest reading_value, not latest timestamp

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use thiserror::Error;
use uuid::Uuid;

// ============================================================================
// Models
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow, utoipa::ToSchema)]
pub struct MeterType {
    pub id: Uuid,
    pub tenant_id: String,
    pub name: String,
    pub unit_label: String,
    pub rollover_value: Option<i64>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow, utoipa::ToSchema)]
pub struct MeterReading {
    pub id: Uuid,
    pub tenant_id: String,
    pub asset_id: Uuid,
    pub meter_type_id: Uuid,
    pub reading_value: i64,
    pub recorded_at: DateTime<Utc>,
    pub recorded_by: Option<String>,
    pub created_at: DateTime<Utc>,
}

// ============================================================================
// Request types
// ============================================================================

#[derive(Debug, Deserialize)]
pub struct CreateMeterTypeRequest {
    pub tenant_id: String,
    pub name: String,
    pub unit_label: String,
    pub rollover_value: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct RecordReadingRequest {
    pub tenant_id: String,
    pub meter_type_id: Uuid,
    pub reading_value: i64,
    pub recorded_at: Option<DateTime<Utc>>,
    pub recorded_by: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ListReadingsQuery {
    pub meter_type_id: Option<Uuid>,
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

// ============================================================================
// Errors
// ============================================================================

#[derive(Debug, Error)]
pub enum MeterError {
    #[error("Meter type name '{0}' already exists for tenant '{1}'")]
    DuplicateName(String, String),

    #[error("Meter type not found")]
    MeterTypeNotFound,

    #[error("Asset not found")]
    AssetNotFound,

    #[error("Monotonicity violation: reading {attempted} < previous max {previous}")]
    MonotonicityViolation { previous: i64, attempted: i64 },

    #[error("Validation error: {0}")]
    Validation(String),

    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),
}

// ============================================================================
// Rollover validation
// ============================================================================

/// Validate a meter reading against the previous max.
///
/// Rules:
/// 1. First reading (prev_max = None) is always valid.
/// 2. new >= prev → valid (monotonically increasing).
/// 3. new < prev AND rollover_value is set → valid only if:
///    - prev >= rollover_value * 90% (near rollover)
///    - new <= rollover_value * 10% (near zero)
pub fn validate_reading(
    new_value: i64,
    prev_max: Option<i64>,
    rollover_value: Option<i64>,
) -> Result<(), MeterError> {
    let Some(prev) = prev_max else {
        return Ok(());
    };
    if new_value >= prev {
        return Ok(());
    }

    let Some(rollover) = rollover_value else {
        return Err(MeterError::MonotonicityViolation {
            previous: prev,
            attempted: new_value,
        });
    };

    if rollover <= 0 {
        return Err(MeterError::MonotonicityViolation {
            previous: prev,
            attempted: new_value,
        });
    }

    let near_rollover = prev >= (rollover * 9) / 10;
    let near_zero = new_value <= rollover / 10;

    if near_rollover && near_zero {
        Ok(())
    } else {
        Err(MeterError::MonotonicityViolation {
            previous: prev,
            attempted: new_value,
        })
    }
}

// ============================================================================
// Repositories
// ============================================================================

pub struct MeterTypeRepo;

impl MeterTypeRepo {
    pub async fn create(
        pool: &PgPool,
        req: &CreateMeterTypeRequest,
    ) -> Result<MeterType, MeterError> {
        if req.tenant_id.trim().is_empty() {
            return Err(MeterError::Validation("tenant_id is required".into()));
        }
        if req.name.trim().is_empty() {
            return Err(MeterError::Validation("name is required".into()));
        }
        if req.unit_label.trim().is_empty() {
            return Err(MeterError::Validation("unit_label is required".into()));
        }
        if let Some(rv) = req.rollover_value {
            if rv <= 0 {
                return Err(MeterError::Validation(
                    "rollover_value must be positive".into(),
                ));
            }
        }

        sqlx::query_as::<_, MeterType>(
            r#"
            INSERT INTO meter_types (tenant_id, name, unit_label, rollover_value)
            VALUES ($1, $2, $3, $4)
            RETURNING *
            "#,
        )
        .bind(&req.tenant_id)
        .bind(req.name.trim())
        .bind(req.unit_label.trim())
        .bind(req.rollover_value)
        .fetch_one(pool)
        .await
        .map_err(|e| {
            if let sqlx::Error::Database(ref dbe) = e {
                if dbe.code().as_deref() == Some("23505") {
                    return MeterError::DuplicateName(req.name.clone(), req.tenant_id.clone());
                }
            }
            MeterError::Database(e)
        })
    }

    pub async fn list(pool: &PgPool, tenant_id: &str) -> Result<Vec<MeterType>, MeterError> {
        sqlx::query_as::<_, MeterType>(
            r#"
            SELECT * FROM meter_types
            WHERE tenant_id = $1
            ORDER BY name
            "#,
        )
        .bind(tenant_id)
        .fetch_all(pool)
        .await
        .map_err(MeterError::Database)
    }
}

pub struct MeterReadingRepo;

impl MeterReadingRepo {
    /// Record a meter reading with monotonicity + rollover validation.
    ///
    /// 1. Verify asset exists for tenant
    /// 2. Verify meter type exists for tenant
    /// 3. Fetch previous max reading_value
    /// 4. Validate monotonicity (with rollover exception)
    /// 5. Insert reading + enqueue outbox event in same transaction
    pub async fn record(
        pool: &PgPool,
        asset_id: Uuid,
        req: &RecordReadingRequest,
    ) -> Result<MeterReading, MeterError> {
        if req.tenant_id.trim().is_empty() {
            return Err(MeterError::Validation("tenant_id is required".into()));
        }
        if req.reading_value < 0 {
            return Err(MeterError::Validation(
                "reading_value must be non-negative".into(),
            ));
        }

        let mut tx = pool.begin().await?;

        // Verify asset exists for tenant
        let asset_exists: Option<(Uuid,)> =
            sqlx::query_as("SELECT id FROM maintainable_assets WHERE id = $1 AND tenant_id = $2")
                .bind(asset_id)
                .bind(&req.tenant_id)
                .fetch_optional(&mut *tx)
                .await?;

        if asset_exists.is_none() {
            return Err(MeterError::AssetNotFound);
        }

        // Fetch meter type (need rollover_value)
        let meter_type = sqlx::query_as::<_, MeterType>(
            "SELECT * FROM meter_types WHERE id = $1 AND tenant_id = $2",
        )
        .bind(req.meter_type_id)
        .bind(&req.tenant_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(MeterError::MeterTypeNotFound)?;

        // Fetch previous max reading_value for this (tenant, asset, meter_type)
        let prev_max_val: Option<i64> = sqlx::query_scalar(
            r#"
            SELECT MAX(reading_value) FROM meter_readings
            WHERE tenant_id = $1 AND asset_id = $2 AND meter_type_id = $3
            "#,
        )
        .bind(&req.tenant_id)
        .bind(asset_id)
        .bind(req.meter_type_id)
        .fetch_one(&mut *tx)
        .await?;

        validate_reading(req.reading_value, prev_max_val, meter_type.rollover_value)?;

        let recorded_at = req.recorded_at.unwrap_or_else(Utc::now);
        let reading_id = Uuid::new_v4();

        let reading = sqlx::query_as::<_, MeterReading>(
            r#"
            INSERT INTO meter_readings
                (id, tenant_id, asset_id, meter_type_id, reading_value, recorded_at, recorded_by)
            VALUES ($1, $2, $3, $4, $5, $6, $7)
            RETURNING *
            "#,
        )
        .bind(reading_id)
        .bind(&req.tenant_id)
        .bind(asset_id)
        .bind(req.meter_type_id)
        .bind(req.reading_value)
        .bind(recorded_at)
        .bind(req.recorded_by.as_deref())
        .fetch_one(&mut *tx)
        .await?;

        // Enqueue outbox event
        let event_payload = serde_json::json!({
            "asset_id": asset_id,
            "meter_type_id": req.meter_type_id,
            "reading_value": req.reading_value,
            "recorded_at": recorded_at,
        });
        let event_id = Uuid::new_v4();
        let env = crate::events::envelope::create_envelope(
            event_id,
            req.tenant_id.clone(),
            crate::events::subjects::METER_READING_RECORDED.to_string(),
            event_payload,
        );
        let env_json = crate::events::envelope::validate_envelope(&env)
            .map_err(|e| MeterError::Validation(format!("envelope: {}", e)))?;
        crate::outbox::enqueue_event_tx(
            &mut tx,
            event_id,
            crate::events::subjects::METER_READING_RECORDED,
            "meter_reading",
            &reading_id.to_string(),
            &env_json,
        )
        .await?;

        tx.commit().await?;

        Ok(reading)
    }

    pub async fn list(
        pool: &PgPool,
        tenant_id: &str,
        asset_id: Uuid,
        q: &ListReadingsQuery,
    ) -> Result<Vec<MeterReading>, MeterError> {
        let limit = q.limit.unwrap_or(50).clamp(1, 100);
        let offset = q.offset.unwrap_or(0);

        sqlx::query_as::<_, MeterReading>(
            r#"
            SELECT * FROM meter_readings
            WHERE tenant_id = $1 AND asset_id = $2
              AND ($3::UUID IS NULL OR meter_type_id = $3)
            ORDER BY recorded_at DESC
            LIMIT $4 OFFSET $5
            "#,
        )
        .bind(tenant_id)
        .bind(asset_id)
        .bind(q.meter_type_id)
        .bind(limit)
        .bind(offset)
        .fetch_all(pool)
        .await
        .map_err(MeterError::Database)
    }

    pub async fn count(pool: &PgPool, tenant_id: &str, asset_id: Uuid, q: &ListReadingsQuery) -> Result<i64, MeterError> {
        let row: (i64,) = sqlx::query_as(r#"SELECT COUNT(*) FROM meter_readings WHERE tenant_id = $1 AND asset_id = $2 AND ($3::UUID IS NULL OR meter_type_id = $3)"#)
            .bind(tenant_id).bind(asset_id).bind(q.meter_type_id).fetch_one(pool).await.map_err(MeterError::Database)?;
        Ok(row.0)
    }
}

// ============================================================================
// Unit tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_reading_always_valid() {
        assert!(validate_reading(100, None, None).is_ok());
        assert!(validate_reading(0, None, Some(999999)).is_ok());
    }

    #[test]
    fn increasing_reading_valid() {
        assert!(validate_reading(200, Some(100), None).is_ok());
        assert!(validate_reading(100, Some(100), None).is_ok()); // equal is ok
    }

    #[test]
    fn decreasing_without_rollover_rejected() {
        let err = validate_reading(50, Some(100), None).unwrap_err();
        assert!(matches!(err, MeterError::MonotonicityViolation { .. }));
    }

    #[test]
    fn valid_rollover_accepted() {
        // rollover at 1,000,000. prev = 950,000 (within 10% of rollover).
        // new = 12 (within 10% of zero = 100,000).
        assert!(validate_reading(12, Some(950_000), Some(1_000_000)).is_ok());
    }

    #[test]
    fn rollover_prev_not_near_max_rejected() {
        // prev = 500,000 (NOT within 10% of 1,000,000)
        let err = validate_reading(12, Some(500_000), Some(1_000_000)).unwrap_err();
        assert!(matches!(err, MeterError::MonotonicityViolation { .. }));
    }

    #[test]
    fn rollover_new_not_near_zero_rejected() {
        // new = 200,000 (NOT within 10% of zero = 100,000)
        let err = validate_reading(200_000, Some(950_000), Some(1_000_000)).unwrap_err();
        assert!(matches!(err, MeterError::MonotonicityViolation { .. }));
    }

    #[test]
    fn rollover_boundary_exactly_90_percent() {
        // prev exactly at 90% of rollover
        assert!(validate_reading(0, Some(900_000), Some(1_000_000)).is_ok());
    }

    #[test]
    fn rollover_boundary_exactly_10_percent() {
        // new exactly at 10% of rollover
        assert!(validate_reading(100_000, Some(950_000), Some(1_000_000)).is_ok());
    }
}
