use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use thiserror::Error;
use utoipa::ToSchema;
use uuid::Uuid;

use crate::domain::idempotency::{check_idempotency, store_idempotency_key, IdempotencyError};
use crate::domain::outbox::enqueue_event;
use crate::events::{self, ProductionEventType};

// ============================================================================
// Model
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow, ToSchema)]
pub struct WorkcenterDowntime {
    pub downtime_id: Uuid,
    pub tenant_id: String,
    pub workcenter_id: Uuid,
    pub reason: String,
    pub reason_code: Option<String>,
    pub started_at: DateTime<Utc>,
    pub ended_at: Option<DateTime<Utc>>,
    pub started_by: Option<String>,
    pub ended_by: Option<String>,
    pub notes: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

// ============================================================================
// Request types
// ============================================================================

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct StartDowntimeRequest {
    pub tenant_id: String,
    pub workcenter_id: Uuid,
    pub reason: String,
    pub reason_code: Option<String>,
    pub started_by: Option<String>,
    pub notes: Option<String>,
    pub idempotency_key: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct EndDowntimeRequest {
    pub tenant_id: String,
    pub ended_by: Option<String>,
}

// ============================================================================
// Errors
// ============================================================================

#[derive(Debug, Error)]
pub enum DowntimeError {
    #[error("Downtime record not found")]
    NotFound,

    #[error("Workcenter not found")]
    WorkcenterNotFound,

    #[error("Downtime already ended")]
    AlreadyEnded,

    #[error("Validation error: {0}")]
    Validation(String),

    #[error("Conflicting idempotency key")]
    ConflictingIdempotencyKey,

    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),
}

// ============================================================================
// Repository
// ============================================================================

pub struct DowntimeRepo;

impl DowntimeRepo {
    pub async fn start(
        pool: &PgPool,
        req: &StartDowntimeRequest,
        correlation_id: &str,
        causation_id: Option<&str>,
    ) -> Result<WorkcenterDowntime, DowntimeError> {
        if req.tenant_id.trim().is_empty() {
            return Err(DowntimeError::Validation("tenant_id is required".into()));
        }
        if req.reason.trim().is_empty() {
            return Err(DowntimeError::Validation("reason is required".into()));
        }

        let request_hash = serde_json::to_string(req)
            .map_err(|e| DowntimeError::Database(sqlx::Error::Protocol(e.to_string())))?;

        let mut tx = pool.begin().await?;

        // Idempotency check
        if let Some(key) = &req.idempotency_key {
            match check_idempotency(&mut tx, &req.tenant_id, key, &request_hash).await {
                Ok(Some(cached)) => {
                    let dt: WorkcenterDowntime = serde_json::from_str(&cached).map_err(|e| {
                        DowntimeError::Database(sqlx::Error::Protocol(e.to_string()))
                    })?;
                    tx.commit().await?;
                    return Ok(dt);
                }
                Ok(None) => {}
                Err(IdempotencyError::Conflict) => {
                    return Err(DowntimeError::ConflictingIdempotencyKey);
                }
                Err(IdempotencyError::Database(e)) => return Err(DowntimeError::Database(e)),
                Err(IdempotencyError::Json(e)) => {
                    return Err(DowntimeError::Database(sqlx::Error::Protocol(
                        e.to_string(),
                    )));
                }
            }
        }

        // Verify workcenter exists and belongs to tenant
        let wc_exists: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM workcenters WHERE workcenter_id = $1 AND tenant_id = $2)",
        )
        .bind(req.workcenter_id)
        .bind(&req.tenant_id)
        .fetch_one(&mut *tx)
        .await?;

        if !wc_exists {
            return Err(DowntimeError::WorkcenterNotFound);
        }

        let dt = sqlx::query_as::<_, WorkcenterDowntime>(
            r#"
            INSERT INTO workcenter_downtime
                (tenant_id, workcenter_id, reason, reason_code, started_by, notes)
            VALUES ($1, $2, $3, $4, $5, $6)
            RETURNING *
            "#,
        )
        .bind(&req.tenant_id)
        .bind(req.workcenter_id)
        .bind(req.reason.trim())
        .bind(req.reason_code.as_deref())
        .bind(req.started_by.as_deref())
        .bind(req.notes.as_deref())
        .fetch_one(&mut *tx)
        .await?;

        enqueue_event(
            &mut tx,
            &req.tenant_id,
            ProductionEventType::DowntimeStarted,
            "workcenter_downtime",
            &dt.downtime_id.to_string(),
            &events::build_downtime_started_envelope(
                dt.downtime_id,
                req.tenant_id.clone(),
                req.workcenter_id,
                req.reason.clone(),
                req.reason_code.clone(),
                dt.started_at,
                req.started_by.clone(),
                correlation_id.to_string(),
                causation_id.map(String::from),
            ),
            correlation_id,
            causation_id,
        )
        .await?;

