//! Calibration event domain model and repository.
//!
//! Invariants:
//! - Calibration events are immutable once created (aerospace audit trail)
//! - Every event is tenant-scoped (multi-tenant isolation)
//! - Calibration status is deterministic: derived from latest event + dates + out_of_service
//! - Guard → Mutation → Outbox atomicity on all writes

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

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow, utoipa::ToSchema)]
pub struct CalibrationEvent {
    pub id: Uuid,
    pub tenant_id: String,
    pub asset_id: Uuid,
    pub performed_at: DateTime<Utc>,
    pub due_at: DateTime<Utc>,
    pub result: String,
    pub doc_revision_id: Option<Uuid>,
    pub idempotency_key: Option<String>,
    pub created_at: DateTime<Utc>,
}

// ============================================================================
// Calibration status (derived, not stored)
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum CalibrationStatus {
    InCal,
    Due,
    Overdue,
    OutOfService,
}

impl CalibrationStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::InCal => "in_cal",
            Self::Due => "due",
            Self::Overdue => "overdue",
            Self::OutOfService => "out_of_service",
        }
    }
}

#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct CalibrationStatusResponse {
    pub asset_id: Uuid,
    pub status: CalibrationStatus,
    pub out_of_service: bool,
    pub last_calibrated_at: Option<DateTime<Utc>>,
    pub next_due_at: Option<DateTime<Utc>>,
}

// ============================================================================
// Request types
// ============================================================================

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct RecordCalibrationRequest {
    pub tenant_id: String,
    pub performed_at: DateTime<Utc>,
    pub due_at: DateTime<Utc>,
    pub result: String,
    pub doc_revision_id: Option<Uuid>,
    pub idempotency_key: Option<String>,
}

const VALID_RESULTS: &[&str] = &["pass", "fail", "conditional"];

// ============================================================================
// Errors
// ============================================================================

#[derive(Debug, Error)]
pub enum CalibrationEventError {
    #[error("Asset not found")]
    AssetNotFound,

    #[error("Validation error: {0}")]
    Validation(String),

    #[error("Idempotent duplicate")]
    IdempotentDuplicate(CalibrationEvent),

    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),
}

// ============================================================================
// Repository
// ============================================================================

pub struct CalibrationEventRepo;

impl CalibrationEventRepo {
    /// Record a calibration event using Guard → Mutation → Outbox.
    pub async fn record(
        pool: &PgPool,
        asset_id: Uuid,
        req: &RecordCalibrationRequest,
    ) -> Result<CalibrationEvent, CalibrationEventError> {
        // ── Guards ──
        if req.tenant_id.trim().is_empty() {
            return Err(CalibrationEventError::Validation(
                "tenant_id is required".into(),
            ));
        }
        if !VALID_RESULTS.contains(&req.result.as_str()) {
            return Err(CalibrationEventError::Validation(format!(
                "result must be one of: {}",
                VALID_RESULTS.join(", ")
            )));
        }
        if req.due_at <= req.performed_at {
            return Err(CalibrationEventError::Validation(
                "due_at must be after performed_at".into(),
            ));
        }

        // ── Idempotency check ──
        if let Some(ref ikey) = req.idempotency_key {
            let existing = sqlx::query_as::<_, CalibrationEvent>(
                "SELECT * FROM calibration_events WHERE tenant_id = $1 AND idempotency_key = $2",
            )
            .bind(&req.tenant_id)
            .bind(ikey)
            .fetch_optional(pool)
            .await?;

            if let Some(event) = existing {
                return Err(CalibrationEventError::IdempotentDuplicate(event));
            }
        }

        let mut tx = pool.begin().await?;

        // Verify asset exists for this tenant
        let asset_exists: Option<(Uuid,)> =
            sqlx::query_as("SELECT id FROM maintainable_assets WHERE id = $1 AND tenant_id = $2")
                .bind(asset_id)
                .bind(&req.tenant_id)
                .fetch_optional(&mut *tx)
                .await?;
        if asset_exists.is_none() {
            return Err(CalibrationEventError::AssetNotFound);
        }

        // ── Mutation ──
        let id = Uuid::new_v4();
        let event = sqlx::query_as::<_, CalibrationEvent>(
            r#"
            INSERT INTO calibration_events
                (id, tenant_id, asset_id, performed_at, due_at, result,
                 doc_revision_id, idempotency_key)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
            RETURNING *
            "#,
        )
        .bind(id)
        .bind(&req.tenant_id)
        .bind(asset_id)
        .bind(req.performed_at)
        .bind(req.due_at)
        .bind(&req.result)
        .bind(req.doc_revision_id)
        .bind(req.idempotency_key.as_deref())
        .fetch_one(&mut *tx)
        .await?;

        // ── Outbox: calibration_event_recorded ──
        let event_id = Uuid::new_v4();
        let payload = serde_json::json!({
            "calibration_event_id": id,
            "tenant_id": &req.tenant_id,
            "asset_id": asset_id,
            "performed_at": req.performed_at.to_rfc3339(),
            "due_at": req.due_at.to_rfc3339(),
            "result": &req.result,
        });
        let env = envelope::create_envelope(
            event_id,
            req.tenant_id.clone(),
            subjects::CALIBRATION_EVENT_RECORDED.to_string(),
            payload,
        );
        let env_json = envelope::validate_envelope(&env)
            .map_err(|e| CalibrationEventError::Validation(format!("envelope: {}", e)))?;
        outbox::enqueue_event_tx(
            &mut tx,
            event_id,
            subjects::CALIBRATION_EVENT_RECORDED,
            "calibration_event",
            &id.to_string(),
            &env_json,
        )
        .await?;

        // ── Outbox: calibration_status_changed ──
        let status = compute_status_from_event(&event, false);
        let status_event_id = Uuid::new_v4();
        let status_payload = serde_json::json!({
            "asset_id": asset_id,
            "tenant_id": &req.tenant_id,
            "status": status.as_str(),
            "last_calibrated_at": req.performed_at.to_rfc3339(),
            "next_due_at": req.due_at.to_rfc3339(),
        });
        let status_env = envelope::create_envelope(
            status_event_id,
            req.tenant_id.clone(),
            subjects::CALIBRATION_STATUS_CHANGED.to_string(),
            status_payload,
        );
        let status_env_json = envelope::validate_envelope(&status_env)
            .map_err(|e| CalibrationEventError::Validation(format!("envelope: {}", e)))?;
        outbox::enqueue_event_tx(
            &mut tx,
            status_event_id,
            subjects::CALIBRATION_STATUS_CHANGED,
            "calibration_event",
            &asset_id.to_string(),
            &status_env_json,
        )
        .await?;

        tx.commit().await?;
        Ok(event)
    }

