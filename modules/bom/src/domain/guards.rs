use thiserror::Error;

#[derive(Debug, Error)]
pub enum GuardError {
    #[error("Validation error: {0}")]
    Validation(String),

    #[error("Not found: {0}")]
    NotFound(String),

    #[error("Conflict: {0}")]
    Conflict(String),

    #[error("Cycle detected in BOM structure")]
    CycleDetected,

    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),
}

pub fn guard_non_empty(value: &str, field: &str) -> Result<(), GuardError> {
    if value.trim().is_empty() {
        return Err(GuardError::Validation(format!("{} cannot be empty", field)));
    }
    Ok(())
}

pub fn guard_positive_quantity(quantity: f64) -> Result<(), GuardError> {
    if quantity <= 0.0 {
        return Err(GuardError::Validation(
            "quantity must be greater than zero".to_string(),
        ));
    }
    Ok(())
}

pub fn guard_scrap_factor(scrap_factor: Option<f64>) -> Result<(), GuardError> {
    if let Some(sf) = scrap_factor {
        if sf < 0.0 || sf >= 1.0 {
            return Err(GuardError::Validation(
                "scrap_factor must be >= 0 and < 1".to_string(),
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn non_empty_rejects_blank() {
        assert!(guard_non_empty("", "field").is_err());
        assert!(guard_non_empty("  ", "field").is_err());
    }

    #[test]
    fn non_empty_accepts_value() {
        assert!(guard_non_empty("abc", "field").is_ok());
    }

    #[test]
    fn positive_quantity_rejects_zero_and_negative() {
        assert!(guard_positive_quantity(0.0).is_err());
        assert!(guard_positive_quantity(-1.0).is_err());
    }

    #[test]
    fn positive_quantity_accepts_positive() {
        assert!(guard_positive_quantity(0.001).is_ok());
        assert!(guard_positive_quantity(100.0).is_ok());
    }

    #[test]
    fn scrap_factor_validates_range() {
        assert!(guard_scrap_factor(Some(-0.1)).is_err());
        assert!(guard_scrap_factor(Some(1.0)).is_err());
        assert!(guard_scrap_factor(Some(0.0)).is_ok());
        assert!(guard_scrap_factor(Some(0.5)).is_ok());
        assert!(guard_scrap_factor(None).is_ok());
    }
}
