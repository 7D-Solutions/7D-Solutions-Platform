//! Downtime event domain model and repository.
//!
//! Invariants:
//! - Downtime records are immutable once created (no UPDATE path)
//! - tenant_id, asset_id, start_time, reason, impact_classification are required
//! - Every query filters by tenant_id for multi-tenant isolation
//! - All mutations use Guard → Mutation → Outbox atomicity
//! - idempotency_key prevents duplicate downtime creation per tenant
//! - end_time, if present, must be after start_time

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use thiserror::Error;
use uuid::Uuid;

use crate::events::{envelope, subjects};
use crate::outbox;

// ============================================================================
// Domain model
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct DowntimeEvent {
    pub id: Uuid,
    pub tenant_id: String,
    pub asset_id: Uuid,
    pub start_time: DateTime<Utc>,
    pub end_time: Option<DateTime<Utc>>,
    pub reason: String,
    pub impact_classification: String,
    pub idempotency_key: Option<String>,
    pub notes: Option<String>,
    pub created_at: DateTime<Utc>,
}

// ============================================================================
// Impact classification enum
// ============================================================================

const VALID_IMPACTS: &[&str] = &["none", "minor", "major", "critical"];

fn validate_impact(s: &str) -> Result<(), DowntimeError> {
    if VALID_IMPACTS.contains(&s) {
        Ok(())
    } else {
        Err(DowntimeError::Validation(format!(
            "invalid impact_classification '{}'; must be one of: {}",
            s,
            VALID_IMPACTS.join(", ")
        )))
    }
}

// ============================================================================
// Request types
// ============================================================================

#[derive(Debug, Deserialize)]
pub struct CreateDowntimeRequest {
    pub tenant_id: String,
    pub asset_id: Uuid,
    pub start_time: DateTime<Utc>,
    pub end_time: Option<DateTime<Utc>>,
    pub reason: String,
    pub impact_classification: String,
    pub idempotency_key: Option<String>,
    pub notes: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ListDowntimeQuery {
    pub tenant_id: String,
    pub asset_id: Option<Uuid>,
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

// ============================================================================
// Errors
// ============================================================================

#[derive(Debug, Error)]
pub enum DowntimeError {
    #[error("Downtime event not found")]
    NotFound,

    #[error("Asset not found")]
    AssetNotFound,

    #[error("Validation error: {0}")]
    Validation(String),

    #[error("Idempotent duplicate — returning existing downtime event")]
    IdempotentDuplicate(DowntimeEvent),

    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),
}

// ============================================================================
// Repository
// ============================================================================

pub struct DowntimeRepo;

impl DowntimeRepo {
    /// Create a downtime event using Guard → Mutation → Outbox.
    pub async fn create(
        pool: &PgPool,
        req: &CreateDowntimeRequest,
    ) -> Result<DowntimeEvent, DowntimeError> {
        // ── Guard ──
        if req.tenant_id.trim().is_empty() {
            return Err(DowntimeError::Validation("tenant_id is required".into()));
        }
        if req.reason.trim().is_empty() {
            return Err(DowntimeError::Validation("reason is required".into()));
        }
        validate_impact(&req.impact_classification)?;

        if let Some(ref end) = req.end_time {
            if *end <= req.start_time {
                return Err(DowntimeError::Validation(
                    "end_time must be after start_time".into(),
                ));
            }
        }

        // ── Idempotency check ──
        if let Some(ref ikey) = req.idempotency_key {
            let existing = sqlx::query_as::<_, DowntimeEvent>(
                "SELECT * FROM downtime_events WHERE tenant_id = $1 AND idempotency_key = $2",
            )
            .bind(&req.tenant_id)
            .bind(ikey)
            .fetch_optional(pool)
            .await?;

            if let Some(event) = existing {
                return Err(DowntimeError::IdempotentDuplicate(event));
            }
        }

        let mut tx = pool.begin().await?;

        // Verify asset exists and belongs to tenant
        let asset_exists: Option<(Uuid,)> =
            sqlx::query_as("SELECT id FROM maintainable_assets WHERE id = $1 AND tenant_id = $2")
                .bind(req.asset_id)
                .bind(&req.tenant_id)
                .fetch_optional(&mut *tx)
                .await?;
        if asset_exists.is_none() {
            return Err(DowntimeError::AssetNotFound);
        }

        // ── Mutation ──
        let id = Uuid::new_v4();

        let event = sqlx::query_as::<_, DowntimeEvent>(
            r#"
            INSERT INTO downtime_events
                (id, tenant_id, asset_id, start_time, end_time, reason,
                 impact_classification, idempotency_key, notes)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
            RETURNING *
            "#,
        )
        .bind(id)
        .bind(&req.tenant_id)
        .bind(req.asset_id)
        .bind(req.start_time)
        .bind(req.end_time)
        .bind(req.reason.trim())
        .bind(&req.impact_classification)
        .bind(req.idempotency_key.as_deref())
        .bind(req.notes.as_deref())
        .fetch_one(&mut *tx)
        .await?;

        // ── Outbox ──
        let event_payload = serde_json::json!({
            "downtime_event_id": id,
            "tenant_id": &req.tenant_id,
            "asset_id": req.asset_id,
            "start_time": req.start_time,
            "end_time": req.end_time,
            "reason": req.reason.trim(),
            "impact_classification": &req.impact_classification,
        });
        let event_id = Uuid::new_v4();
        let env = envelope::create_envelope(
            event_id,
            req.tenant_id.clone(),
            subjects::DOWNTIME_RECORDED.to_string(),
            event_payload,
        );
        let env_json = envelope::validate_envelope(&env)
            .map_err(|e| DowntimeError::Validation(format!("envelope: {}", e)))?;
        outbox::enqueue_event_tx(
            &mut tx,
            event_id,
            subjects::DOWNTIME_RECORDED,
            "downtime_event",
            &id.to_string(),
            &env_json,
        )
        .await?;

        tx.commit().await?;
        Ok(event)
    }

