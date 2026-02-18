//! Consolidation configuration domain — groups, entities, COA mappings,
//! elimination rules, and FX translation policies.

pub mod models;
pub mod service;
pub mod service_rules;

use thiserror::Error;
use uuid::Uuid;

pub use models::*;

// ============================================================================
// Error type
// ============================================================================

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("Group not found: {0}")]
    GroupNotFound(Uuid),

    #[error("Entity not found: {0}")]
    EntityNotFound(Uuid),

    #[error("Elimination rule not found: {0}")]
    RuleNotFound(Uuid),

    #[error("FX policy not found: {0}")]
    PolicyNotFound(Uuid),

    #[error("COA mapping not found: {0}")]
    MappingNotFound(Uuid),

    #[error("Validation error: {0}")]
    Validation(String),

    #[error("Conflict: {0}")]
    Conflict(String),

    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),
}

// ============================================================================
// Validation helpers
// ============================================================================

const VALID_CONSOLIDATION_METHODS: &[&str] = &["full", "proportional", "equity"];
const VALID_RATE_TYPES: &[&str] = &["closing", "average", "historical"];
const VALID_RULE_TYPES: &[&str] = &[
    "intercompany_revenue_cost",
    "intercompany_receivable_payable",
    "intercompany_investment_equity",
    "custom",
];

pub fn validate_currency(c: &str) -> Result<(), ConfigError> {
    if c.len() != 3 || !c.chars().all(|ch| ch.is_ascii_alphabetic()) {
        return Err(ConfigError::Validation(
            "currency must be a 3-letter ISO 4217 code".into(),
        ));
    }
    Ok(())
}

pub fn validate_consolidation_method(m: &str) -> Result<(), ConfigError> {
    if !VALID_CONSOLIDATION_METHODS.contains(&m) {
        return Err(ConfigError::Validation(format!(
            "consolidation_method must be one of: {}",
            VALID_CONSOLIDATION_METHODS.join(", ")
        )));
    }
    Ok(())
}

pub fn validate_rate_type(rt: &str, field: &str) -> Result<(), ConfigError> {
    if !VALID_RATE_TYPES.contains(&rt) {
        return Err(ConfigError::Validation(format!(
            "{} must be one of: {}",
            field,
            VALID_RATE_TYPES.join(", ")
        )));
    }
    Ok(())
}

pub fn validate_rule_type(rt: &str) -> Result<(), ConfigError> {
    if !VALID_RULE_TYPES.contains(&rt) {
        return Err(ConfigError::Validation(format!(
            "rule_type must be one of: {}",
            VALID_RULE_TYPES.join(", ")
        )));
    }
    Ok(())
}

pub fn validate_not_blank(value: &str, field: &str) -> Result<(), ConfigError> {
    if value.trim().is_empty() {
        return Err(ConfigError::Validation(format!("{} cannot be blank", field)));
    }
    Ok(())
}

pub fn validate_ownership_bp(bp: i32) -> Result<(), ConfigError> {
    if bp <= 0 || bp > 10000 {
        return Err(ConfigError::Validation(
            "ownership_pct_bp must be 1–10000 (basis points)".into(),
        ));
    }
    Ok(())
}

pub fn validate_fiscal_month(m: i16) -> Result<(), ConfigError> {
    if !(1..=12).contains(&m) {
        return Err(ConfigError::Validation(
            "fiscal_year_end_month must be 1–12".into(),
        ));
    }
    Ok(())
}
