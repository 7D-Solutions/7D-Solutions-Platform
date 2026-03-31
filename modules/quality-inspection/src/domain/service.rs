use thiserror::Error;

#[derive(Debug, Error)]
pub enum QiError {
    #[error("Not found: {0}")]
    NotFound(String),

    #[error("Validation: {0}")]
    Validation(String),

    #[error("Unauthorized: {0}")]
    Unauthorized(String),

    #[error("Service unavailable: {0}")]
    ServiceUnavailable(String),

    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),
}

// Re-export from sub-modules so callers using `service::foo()` keep working.
pub use crate::domain::inspection_service::*;
pub use crate::domain::plan_service::*;
