//! Vendor bounded context — types, validation, and due-date derivation.
//!
//! Vendors are the master identity anchor for all AP transactions.
//! All fields are tenant-scoped via tenant_id.
//! Payment method metadata is stored (method type, remittance pointers) but
//! NO secret credentials are ever persisted here.

pub mod service;

use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

// ============================================================================
// Error Types
// ============================================================================

#[derive(Debug, Error)]
pub enum VendorError {
    #[error("Vendor not found: {0}")]
    NotFound(Uuid),

    #[error("Duplicate vendor name '{0}' already exists for tenant")]
    DuplicateName(String),

    #[error("Validation error: {0}")]
    Validation(String),

    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),
}

// ============================================================================
// Payment Terms
// ============================================================================

/// Named payment term presets. Stored as days in the DB.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PaymentTermsPreset {
    Net15,
    Net30,
    Net45,
    Net60,
    Net90,
    /// Immediate payment on receipt
    Net0,
}

impl PaymentTermsPreset {
    pub fn days(self) -> i32 {
        match self {
            PaymentTermsPreset::Net0 => 0,
            PaymentTermsPreset::Net15 => 15,
            PaymentTermsPreset::Net30 => 30,
            PaymentTermsPreset::Net45 => 45,
            PaymentTermsPreset::Net60 => 60,
            PaymentTermsPreset::Net90 => 90,
        }
    }

    pub fn from_days(days: i32) -> Option<Self> {
        match days {
            0 => Some(PaymentTermsPreset::Net0),
            15 => Some(PaymentTermsPreset::Net15),
            30 => Some(PaymentTermsPreset::Net30),
            45 => Some(PaymentTermsPreset::Net45),
            60 => Some(PaymentTermsPreset::Net60),
            90 => Some(PaymentTermsPreset::Net90),
            _ => None,
        }
    }
}

/// Compute the due date deterministically from an invoice date and payment terms.
///
/// This is a pure function — given the same inputs it always returns the same date.
/// `payment_terms_days` must be >= 0.
pub fn compute_due_date(invoice_date: NaiveDate, payment_terms_days: i32) -> NaiveDate {
    debug_assert!(payment_terms_days >= 0, "payment_terms_days must be non-negative");
    let days = payment_terms_days.max(0) as i64;
    invoice_date + chrono::Duration::days(days)
}

// ============================================================================
// Domain Structs
// ============================================================================

/// Full vendor record as stored and returned.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Vendor {
    pub vendor_id: Uuid,
    pub tenant_id: String,
    pub name: String,
    pub tax_id: Option<String>,
    /// ISO 4217 currency code (e.g. "USD")
    pub currency: String,
    /// Net payment terms in calendar days (e.g. 30 = Net-30)
    pub payment_terms_days: i32,
    /// Preferred payment method type: "ach", "wire", "check", etc.
    pub payment_method: Option<String>,
    /// Remittance email address (pointer, not a credential)
    pub remittance_email: Option<String>,
    pub is_active: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    /// Optional link to a Party record in the party-master service.
    #[sqlx(default)]
    pub party_id: Option<Uuid>,
}

/// Request body to create a new vendor.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateVendorRequest {
    pub name: String,
    pub tax_id: Option<String>,
    /// ISO 4217 currency code
    pub currency: String,
    /// Payment terms in days. Use PaymentTermsPreset for named values.
    pub payment_terms_days: i32,
    /// Payment method type (e.g. "ach", "wire", "check")
    pub payment_method: Option<String>,
    /// Remittance email (pointer only, no secrets)
    pub remittance_email: Option<String>,
    /// Optional link to a Party record in the party-master service.
    pub party_id: Option<Uuid>,
}

/// Request body to update an existing vendor. All fields are optional (partial update).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateVendorRequest {
    pub name: Option<String>,
    pub tax_id: Option<String>,
    pub currency: Option<String>,
    pub payment_terms_days: Option<i32>,
    pub payment_method: Option<String>,
    pub remittance_email: Option<String>,
    /// Actor performing the update (for event attribution)
    pub updated_by: Option<String>,
    /// Optional link to a Party record in the party-master service.
    pub party_id: Option<Uuid>,
}

// ============================================================================
// Validation
// ============================================================================

impl CreateVendorRequest {
    pub fn validate(&self) -> Result<(), VendorError> {
        if self.name.trim().is_empty() {
            return Err(VendorError::Validation("name cannot be empty".to_string()));
        }
        validate_currency_code(&self.currency)?;
        if self.payment_terms_days < 0 {
            return Err(VendorError::Validation(
                "payment_terms_days must be >= 0".to_string(),
            ));
        }
        Ok(())
    }
}

impl UpdateVendorRequest {
    pub fn validate(&self) -> Result<(), VendorError> {
        if let Some(ref name) = self.name {
            if name.trim().is_empty() {
                return Err(VendorError::Validation("name cannot be empty".to_string()));
            }
        }
        if let Some(ref currency) = self.currency {
            validate_currency_code(currency)?;
        }
        if let Some(days) = self.payment_terms_days {
            if days < 0 {
                return Err(VendorError::Validation(
                    "payment_terms_days must be >= 0".to_string(),
                ));
            }
        }
        Ok(())
    }
}

