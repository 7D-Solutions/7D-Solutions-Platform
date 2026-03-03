//! Scorecard domain types — vendor performance metrics and review tracking.

use chrono::{DateTime, NaiveDate, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::party::PartyError;

// ============================================================================
// Model
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Scorecard {
    pub id: Uuid,
    pub party_id: Uuid,
    pub app_id: String,
    pub metric_name: String,
    pub score: Decimal,
    pub max_score: Decimal,
    pub review_date: NaiveDate,
    pub reviewer: Option<String>,
    pub notes: Option<String>,
    pub idempotency_key: Option<String>,
    pub metadata: Option<serde_json::Value>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

// ============================================================================
// Create Request
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateScorecardRequest {
    pub metric_name: String,
    pub score: f64,
    pub max_score: Option<f64>,
    pub review_date: NaiveDate,
    pub reviewer: Option<String>,
    pub notes: Option<String>,
    pub idempotency_key: Option<String>,
    pub metadata: Option<serde_json::Value>,
}

impl CreateScorecardRequest {
    pub fn validate(&self) -> Result<(), PartyError> {
        if self.metric_name.trim().is_empty() {
            return Err(PartyError::Validation(
                "metric_name cannot be empty".to_string(),
            ));
        }
        if self.score < 0.0 {
            return Err(PartyError::Validation(
                "score cannot be negative".to_string(),
            ));
        }
        if let Some(max) = self.max_score {
            if max <= 0.0 {
                return Err(PartyError::Validation(
                    "max_score must be positive".to_string(),
                ));
            }
        }
        Ok(())
    }
}

// ============================================================================
// Update Request
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateScorecardRequest {
    pub metric_name: Option<String>,
    pub score: Option<f64>,
    pub max_score: Option<f64>,
    pub review_date: Option<NaiveDate>,
    pub reviewer: Option<String>,
    pub notes: Option<String>,
    pub metadata: Option<serde_json::Value>,
}

impl UpdateScorecardRequest {
    pub fn validate(&self) -> Result<(), PartyError> {
        if let Some(ref name) = self.metric_name {
            if name.trim().is_empty() {
                return Err(PartyError::Validation(
                    "metric_name cannot be empty".to_string(),
                ));
            }
        }
        if let Some(s) = self.score {
            if s < 0.0 {
                return Err(PartyError::Validation(
                    "score cannot be negative".to_string(),
                ));
            }
        }
        Ok(())
    }
}
