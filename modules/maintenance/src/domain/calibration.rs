//! Calibration record domain model and repository.
//!
//! Invariants:
//! - Every calibration record is scoped to a tenant (multi-tenant isolation)
//! - idempotency_key is unique per tenant (prevents duplicate creation)
//! - Completed calibration records are immutable (aerospace audit requirement)
//! - All mutations follow Guard → Mutation → Outbox atomicity

use chrono::{DateTime, NaiveDate, Utc};
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
pub struct CalibrationRecord {
    pub id: Uuid,
    pub tenant_id: String,
    pub asset_id: Uuid,
    pub calibration_type: String,
    pub due_date: NaiveDate,
    pub completed_date: Option<DateTime<Utc>>,
    pub certificate_ref: Option<String>,
    pub status: String,
    pub idempotency_key: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

// ============================================================================
// Request types
// ============================================================================

#[derive(Debug, Deserialize)]
pub struct CreateCalibrationRequest {
    pub tenant_id: String,
    pub asset_id: Uuid,
    pub calibration_type: String,
    pub due_date: NaiveDate,
    pub idempotency_key: String,
}

#[derive(Debug, Deserialize)]
pub struct CompleteCalibrationRequest {
    pub tenant_id: String,
    pub certificate_ref: String,
    pub completed_date: Option<DateTime<Utc>>,
}

// ============================================================================
// Errors
// ============================================================================

#[derive(Debug, Error)]
pub enum CalibrationError {
    #[error("Calibration record not found")]
    NotFound,

    #[error("Asset not found")]
    AssetNotFound,

    #[error("Validation error: {0}")]
    Validation(String),

    #[error("Calibration already completed — immutable")]
    AlreadyCompleted,

    #[error("Duplicate idempotency key")]
    DuplicateKey,

    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),
}

// ============================================================================
// Overdue query result
// ============================================================================

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct OverdueCalibration {
    pub id: Uuid,
    pub tenant_id: String,
    pub asset_id: Uuid,
    pub calibration_type: String,
    pub due_date: NaiveDate,
    pub days_overdue: i32,
}

// ============================================================================
// Repository
// ============================================================================

pub struct CalibrationRepo;

impl CalibrationRepo {
    /// Create a calibration record with Guard → Mutation → Outbox atomicity.
    ///
    /// Idempotent: if the same (tenant_id, idempotency_key) already exists,
    /// returns the existing record without creating a duplicate.
    pub async fn create(
        pool: &PgPool,
        req: &CreateCalibrationRequest,
    ) -> Result<CalibrationRecord, CalibrationError> {
        // ── Guards ──
        if req.tenant_id.trim().is_empty() {
            return Err(CalibrationError::Validation("tenant_id is required".into()));
        }
        if req.calibration_type.trim().is_empty() {
            return Err(CalibrationError::Validation(
                "calibration_type is required".into(),
            ));
        }
        if req.idempotency_key.trim().is_empty() {
            return Err(CalibrationError::Validation(
                "idempotency_key is required".into(),
            ));
        }

        let mut tx = pool.begin().await?;

        // Verify asset exists for this tenant
        let asset_exists: Option<(Uuid,)> =
            sqlx::query_as("SELECT id FROM maintainable_assets WHERE id = $1 AND tenant_id = $2")
                .bind(req.asset_id)
                .bind(&req.tenant_id)
                .fetch_optional(&mut *tx)
                .await?;
        if asset_exists.is_none() {
            return Err(CalibrationError::AssetNotFound);
        }

        // Check for existing record with same idempotency key
        let existing = sqlx::query_as::<_, CalibrationRecord>(
            "SELECT * FROM calibration_records WHERE tenant_id = $1 AND idempotency_key = $2",
        )
        .bind(&req.tenant_id)
        .bind(&req.idempotency_key)
        .fetch_optional(&mut *tx)
        .await?;

        if let Some(record) = existing {
            // Idempotent: return existing record
            tx.commit().await?;
            return Ok(record);
        }

        // ── Mutation ──
        let id = Uuid::new_v4();
        let record = sqlx::query_as::<_, CalibrationRecord>(
            r#"
            INSERT INTO calibration_records
                (id, tenant_id, asset_id, calibration_type, due_date, idempotency_key)
            VALUES ($1, $2, $3, $4, $5, $6)
            RETURNING *
            "#,
        )
        .bind(id)
        .bind(&req.tenant_id)
        .bind(req.asset_id)
        .bind(req.calibration_type.trim())
        .bind(req.due_date)
        .bind(req.idempotency_key.trim())
        .fetch_one(&mut *tx)
        .await?;

        // ── Outbox ──
        let event_id = Uuid::new_v4();
        let event_payload = serde_json::json!({
            "calibration_id": id,
            "tenant_id": &req.tenant_id,
            "asset_id": req.asset_id,
            "calibration_type": req.calibration_type.trim(),
            "due_date": req.due_date.to_string(),
        });
        let env = envelope::create_envelope(
            event_id,
            req.tenant_id.clone(),
            subjects::CALIBRATION_CREATED.to_string(),
            event_payload,
        );
        let env_json = envelope::validate_envelope(&env)
            .map_err(|e| CalibrationError::Validation(format!("envelope: {}", e)))?;
        outbox::enqueue_event_tx(
            &mut tx,
            event_id,
            subjects::CALIBRATION_CREATED,
            "calibration",
            &id.to_string(),
            &env_json,
        )
        .await?;

        tx.commit().await?;
        Ok(record)
    }

