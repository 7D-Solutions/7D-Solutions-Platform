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
pub struct TimeEntry {
    pub time_entry_id: Uuid,
    pub tenant_id: String,
    pub work_order_id: Uuid,
    pub operation_id: Option<Uuid>,
    pub actor_id: String,
    pub start_ts: DateTime<Utc>,
    pub end_ts: Option<DateTime<Utc>>,
    pub minutes: Option<i32>,
    pub notes: Option<String>,
    pub created_at: DateTime<Utc>,
}

// ============================================================================
// Request types
// ============================================================================

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct StartTimerRequest {
    pub work_order_id: Uuid,
    pub operation_id: Option<Uuid>,
    pub actor_id: String,
    pub notes: Option<String>,
    pub idempotency_key: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct StopTimerRequest {
    pub end_ts: Option<DateTime<Utc>>,
}

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct ManualEntryRequest {
    pub work_order_id: Uuid,
    pub operation_id: Option<Uuid>,
    pub actor_id: String,
    pub start_ts: DateTime<Utc>,
    pub end_ts: DateTime<Utc>,
    pub minutes: i32,
    pub notes: Option<String>,
    pub idempotency_key: Option<String>,
}

// ============================================================================
// Errors
// ============================================================================

#[derive(Debug, Error)]
pub enum TimeEntryError {
    #[error("Time entry not found")]
    NotFound,

    #[error("Work order not found")]
    WorkOrderNotFound,

    #[error("Operation not found or does not belong to work order")]
    OperationNotFound,

    #[error("Timer already stopped")]
    AlreadyStopped,

    #[error("Invalid time range: end must be after start")]
    InvalidTimeRange,

    #[error("Conflicting idempotency key")]
    ConflictingIdempotencyKey,

    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),
}

// ============================================================================
// Repository
// ============================================================================

pub struct TimeEntryRepo;

impl TimeEntryRepo {
    /// Start a timer: creates a time entry with start_ts = now, no end_ts.
    pub async fn start_timer(
        pool: &PgPool,
        req: &StartTimerRequest,
        tenant_id: &str,
        correlation_id: &str,
        causation_id: Option<&str>,
    ) -> Result<TimeEntry, TimeEntryError> {
        let request_hash = serde_json::to_string(req)
            .map_err(|e| TimeEntryError::Database(sqlx::Error::Protocol(e.to_string())))?;

        let mut tx = pool.begin().await?;

        // Idempotency check
        if let Some(key) = &req.idempotency_key {
            match check_idempotency(&mut tx, tenant_id, key, &request_hash).await {
                Ok(Some(cached)) => {
                    let entry: TimeEntry = serde_json::from_str(&cached).map_err(|e| {
                        TimeEntryError::Database(sqlx::Error::Protocol(e.to_string()))
                    })?;
                    tx.commit().await?;
                    return Ok(entry);
                }
                Ok(None) => {}
                Err(IdempotencyError::Conflict) => {
                    return Err(TimeEntryError::ConflictingIdempotencyKey);
                }
                Err(IdempotencyError::Database(e)) => return Err(TimeEntryError::Database(e)),
                Err(IdempotencyError::Json(e)) => {
                    return Err(TimeEntryError::Database(sqlx::Error::Protocol(
                        e.to_string(),
                    )));
                }
            }
        }

        // Verify work order exists and belongs to tenant
        let wo_exists: Option<(Uuid,)> = sqlx::query_as(
            "SELECT work_order_id FROM work_orders WHERE work_order_id = $1 AND tenant_id = $2",
        )
        .bind(req.work_order_id)
        .bind(tenant_id)
        .fetch_optional(&mut *tx)
        .await?;

        if wo_exists.is_none() {
            return Err(TimeEntryError::WorkOrderNotFound);
        }

        // If operation_id provided, verify it belongs to the work order
        if let Some(op_id) = req.operation_id {
            let op_exists: Option<(Uuid,)> = sqlx::query_as(
                "SELECT operation_id FROM operations WHERE operation_id = $1 AND work_order_id = $2",
            )
            .bind(op_id)
            .bind(req.work_order_id)
            .fetch_optional(&mut *tx)
            .await?;

            if op_exists.is_none() {
                return Err(TimeEntryError::OperationNotFound);
            }
        }

        let entry = sqlx::query_as::<_, TimeEntry>(
            r#"
            INSERT INTO time_entries (tenant_id, work_order_id, operation_id, actor_id, start_ts, notes)
            VALUES ($1, $2, $3, $4, NOW(), $5)
            RETURNING *
            "#,
        )
        .bind(tenant_id)
        .bind(req.work_order_id)
        .bind(req.operation_id)
        .bind(&req.actor_id)
        .bind(&req.notes)
        .fetch_one(&mut *tx)
        .await?;

        enqueue_event(
            &mut tx,
            tenant_id,
            ProductionEventType::TimeEntryCreated,
            "time_entry",
            &entry.time_entry_id.to_string(),
            &events::build_time_entry_created_envelope(
                entry.time_entry_id,
                req.work_order_id,
                req.operation_id,
                tenant_id.to_string(),
                req.actor_id.clone(),
                entry.start_ts,
                None,
                None,
                correlation_id.to_string(),
                causation_id.map(String::from),
            ),
            correlation_id,
            causation_id,
        )
        .await?;

        // Store idempotency key
        if let Some(key) = &req.idempotency_key {
            let resp = serde_json::to_string(&entry)
                .map_err(|e| TimeEntryError::Database(sqlx::Error::Protocol(e.to_string())))?;
            store_idempotency_key(
                &mut tx,
                tenant_id,
                key,
                &request_hash,
                &resp,
                201,
                Utc::now() + Duration::hours(24),
            )
            .await?;
        }

        tx.commit().await?;
        Ok(entry)
    }

