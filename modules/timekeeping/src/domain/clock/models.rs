//! Clock session domain model and request types.
//!
//! Invariants:
//! - No concurrent open sessions per employee within a tenant (DB + guard).
//! - duration_minutes is computed on clock-out: ceil((clock_out - clock_in) / 60).
//! - Idempotent clock-in/out via idempotency_key.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

// ============================================================================
// Domain model
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct ClockSession {
    pub id: Uuid,
    pub app_id: String,
    pub employee_id: Uuid,
    pub clock_in_at: DateTime<Utc>,
    pub clock_out_at: Option<DateTime<Utc>>,
    pub duration_minutes: Option<i32>,
    pub status: String,
    pub idempotency_key: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

// ============================================================================
// Request types
// ============================================================================

#[derive(Debug, Deserialize)]
pub struct ClockInRequest {
    pub app_id: String,
    pub employee_id: Uuid,
    pub idempotency_key: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ClockOutRequest {
    pub app_id: String,
    pub employee_id: Uuid,
    pub idempotency_key: Option<String>,
}

// ============================================================================
// Errors
// ============================================================================

#[derive(Debug, Error)]
pub enum ClockError {
    #[error("Concurrent open session exists for employee {0}")]
    ConcurrentSession(Uuid),

    #[error("No open session for employee {0}")]
    NoOpenSession(Uuid),

    #[error("Validation error: {0}")]
    Validation(String),

    #[error("Idempotent replay")]
    IdempotentReplay {
        status_code: u16,
        body: serde_json::Value,
    },

    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),
}

// ============================================================================
// Validation
// ============================================================================

impl ClockInRequest {
    pub fn validate(&self) -> Result<(), ClockError> {
        if self.app_id.trim().is_empty() {
            return Err(ClockError::Validation("app_id must not be empty".into()));
        }
        Ok(())
    }
}

impl ClockOutRequest {
    pub fn validate(&self) -> Result<(), ClockError> {
        if self.app_id.trim().is_empty() {
            return Err(ClockError::Validation("app_id must not be empty".into()));
        }
        Ok(())
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clock_in_valid() {
        let req = ClockInRequest {
            app_id: "acme".into(),
            employee_id: Uuid::new_v4(),
            idempotency_key: None,
        };
        assert!(req.validate().is_ok());
    }

    #[test]
    fn clock_in_empty_app_id() {
        let req = ClockInRequest {
            app_id: "  ".into(),
            employee_id: Uuid::new_v4(),
            idempotency_key: None,
        };
        assert!(matches!(req.validate(), Err(ClockError::Validation(_))));
    }

    #[test]
    fn clock_out_valid() {
        let req = ClockOutRequest {
            app_id: "acme".into(),
            employee_id: Uuid::new_v4(),
            idempotency_key: None,
        };
        assert!(req.validate().is_ok());
    }

    #[test]
    fn clock_out_empty_app_id() {
        let req = ClockOutRequest {
            app_id: "".into(),
            employee_id: Uuid::new_v4(),
            idempotency_key: None,
        };
        assert!(matches!(req.validate(), Err(ClockError::Validation(_))));
    }
}
