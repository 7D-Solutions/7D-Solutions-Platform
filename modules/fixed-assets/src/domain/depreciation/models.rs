//! Domain models for depreciation runs and schedules.
//!
//! All monetary values are i64 minor units + TEXT currency.
//! Tenant-scoped: every query filters by tenant_id.

use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// One period's planned depreciation for an asset (row in fa_depreciation_schedules).
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct DepreciationSchedule {
    pub id: Uuid,
    pub tenant_id: String,
    pub asset_id: Uuid,
    pub period_number: i32,
    pub period_start: NaiveDate,
    pub period_end: NaiveDate,
    pub depreciation_amount_minor: i64,
    pub currency: String,
    pub cumulative_depreciation_minor: i64,
    pub remaining_book_value_minor: i64,
    pub is_posted: bool,
    pub posted_at: Option<DateTime<Utc>>,
    pub posted_by_run_id: Option<Uuid>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// A depreciation run: posts all unposted schedule periods up to as_of_date.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct DepreciationRun {
    pub id: Uuid,
    pub tenant_id: String,
    pub as_of_date: NaiveDate,
    pub status: String,
    pub assets_processed: i32,
    pub periods_posted: i32,
    pub total_depreciation_minor: i64,
    pub currency: String,
    pub error_message: Option<String>,
    pub idempotency_key: Uuid,
    pub started_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub created_by: Option<String>,
}

/// Request to generate/refresh the depreciation schedule for a single asset.
#[derive(Debug, Deserialize)]
pub struct GenerateScheduleRequest {
    pub tenant_id: String,
    pub asset_id: Uuid,
}

/// Request to execute a depreciation run for a tenant up to as_of_date.
#[derive(Debug, Deserialize)]
pub struct CreateRunRequest {
    pub tenant_id: String,
    pub as_of_date: NaiveDate,
    pub currency: Option<String>,
    pub created_by: Option<String>,
}

/// Outbox event emitted when a run completes.
#[derive(Debug, Serialize)]
pub struct DepreciationRunCompletedEvent {
    pub run_id: Uuid,
    pub tenant_id: String,
    pub as_of_date: NaiveDate,
    pub periods_posted: i32,
    pub total_depreciation_minor: i64,
}

#[derive(Debug, thiserror::Error)]
pub enum DepreciationError {
    #[error("Asset not found: {0}")]
    AssetNotFound(Uuid),
    #[error("Asset has no in-service date: {0}")]
    AssetNotInService(Uuid),
    #[error("Depreciation method not supported in straight-line engine: {0}")]
    UnsupportedMethod(String),
    #[error("Validation error: {0}")]
    Validation(String),
    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),
}
