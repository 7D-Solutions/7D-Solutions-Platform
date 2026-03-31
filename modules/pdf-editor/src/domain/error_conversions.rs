//! Conversions from domain errors to `ApiError`.

use platform_http_contracts::ApiError;

use super::forms::FormError;
use super::submissions::SubmissionError;

impl From<FormError> for ApiError {
    fn from(err: FormError) -> Self {
        match err {
            FormError::TemplateNotFound => ApiError::not_found("Template not found"),
            FormError::FieldNotFound => ApiError::not_found("Field not found"),
            FormError::DuplicateFieldKey => {
                ApiError::conflict("Field key already exists on this template")
            }
            FormError::Validation(msg) => ApiError::bad_request(msg),
            FormError::Database(e) => {
                tracing::error!(error = %e, "form database error");
                ApiError::internal("Database error")
            }
        }
    }
}

impl From<SubmissionError> for ApiError {
    fn from(err: SubmissionError) -> Self {
        match err {
            SubmissionError::NotFound => ApiError::not_found("Submission not found"),
            SubmissionError::TemplateNotFound => ApiError::not_found("Template not found"),
            SubmissionError::AlreadySubmitted => {
                ApiError::conflict("Submission has already been submitted")
            }
            SubmissionError::Validation(msg) => ApiError::bad_request(msg),
            SubmissionError::Database(e) => {
                tracing::error!(error = %e, "submission database error");
                ApiError::internal("Database error")
            }
        }
    }
}