    /// Get calibration status for an asset, derived from latest calibration event.
    pub async fn get_status(
        pool: &PgPool,
        asset_id: Uuid,
        tenant_id: &str,
    ) -> Result<CalibrationStatusResponse, CalibrationEventError> {
        // Verify asset exists and get out_of_service flag
        let asset: Option<(bool,)> = sqlx::query_as(
            "SELECT out_of_service FROM maintainable_assets WHERE id = $1 AND tenant_id = $2",
        )
        .bind(asset_id)
        .bind(tenant_id)
        .fetch_optional(pool)
        .await?;
        let (out_of_service,) = asset.ok_or(CalibrationEventError::AssetNotFound)?;

        if out_of_service {
            // Fetch latest event for informational fields even if out_of_service
            let latest = Self::find_latest(pool, asset_id, tenant_id).await?;
            return Ok(CalibrationStatusResponse {
                asset_id,
                status: CalibrationStatus::OutOfService,
                out_of_service: true,
                last_calibrated_at: latest.as_ref().map(|e| e.performed_at),
                next_due_at: latest.as_ref().map(|e| e.due_at),
            });
        }

        let latest = Self::find_latest(pool, asset_id, tenant_id).await?;
        match latest {
            None => Ok(CalibrationStatusResponse {
                asset_id,
                status: CalibrationStatus::Overdue,
                out_of_service: false,
                last_calibrated_at: None,
                next_due_at: None,
            }),
            Some(event) => {
                let status = compute_status_from_event(&event, false);
                Ok(CalibrationStatusResponse {
                    asset_id,
                    status,
                    out_of_service: false,
                    last_calibrated_at: Some(event.performed_at),
                    next_due_at: Some(event.due_at),
                })
            }
        }
    }

    /// Find the most recent calibration event for an asset.
    pub async fn find_latest(
        pool: &PgPool,
        asset_id: Uuid,
        tenant_id: &str,
    ) -> Result<Option<CalibrationEvent>, CalibrationEventError> {
        sqlx::query_as::<_, CalibrationEvent>(
            r#"
            SELECT * FROM calibration_events
            WHERE tenant_id = $1 AND asset_id = $2
            ORDER BY performed_at DESC
            LIMIT 1
            "#,
        )
        .bind(tenant_id)
        .bind(asset_id)
        .fetch_optional(pool)
        .await
        .map_err(CalibrationEventError::Database)
    }
}

/// Derive calibration status from a single event + out_of_service flag.
fn compute_status_from_event(event: &CalibrationEvent, out_of_service: bool) -> CalibrationStatus {
    if out_of_service {
        return CalibrationStatus::OutOfService;
    }
    let now = Utc::now();
    if event.due_at <= now {
        CalibrationStatus::Overdue
    } else {
        CalibrationStatus::InCal
    }
}
