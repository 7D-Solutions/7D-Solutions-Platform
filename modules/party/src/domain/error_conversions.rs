use platform_http_contracts::ApiError;

use super::party::PartyError;

impl From<PartyError> for ApiError {
    fn from(err: PartyError) -> Self {
        match err {
            PartyError::NotFound(id) => ApiError::not_found(format!("Party {} not found", id)),
            PartyError::Validation(msg) => ApiError::new(422, "validation_error", msg),
            PartyError::Conflict(msg) => ApiError::conflict(msg),
            PartyError::Database(e) => {
                tracing::error!("Party DB error: {}", e);
                ApiError::internal("Internal database error")
            }
        }
    }
}