    pub async fn find_by_id(
        pool: &PgPool,
        id: Uuid,
        tenant_id: &str,
    ) -> Result<Option<DowntimeEvent>, DowntimeError> {
        sqlx::query_as::<_, DowntimeEvent>(
            "SELECT * FROM downtime_events WHERE id = $1 AND tenant_id = $2",
        )
        .bind(id)
        .bind(tenant_id)
        .fetch_optional(pool)
        .await
        .map_err(DowntimeError::Database)
    }

    /// List downtime events for a tenant, optionally filtered by asset.
    /// Results ordered by start_time DESC (most recent first).
    pub async fn list(
        pool: &PgPool,
        q: &ListDowntimeQuery,
    ) -> Result<Vec<DowntimeEvent>, DowntimeError> {
        if q.tenant_id.trim().is_empty() {
            return Err(DowntimeError::Validation("tenant_id is required".into()));
        }
        let limit = q.limit.unwrap_or(50).clamp(1, 100);
        let offset = q.offset.unwrap_or(0);

        sqlx::query_as::<_, DowntimeEvent>(
            r#"
            SELECT * FROM downtime_events
            WHERE tenant_id = $1
              AND ($2::UUID IS NULL OR asset_id = $2)
            ORDER BY start_time DESC
            LIMIT $3 OFFSET $4
            "#,
        )
        .bind(&q.tenant_id)
        .bind(q.asset_id)
        .bind(limit)
        .bind(offset)
        .fetch_all(pool)
        .await
        .map_err(DowntimeError::Database)
    }

    /// List downtime events for a specific asset, ordered by start_time DESC.
    pub async fn list_for_asset(
        pool: &PgPool,
        asset_id: Uuid,
        tenant_id: &str,
    ) -> Result<Vec<DowntimeEvent>, DowntimeError> {
        sqlx::query_as::<_, DowntimeEvent>(
            r#"
            SELECT * FROM downtime_events
            WHERE tenant_id = $1 AND asset_id = $2
            ORDER BY start_time DESC
            "#,
        )
        .bind(tenant_id)
        .bind(asset_id)
        .fetch_all(pool)
        .await
        .map_err(DowntimeError::Database)
    }
}
