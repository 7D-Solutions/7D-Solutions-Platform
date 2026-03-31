use platform_http_contracts::ApiError;

use super::service::QiError;

impl From<QiError> for ApiError {
    fn from(err: QiError) -> Self {
        match err {
            QiError::NotFound(msg) => ApiError::not_found(msg),
            QiError::Validation(msg) => ApiError::new(422, "validation_error", msg),
            QiError::Unauthorized(msg) => ApiError::new(403, "unauthorized_inspector", msg),
            QiError::ServiceUnavailable(msg) => {
                tracing::error!(error = %msg, "workforce-competence authorization check failed");
                ApiError::new(503, "service_unavailable", msg)
            }
            QiError::Serialization(e) => {
                tracing::error!(error = %e, "serialization error");
                ApiError::internal("Serialization error")
            }
            QiError::Database(ref e) => {
                if let sqlx::Error::Database(dbe) = e {
                    if dbe.code().as_deref() == Some("23505") {
                        return ApiError::conflict(dbe.message().to_string());
                    }
                }
                tracing::error!(error = %e, "database error");
                ApiError::internal("Database error")
            }
        }
    }
}