        // Store idempotency key
        if let Some(key) = &req.idempotency_key {
            let resp = serde_json::to_string(&dt)
                .map_err(|e| DowntimeError::Database(sqlx::Error::Protocol(e.to_string())))?;
            store_idempotency_key(
                &mut tx,
                &req.tenant_id,
                key,
                &request_hash,
                &resp,
                201,
                Utc::now() + Duration::hours(24),
            )
            .await?;
        }

        tx.commit().await?;
        Ok(dt)
    }

    pub async fn end(
        pool: &PgPool,
        downtime_id: Uuid,
        req: &EndDowntimeRequest,
        correlation_id: &str,
        causation_id: Option<&str>,
    ) -> Result<WorkcenterDowntime, DowntimeError> {
        if req.tenant_id.trim().is_empty() {
            return Err(DowntimeError::Validation("tenant_id is required".into()));
        }

        let mut tx = pool.begin().await?;

        let existing = sqlx::query_as::<_, WorkcenterDowntime>(
            "SELECT * FROM workcenter_downtime WHERE downtime_id = $1 AND tenant_id = $2",
        )
        .bind(downtime_id)
        .bind(&req.tenant_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(DowntimeError::NotFound)?;

        if existing.ended_at.is_some() {
            return Err(DowntimeError::AlreadyEnded);
        }

        let dt = sqlx::query_as::<_, WorkcenterDowntime>(
            r#"
            UPDATE workcenter_downtime
            SET ended_at = NOW(), ended_by = $3, updated_at = NOW()
            WHERE downtime_id = $1 AND tenant_id = $2
            RETURNING *
            "#,
        )
        .bind(downtime_id)
        .bind(&req.tenant_id)
        .bind(req.ended_by.as_deref())
        .fetch_one(&mut *tx)
        .await?;

        enqueue_event(
            &mut tx,
            &req.tenant_id,
            ProductionEventType::DowntimeEnded,
            "workcenter_downtime",
            &dt.downtime_id.to_string(),
            &events::build_downtime_ended_envelope(
                dt.downtime_id,
                req.tenant_id.clone(),
                existing.workcenter_id,
                existing.started_at,
                dt.ended_at.expect("just set"),
                req.ended_by.clone(),
                correlation_id.to_string(),
                causation_id.map(String::from),
            ),
            correlation_id,
            causation_id,
        )
        .await?;

        tx.commit().await?;
        Ok(dt)
    }

    pub async fn find_by_id(
        pool: &PgPool,
        id: Uuid,
        tenant_id: &str,
    ) -> Result<Option<WorkcenterDowntime>, DowntimeError> {
        sqlx::query_as::<_, WorkcenterDowntime>(
            "SELECT * FROM workcenter_downtime WHERE downtime_id = $1 AND tenant_id = $2",
        )
        .bind(id)
        .bind(tenant_id)
        .fetch_optional(pool)
        .await
        .map_err(DowntimeError::Database)
    }

    pub async fn list_active(
        pool: &PgPool,
        tenant_id: &str,
        page: i64,
        page_size: i64,
    ) -> Result<(Vec<WorkcenterDowntime>, i64), DowntimeError> {
        let limit = page_size.clamp(1, 200);
        let offset = (page.max(1) - 1) * limit;

        let items = sqlx::query_as::<_, WorkcenterDowntime>(
            r#"
            SELECT * FROM workcenter_downtime
            WHERE tenant_id = $1 AND ended_at IS NULL
            ORDER BY started_at DESC
            LIMIT $2 OFFSET $3
            "#,
        )
        .bind(tenant_id)
        .bind(limit)
        .bind(offset)
        .fetch_all(pool)
        .await
        .map_err(DowntimeError::Database)?;

        let (total,): (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM workcenter_downtime WHERE tenant_id = $1 AND ended_at IS NULL",
        )
        .bind(tenant_id)
        .fetch_one(pool)
        .await
        .map_err(DowntimeError::Database)?;

        Ok((items, total))
    }

    pub async fn list_for_workcenter(
        pool: &PgPool,
        workcenter_id: Uuid,
        tenant_id: &str,
    ) -> Result<Vec<WorkcenterDowntime>, DowntimeError> {
        sqlx::query_as::<_, WorkcenterDowntime>(
            r#"
            SELECT * FROM workcenter_downtime
            WHERE tenant_id = $1 AND workcenter_id = $2
            ORDER BY started_at DESC
            "#,
        )
        .bind(tenant_id)
        .bind(workcenter_id)
        .fetch_all(pool)
        .await
        .map_err(DowntimeError::Database)
    }
}
