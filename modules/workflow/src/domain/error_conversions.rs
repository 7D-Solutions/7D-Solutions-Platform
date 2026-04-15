use platform_http_contracts::ApiError;

use super::definitions::DefError;
use super::instances::InstanceError;

impl From<DefError> for ApiError {
    fn from(err: DefError) -> Self {
        match err {
            DefError::NotFound => ApiError::not_found("Workflow definition not found"),
            DefError::Validation(msg) => ApiError::new(422, "validation_error", msg),
            DefError::Duplicate => {
                ApiError::conflict("Definition with this name+version already exists")
            }
            DefError::Database(e) => {
                tracing::error!(error = %e, "workflow definition database error");
                ApiError::internal("Database error")
            }
        }
    }
}

impl From<InstanceError> for ApiError {
    fn from(err: InstanceError) -> Self {
        match err {
            InstanceError::NotFound => ApiError::not_found("Workflow instance not found"),
            InstanceError::DefinitionNotFound => {
                ApiError::not_found("Workflow definition not found")
            }
            InstanceError::Validation(msg) => ApiError::new(422, "validation_error", msg),
            InstanceError::InvalidTransition(msg) => ApiError::new(422, "invalid_transition", msg),
            InstanceError::NotActive(status) => ApiError::new(
                422,
                "not_active",
                format!("Instance is not active (status: {})", status),
            ),
            InstanceError::Database(e) => {
                tracing::error!(error = %e, "workflow instance database error");
                ApiError::internal("Database error")
            }
        }
    }
}