fn validate_currency_code(currency: &str) -> Result<(), VendorError> {
    let trimmed = currency.trim();
    if trimmed.len() != 3 || !trimmed.chars().all(|c| c.is_ascii_alphabetic()) {
        return Err(VendorError::Validation(
            "currency must be a 3-letter ISO 4217 code (e.g. USD)".to_string(),
        ));
    }
    Ok(())
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    #[test]
    fn compute_due_date_net30() {
        let invoice = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
        let due = compute_due_date(invoice, 30);
        assert_eq!(due, NaiveDate::from_ymd_opt(2026, 1, 31).unwrap());
    }

    #[test]
    fn compute_due_date_net0_is_same_day() {
        let invoice = NaiveDate::from_ymd_opt(2026, 3, 15).unwrap();
        let due = compute_due_date(invoice, 0);
        assert_eq!(due, invoice);
    }

    #[test]
    fn compute_due_date_net15() {
        let invoice = NaiveDate::from_ymd_opt(2026, 2, 14).unwrap();
        let due = compute_due_date(invoice, 15);
        assert_eq!(due, NaiveDate::from_ymd_opt(2026, 3, 1).unwrap());
    }

    #[test]
    fn compute_due_date_crosses_year_boundary() {
        let invoice = NaiveDate::from_ymd_opt(2025, 12, 15).unwrap();
        let due = compute_due_date(invoice, 30);
        assert_eq!(due, NaiveDate::from_ymd_opt(2026, 1, 14).unwrap());
    }

    #[test]
    fn payment_terms_preset_days() {
        assert_eq!(PaymentTermsPreset::Net30.days(), 30);
        assert_eq!(PaymentTermsPreset::Net15.days(), 15);
        assert_eq!(PaymentTermsPreset::Net0.days(), 0);
    }

    #[test]
    fn payment_terms_preset_from_days() {
        assert_eq!(PaymentTermsPreset::from_days(30), Some(PaymentTermsPreset::Net30));
        assert_eq!(PaymentTermsPreset::from_days(99), None);
    }

    #[test]
    fn create_vendor_request_validates_name() {
        let req = CreateVendorRequest {
            name: "  ".to_string(),
            tax_id: None,
            currency: "USD".to_string(),
            payment_terms_days: 30,
            payment_method: None,
            remittance_email: None,
            party_id: None,
        };
        assert!(req.validate().is_err());
    }

    #[test]
    fn create_vendor_request_validates_currency() {
        let req = CreateVendorRequest {
            name: "Acme".to_string(),
            tax_id: None,
            currency: "US".to_string(), // too short
            payment_terms_days: 30,
            payment_method: None,
            remittance_email: None,
            party_id: None,
        };
        assert!(req.validate().is_err());
    }

    #[test]
    fn create_vendor_validation_rejects_numeric_currency() {
        let req = CreateVendorRequest {
            name: "Acme".to_string(),
            tax_id: None,
            currency: "123".to_string(),
            payment_terms_days: 30,
            payment_method: None,
            remittance_email: None,
            party_id: None,
        };
        assert!(req.validate().is_err());
    }

    #[test]
    fn update_vendor_validation_rejects_numeric_currency() {
        let req = UpdateVendorRequest {
            name: None,
            tax_id: None,
            currency: Some("9$D".to_string()),
            payment_terms_days: None,
            payment_method: None,
            remittance_email: None,
            updated_by: None,
            party_id: None,
        };
        assert!(req.validate().is_err());
    }

    #[test]
    fn create_vendor_request_validates_terms() {
        let req = CreateVendorRequest {
            name: "Acme".to_string(),
            tax_id: None,
            currency: "USD".to_string(),
            payment_terms_days: -1,
            payment_method: None,
            remittance_email: None,
            party_id: None,
        };
        assert!(req.validate().is_err());
    }

    #[test]
    fn create_vendor_request_valid() {
        let req = CreateVendorRequest {
            name: "Acme Corp".to_string(),
            tax_id: Some("12-3456789".to_string()),
            currency: "USD".to_string(),
            payment_terms_days: 30,
            payment_method: Some("ach".to_string()),
            remittance_email: Some("ap@acme.example".to_string()),
            party_id: None,
        };
        assert!(req.validate().is_ok());
    }

    #[test]
    fn create_vendor_request_party_id_is_optional() {
        let req = CreateVendorRequest {
            name: "Acme Corp".to_string(),
            tax_id: None,
            currency: "USD".to_string(),
            payment_terms_days: 30,
            payment_method: None,
            remittance_email: None,
            party_id: None,
        };
        assert!(req.party_id.is_none());
        assert!(req.validate().is_ok());
    }

    #[test]
    fn create_vendor_request_accepts_party_id() {
        let id = Uuid::new_v4();
        let req = CreateVendorRequest {
            name: "Acme Corp".to_string(),
            tax_id: None,
            currency: "USD".to_string(),
            payment_terms_days: 30,
            payment_method: None,
            remittance_email: None,
            party_id: Some(id),
        };
        assert_eq!(req.party_id, Some(id));
        assert!(req.validate().is_ok());
    }

    #[test]
    fn update_vendor_request_party_id_is_optional() {
        let req = UpdateVendorRequest {
            name: None,
            tax_id: None,
            currency: None,
            payment_terms_days: None,
            payment_method: None,
            remittance_email: None,
            updated_by: None,
            party_id: None,
        };
        assert!(req.party_id.is_none());
    }
}