    /// Mark a calibration as completed with certificate reference.
    ///
    /// Guard: rejects if already completed (immutability invariant).
    pub async fn complete(
        pool: &PgPool,
        calibration_id: Uuid,
        req: &CompleteCalibrationRequest,
    ) -> Result<CalibrationRecord, CalibrationError> {
        // ── Guards ──
        if req.tenant_id.trim().is_empty() {
            return Err(CalibrationError::Validation("tenant_id is required".into()));
        }
        if req.certificate_ref.trim().is_empty() {
            return Err(CalibrationError::Validation(
                "certificate_ref is required".into(),
            ));
        }

        let mut tx = pool.begin().await?;

        // Fetch current record with row lock
        let current = sqlx::query_as::<_, CalibrationRecord>(
            "SELECT * FROM calibration_records WHERE id = $1 AND tenant_id = $2 FOR UPDATE",
        )
        .bind(calibration_id)
        .bind(&req.tenant_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(CalibrationError::NotFound)?;

        if current.status == "completed" {
            return Err(CalibrationError::AlreadyCompleted);
        }

        // ── Mutation ──
        let completed_date = req.completed_date.unwrap_or_else(Utc::now);
        let record = sqlx::query_as::<_, CalibrationRecord>(
            r#"
            UPDATE calibration_records SET
                status         = 'completed',
                completed_date = $3,
                certificate_ref = $4,
                updated_at     = NOW()
            WHERE id = $1 AND tenant_id = $2
            RETURNING *
            "#,
        )
        .bind(calibration_id)
        .bind(&req.tenant_id)
        .bind(completed_date)
        .bind(req.certificate_ref.trim())
        .fetch_one(&mut *tx)
        .await?;

        // ── Outbox ──
        let event_id = Uuid::new_v4();
        let event_payload = serde_json::json!({
            "calibration_id": calibration_id,
            "tenant_id": &req.tenant_id,
            "asset_id": current.asset_id,
            "calibration_type": &current.calibration_type,
            "certificate_ref": req.certificate_ref.trim(),
            "completed_date": completed_date.to_rfc3339(),
        });
        let env = envelope::create_envelope(
            event_id,
            req.tenant_id.clone(),
            subjects::CALIBRATION_COMPLETED.to_string(),
            event_payload,
        );
        let env_json = envelope::validate_envelope(&env)
            .map_err(|e| CalibrationError::Validation(format!("envelope: {}", e)))?;
        outbox::enqueue_event_tx(
            &mut tx,
            event_id,
            subjects::CALIBRATION_COMPLETED,
            "calibration",
            &calibration_id.to_string(),
            &env_json,
        )
        .await?;

        tx.commit().await?;
        Ok(record)
    }

    /// Find a calibration record by ID within tenant scope.
    pub async fn find_by_id(
        pool: &PgPool,
        id: Uuid,
        tenant_id: &str,
    ) -> Result<Option<CalibrationRecord>, CalibrationError> {
        sqlx::query_as::<_, CalibrationRecord>(
            "SELECT * FROM calibration_records WHERE id = $1 AND tenant_id = $2",
        )
        .bind(id)
        .bind(tenant_id)
        .fetch_optional(pool)
        .await
        .map_err(CalibrationError::Database)
    }

    /// Find overdue calibrations — scheduled records with due_date before today.
    pub async fn find_overdue(
        pool: &PgPool,
        tenant_id: &str,
    ) -> Result<Vec<OverdueCalibration>, CalibrationError> {
        let today = Utc::now().date_naive();
        sqlx::query_as::<_, OverdueCalibration>(
            r#"
            SELECT id, tenant_id, asset_id, calibration_type, due_date,
                   ($2 - due_date)::INT AS days_overdue
            FROM calibration_records
            WHERE tenant_id = $1
              AND status = 'scheduled'
              AND due_date < $2
            ORDER BY due_date ASC
            "#,
        )
        .bind(tenant_id)
        .bind(today)
        .fetch_all(pool)
        .await
        .map_err(CalibrationError::Database)
    }

    /// Find all overdue calibrations across all tenants (for batch compliance checks).
    pub async fn find_all_overdue(
        pool: &PgPool,
    ) -> Result<Vec<OverdueCalibration>, CalibrationError> {
        let today = Utc::now().date_naive();
        sqlx::query_as::<_, OverdueCalibration>(
            r#"
            SELECT id, tenant_id, asset_id, calibration_type, due_date,
                   ($1 - due_date)::INT AS days_overdue
            FROM calibration_records
            WHERE status = 'scheduled'
              AND due_date < $1
            ORDER BY due_date ASC
            "#,
        )
        .bind(today)
        .fetch_all(pool)
        .await
        .map_err(CalibrationError::Database)
    }
}
