use platform_http_contracts::ApiError;

use super::config::ConfigError;
use super::engine::EngineError;

impl From<ConfigError> for ApiError {
    fn from(err: ConfigError) -> Self {
        match err {
            ConfigError::GroupNotFound(id) => {
                ApiError::not_found(format!("Group {} not found", id))
            }
            ConfigError::EntityNotFound(id) => {
                ApiError::not_found(format!("Entity {} not found", id))
            }
            ConfigError::RuleNotFound(id) => {
                ApiError::not_found(format!("Elimination rule {} not found", id))
            }
            ConfigError::PolicyNotFound(id) => {
                ApiError::not_found(format!("FX policy {} not found", id))
            }
            ConfigError::MappingNotFound(id) => {
                ApiError::not_found(format!("COA mapping {} not found", id))
            }
            ConfigError::Validation(msg) => ApiError::new(422, "validation_error", msg),
            ConfigError::Conflict(msg) => ApiError::conflict(msg),
            ConfigError::Database(e) => {
                tracing::error!(error = %e, "consolidation config database error");
                ApiError::internal("Database error")
            }
        }
    }
}

impl From<EngineError> for ApiError {
    fn from(err: EngineError) -> Self {
        match &err {
            EngineError::PeriodNotClosed(_) => {
                ApiError::new(412, "precondition_failed", err.to_string())
            }
            EngineError::HashMismatch { .. } => ApiError::conflict(err.to_string()),
            EngineError::MissingCoaMapping { .. } | EngineError::MissingFxPolicy(_) => {
                ApiError::new(422, "validation_error", err.to_string())
            }
            EngineError::FxRateNotFound { .. } => {
                ApiError::new(422, "fx_rate_not_found", err.to_string())
            }
            EngineError::Config(_) => ApiError::not_found(err.to_string()),
            EngineError::GlClient(_) | EngineError::Database(_) => {
                tracing::error!(error = %err, "consolidation engine error");
                ApiError::internal("Internal consolidation error")
            }
        }
    }
}