    /// Stop a running timer: sets end_ts and computes minutes.
    pub async fn stop_timer(
        pool: &PgPool,
        time_entry_id: Uuid,
        req: &StopTimerRequest,
        tenant_id: &str,
        correlation_id: &str,
        causation_id: Option<&str>,
    ) -> Result<TimeEntry, TimeEntryError> {
        let mut tx = pool.begin().await?;

        let entry = sqlx::query_as::<_, TimeEntry>(
            "SELECT * FROM time_entries WHERE time_entry_id = $1 AND tenant_id = $2 FOR UPDATE",
        )
        .bind(time_entry_id)
        .bind(tenant_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(TimeEntryError::NotFound)?;

        if entry.end_ts.is_some() {
            return Err(TimeEntryError::AlreadyStopped);
        }

        let end = req.end_ts.unwrap_or_else(Utc::now);
        if end <= entry.start_ts {
            return Err(TimeEntryError::InvalidTimeRange);
        }

        let duration_mins = (end - entry.start_ts).num_minutes() as i32;

        let updated = sqlx::query_as::<_, TimeEntry>(
            r#"
            UPDATE time_entries
            SET end_ts = $1, minutes = $2
            WHERE time_entry_id = $3
            RETURNING *
            "#,
        )
        .bind(end)
        .bind(duration_mins)
        .bind(time_entry_id)
        .fetch_one(&mut *tx)
        .await?;

        enqueue_event(
            &mut tx,
            tenant_id,
            ProductionEventType::TimeEntryStopped,
            "time_entry",
            &time_entry_id.to_string(),
            &events::build_time_entry_stopped_envelope(
                time_entry_id,
                entry.work_order_id,
                entry.operation_id,
                tenant_id.to_string(),
                entry.actor_id.clone(),
                entry.start_ts,
                end,
                duration_mins,
                correlation_id.to_string(),
                causation_id.map(String::from),
            ),
            correlation_id,
            causation_id,
        )
        .await?;

        tx.commit().await?;
        Ok(updated)
    }

    /// Manual entry: create a completed time entry with explicit start/end/minutes.
    pub async fn manual_entry(
        pool: &PgPool,
        req: &ManualEntryRequest,
        tenant_id: &str,
        correlation_id: &str,
        causation_id: Option<&str>,
    ) -> Result<TimeEntry, TimeEntryError> {
        if req.end_ts <= req.start_ts {
            return Err(TimeEntryError::InvalidTimeRange);
        }

        let request_hash = serde_json::to_string(req)
            .map_err(|e| TimeEntryError::Database(sqlx::Error::Protocol(e.to_string())))?;

        let mut tx = pool.begin().await?;

        // Idempotency check
        if let Some(key) = &req.idempotency_key {
            match check_idempotency(&mut tx, tenant_id, key, &request_hash).await {
                Ok(Some(cached)) => {
                    let entry: TimeEntry = serde_json::from_str(&cached).map_err(|e| {
                        TimeEntryError::Database(sqlx::Error::Protocol(e.to_string()))
                    })?;
                    tx.commit().await?;
                    return Ok(entry);
                }
                Ok(None) => {}
                Err(IdempotencyError::Conflict) => {
                    return Err(TimeEntryError::ConflictingIdempotencyKey);
                }
                Err(IdempotencyError::Database(e)) => return Err(TimeEntryError::Database(e)),
                Err(IdempotencyError::Json(e)) => {
                    return Err(TimeEntryError::Database(sqlx::Error::Protocol(
                        e.to_string(),
                    )));
                }
            }
        }

        // Verify work order
        let wo_exists: Option<(Uuid,)> = sqlx::query_as(
            "SELECT work_order_id FROM work_orders WHERE work_order_id = $1 AND tenant_id = $2",
        )
        .bind(req.work_order_id)
        .bind(tenant_id)
        .fetch_optional(&mut *tx)
        .await?;

        if wo_exists.is_none() {
            return Err(TimeEntryError::WorkOrderNotFound);
        }

        // Verify operation if provided
        if let Some(op_id) = req.operation_id {
            let op_exists: Option<(Uuid,)> = sqlx::query_as(
                "SELECT operation_id FROM operations WHERE operation_id = $1 AND work_order_id = $2",
            )
            .bind(op_id)
            .bind(req.work_order_id)
            .fetch_optional(&mut *tx)
            .await?;

            if op_exists.is_none() {
                return Err(TimeEntryError::OperationNotFound);
            }
        }

        let entry = sqlx::query_as::<_, TimeEntry>(
            r#"
            INSERT INTO time_entries (tenant_id, work_order_id, operation_id, actor_id, start_ts, end_ts, minutes, notes)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
            RETURNING *
            "#,
        )
        .bind(tenant_id)
        .bind(req.work_order_id)
        .bind(req.operation_id)
        .bind(&req.actor_id)
        .bind(req.start_ts)
        .bind(req.end_ts)
        .bind(req.minutes)
        .bind(&req.notes)
        .fetch_one(&mut *tx)
        .await?;

        enqueue_event(
            &mut tx,
            tenant_id,
            ProductionEventType::TimeEntryCreated,
            "time_entry",
            &entry.time_entry_id.to_string(),
            &events::build_time_entry_created_envelope(
                entry.time_entry_id,
                req.work_order_id,
                req.operation_id,
                tenant_id.to_string(),
                req.actor_id.clone(),
                req.start_ts,
                Some(req.end_ts),
                Some(req.minutes),
                correlation_id.to_string(),
                causation_id.map(String::from),
            ),
            correlation_id,
            causation_id,
        )
        .await?;

        // Store idempotency key
        if let Some(key) = &req.idempotency_key {
            let resp = serde_json::to_string(&entry)
                .map_err(|e| TimeEntryError::Database(sqlx::Error::Protocol(e.to_string())))?;
            store_idempotency_key(
                &mut tx,
                tenant_id,
                key,
                &request_hash,
                &resp,
                201,
                Utc::now() + Duration::hours(24),
            )
            .await?;
        }

        tx.commit().await?;
        Ok(entry)
    }

    /// List time entries for a work order.
    pub async fn list_by_work_order(
        pool: &PgPool,
        work_order_id: Uuid,
        tenant_id: &str,
    ) -> Result<Vec<TimeEntry>, TimeEntryError> {
        sqlx::query_as::<_, TimeEntry>(
            "SELECT * FROM time_entries WHERE work_order_id = $1 AND tenant_id = $2 ORDER BY start_ts",
        )
        .bind(work_order_id)
        .bind(tenant_id)
        .fetch_all(pool)
        .await
        .map_err(TimeEntryError::Database)
    }
}
