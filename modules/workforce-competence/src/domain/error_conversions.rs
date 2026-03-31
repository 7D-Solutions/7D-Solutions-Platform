//! Conversions from domain errors to `ApiError`.

use platform_http_contracts::ApiError;

use super::guards::GuardError;
use super::service::ServiceError;

impl From<ServiceError> for ApiError {
    fn from(err: ServiceError) -> Self {
        match err {
            ServiceError::Guard(guard_err) => guard_err.into(),
            ServiceError::ConflictingIdempotencyKey => {
                ApiError::conflict("Idempotency key already used with a different request body")
            }
            ServiceError::Serialization(e) => {
                tracing::error!(error = %e, "serialization error");
                ApiError::internal("Serialization error")
            }
            ServiceError::Database(e) => {
                tracing::error!(error = %e, "database error");
                ApiError::internal("Database error")
            }
        }
    }
}

impl From<GuardError> for ApiError {
    fn from(err: GuardError) -> Self {
        match err {
            GuardError::Validation(msg) => ApiError::new(422, "validation_error", msg),
            GuardError::ArtifactNotFound => {
                ApiError::not_found("Artifact not found or does not belong to this tenant")
            }
            GuardError::ArtifactInactive => {
                ApiError::new(422, "artifact_inactive", "Artifact is inactive")
            }
            GuardError::Database(e) => {
                tracing::error!(error = %e, "guard database error");
                ApiError::internal("Database error")
            }
        }
    }
}
