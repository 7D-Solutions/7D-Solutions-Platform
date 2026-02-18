//! Employee domain model and request types.
//!
//! Invariants:
//! - employee_code is unique per app_id (DB constraint + validation)
//! - first_name, last_name, employee_code must be non-empty
//! - external_payroll_id is optional (integration with ADP, Gusto, etc.)
//! - Deactivate is idempotent

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

// ============================================================================
// Domain model
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Employee {
    pub id: Uuid,
    pub app_id: String,
    pub employee_code: String,
    pub first_name: String,
    pub last_name: String,
    pub email: Option<String>,
    pub department: Option<String>,
    pub external_payroll_id: Option<String>,
    pub hourly_rate_minor: Option<i64>,
    pub currency: String,
    pub active: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

// ============================================================================
// Request types
// ============================================================================

#[derive(Debug, Deserialize)]
pub struct CreateEmployeeRequest {
    pub app_id: String,
    pub employee_code: String,
    pub first_name: String,
    pub last_name: String,
    pub email: Option<String>,
    pub department: Option<String>,
    pub external_payroll_id: Option<String>,
    pub hourly_rate_minor: Option<i64>,
    pub currency: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateEmployeeRequest {
    pub app_id: String,
    pub first_name: Option<String>,
    pub last_name: Option<String>,
    pub email: Option<String>,
    pub department: Option<String>,
    pub external_payroll_id: Option<String>,
    pub hourly_rate_minor: Option<i64>,
    pub currency: Option<String>,
}

// ============================================================================
// Errors
// ============================================================================

#[derive(Debug, Error)]
pub enum EmployeeError {
    #[error("Employee code '{0}' already exists for app '{1}'")]
    DuplicateCode(String, String),

    #[error("Employee not found")]
    NotFound,

    #[error("Validation error: {0}")]
    Validation(String),

    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),
}

// ============================================================================
// Validation
// ============================================================================

fn require_non_empty(value: &str, field: &str) -> Result<(), EmployeeError> {
    if value.trim().is_empty() {
        return Err(EmployeeError::Validation(format!(
            "{} must not be empty",
            field
        )));
    }
    Ok(())
}

impl CreateEmployeeRequest {
    pub fn validate(&self) -> Result<(), EmployeeError> {
        require_non_empty(&self.app_id, "app_id")?;
        require_non_empty(&self.employee_code, "employee_code")?;
        require_non_empty(&self.first_name, "first_name")?;
        require_non_empty(&self.last_name, "last_name")?;
        if let Some(ref email) = self.email {
            require_non_empty(email, "email")?;
        }
        Ok(())
    }
}

impl UpdateEmployeeRequest {
    pub fn validate(&self) -> Result<(), EmployeeError> {
        require_non_empty(&self.app_id, "app_id")?;
        if let Some(ref first_name) = self.first_name {
            require_non_empty(first_name, "first_name")?;
        }
        if let Some(ref last_name) = self.last_name {
            require_non_empty(last_name, "last_name")?;
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

    fn valid_create() -> CreateEmployeeRequest {
        CreateEmployeeRequest {
            app_id: "acme".to_string(),
            employee_code: "EMP-001".to_string(),
            first_name: "Jane".to_string(),
            last_name: "Doe".to_string(),
            email: Some("jane@acme.com".to_string()),
            department: Some("Engineering".to_string()),
            external_payroll_id: Some("ADP-12345".to_string()),
            hourly_rate_minor: Some(5000),
            currency: Some("USD".to_string()),
        }
    }

    #[test]
    fn create_valid() {
        assert!(valid_create().validate().is_ok());
    }

    #[test]
    fn create_empty_code_rejected() {
        let mut r = valid_create();
        r.employee_code = "  ".to_string();
        assert!(matches!(r.validate(), Err(EmployeeError::Validation(_))));
    }

    #[test]
    fn create_empty_first_name_rejected() {
        let mut r = valid_create();
        r.first_name = "".to_string();
        assert!(matches!(r.validate(), Err(EmployeeError::Validation(_))));
    }

    #[test]
    fn create_empty_last_name_rejected() {
        let mut r = valid_create();
        r.last_name = "".to_string();
        assert!(matches!(r.validate(), Err(EmployeeError::Validation(_))));
    }

    #[test]
    fn create_empty_email_rejected() {
        let mut r = valid_create();
        r.email = Some("".to_string());
        assert!(matches!(r.validate(), Err(EmployeeError::Validation(_))));
    }

    #[test]
    fn update_valid_all_none() {
        let r = UpdateEmployeeRequest {
            app_id: "acme".to_string(),
            first_name: None,
            last_name: None,
            email: None,
            department: None,
            external_payroll_id: None,
            hourly_rate_minor: None,
            currency: None,
        };
        assert!(r.validate().is_ok());
    }

    #[test]
    fn update_empty_first_name_rejected() {
        let r = UpdateEmployeeRequest {
            app_id: "acme".to_string(),
            first_name: Some("  ".to_string()),
            last_name: None,
            email: None,
            department: None,
            external_payroll_id: None,
            hourly_rate_minor: None,
            currency: None,
        };
        assert!(matches!(r.validate(), Err(EmployeeError::Validation(_))));
    }
}
