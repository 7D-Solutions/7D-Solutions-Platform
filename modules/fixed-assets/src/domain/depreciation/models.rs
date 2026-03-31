//! Domain models for depreciation runs and schedules.
//!
//! All monetary values are i64 minor units + TEXT currency.
//! Tenant-scoped: every query filters by tenant_id.

use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

/// One period's planned depreciation for an asset (row in fa_depreciation_schedules).
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow, ToSchema)]
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
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow, ToSchema)]
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
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct GenerateScheduleRequest {
    pub tenant_id: String,
    pub asset_id: Uuid,
}

impl GenerateScheduleRequest {
    pub fn validate(&self) -> Result<(), DepreciationError> {
        if self.tenant_id.trim().is_empty() {
            return Err(DepreciationError::Validation("tenant_id required".into()));
        }
        Ok(())
    }
}

/// Request to execute a depreciation run for a tenant up to as_of_date.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct CreateRunRequest {
    pub tenant_id: String,
    pub as_of_date: NaiveDate,
    pub currency: Option<String>,
    pub created_by: Option<String>,
}

impl CreateRunRequest {
    pub fn validate(&self) -> Result<(), DepreciationError> {
        if self.tenant_id.trim().is_empty() {
            return Err(DepreciationError::Validation("tenant_id required".into()));
        }
        if let Some(ref c) = self.currency {
            let trimmed = c.trim();
            if trimmed.len() != 3 || !trimmed.chars().all(|ch| ch.is_ascii_alphabetic()) {
                return Err(DepreciationError::Validation(
                    "currency must be a 3-letter ISO 4217 code (e.g. USD)".into(),
                ));
            }
        }
        Ok(())
    }
}

/// Per-entry GL posting data embedded in DepreciationRunCompletedEvent.
///
/// Carries the GL account refs from fa_categories so the GL consumer can post
/// balanced journal entries without querying the FA database.
#[derive(Debug, Serialize, Deserialize, sqlx::FromRow)]
pub struct DepreciationGlEntry {
    /// fa_depreciation_schedules.id — used as GL idempotency key.
    pub entry_id: Uuid,
    pub asset_id: Uuid,
    pub period_end: NaiveDate,
    pub depreciation_amount_minor: i64,
    pub currency: String,
    /// fa_categories.depreciation_expense_ref (e.g. "6100")
    pub expense_account_ref: String,
    /// fa_categories.accum_depreciation_ref (e.g. "1510")
    pub accum_depreciation_ref: String,
}

/// Outbox event emitted when a run completes.
#[derive(Debug, Serialize)]
pub struct DepreciationRunCompletedEvent {
    pub run_id: Uuid,
    pub tenant_id: String,
    pub as_of_date: NaiveDate,
    pub periods_posted: i32,
    pub total_depreciation_minor: i64,
    /// Per-entry GL data for the GL consumer to post balanced journal entries.
    pub gl_entries: Vec<DepreciationGlEntry>,
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

impl From<DepreciationError> for platform_http_contracts::ApiError {
    fn from(err: DepreciationError) -> Self {
        match &err {
            DepreciationError::AssetNotFound(id) => {
                Self::not_found(format!("Asset {} not found", id))
            }
            DepreciationError::AssetNotInService(id) => Self::new(
                422,
                "asset_not_in_service",
                format!("Asset {} has no in-service date", id),
            ),
            DepreciationError::UnsupportedMethod(_) => {
                Self::new(422, "unsupported_method", err.to_string())
            }
            DepreciationError::Validation(msg) => {
                Self::new(422, "validation_error", msg.clone())
            }
            DepreciationError::Database(e) => {
                tracing::error!("Fixed-assets depreciation DB error: {}", e);
                Self::internal("Internal database error")
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    #[test]
    fn generate_schedule_validation_rejects_empty_tenant() {
        let req = GenerateScheduleRequest {
            tenant_id: "  ".into(),
            asset_id: Uuid::new_v4(),
        };
        assert!(matches!(
            req.validate(),
            Err(DepreciationError::Validation(_))
        ));
    }

    #[test]
    fn generate_schedule_validation_accepts_valid() {
        let req = GenerateScheduleRequest {
            tenant_id: "t1".into(),
            asset_id: Uuid::new_v4(),
        };
        assert!(req.validate().is_ok());
    }

    #[test]
    fn create_run_validation_rejects_empty_tenant() {
        let req = CreateRunRequest {
            tenant_id: "".into(),
            as_of_date: NaiveDate::from_ymd_opt(2026, 6, 30).expect("valid date"),
            currency: None,
            created_by: None,
        };
        assert!(matches!(
            req.validate(),
            Err(DepreciationError::Validation(_))
        ));
    }

    #[test]
    fn create_run_validation_rejects_numeric_currency() {
        let req = CreateRunRequest {
            tenant_id: "t1".into(),
            as_of_date: NaiveDate::from_ymd_opt(2026, 6, 30).expect("valid date"),
            currency: Some("123".into()),
            created_by: None,
        };
        assert!(matches!(
            req.validate(),
            Err(DepreciationError::Validation(_))
        ));
    }

    #[test]
    fn create_run_validation_rejects_short_currency() {
        let req = CreateRunRequest {
            tenant_id: "t1".into(),
            as_of_date: NaiveDate::from_ymd_opt(2026, 6, 30).expect("valid date"),
            currency: Some("US".into()),
            created_by: None,
        };
        assert!(matches!(
            req.validate(),
            Err(DepreciationError::Validation(_))
        ));
    }

    #[test]
    fn create_run_validation_accepts_valid() {
        let req = CreateRunRequest {
            tenant_id: "t1".into(),
            as_of_date: NaiveDate::from_ymd_opt(2026, 6, 30).expect("valid date"),
            currency: Some("usd".into()),
            created_by: Some("admin".into()),
        };
        assert!(req.validate().is_ok());
    }

    #[test]
    fn create_run_validation_accepts_no_currency() {
        let req = CreateRunRequest {
            tenant_id: "t1".into(),
            as_of_date: NaiveDate::from_ymd_opt(2026, 6, 30).expect("valid date"),
            currency: None,
            created_by: None,
        };
        assert!(req.validate().is_ok());
    }
}
