use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// A single field-level validation error.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct FieldError {
    pub field: String,
    pub message: String,
}

/// Standard API error envelope.
///
/// `error` is a machine-readable code (`not_found`, `validation_error`, etc.).
/// `message` is human-readable.  `request_id` is populated from the tracing
/// context already present in request extensions.  `details` carries per-field
/// errors for 422 responses.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ApiError {
    pub error: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<Vec<FieldError>>,

    /// HTTP status code — serialized for internal use but skipped in JSON output.
    #[serde(skip)]
    #[schema(ignore)]
    status: u16,
}

impl ApiError {
    pub fn new(status: u16, error: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            error: error.into(),
            message: message.into(),
            request_id: None,
            details: None,
            status,
        }
    }

    pub fn with_request_id(mut self, id: impl Into<String>) -> Self {
        self.request_id = Some(id.into());
        self
    }

    pub fn with_details(mut self, details: Vec<FieldError>) -> Self {
        self.details = Some(details);
        self
    }

    pub fn status_code(&self) -> u16 {
        self.status
    }

    // ── Common constructors ────────────────────────────────────────────

    pub fn not_found(message: impl Into<String>) -> Self {
        Self::new(404, "not_found", message)
    }

    pub fn bad_request(message: impl Into<String>) -> Self {
        Self::new(400, "bad_request", message)
    }

    pub fn validation_error(message: impl Into<String>, details: Vec<FieldError>) -> Self {
        Self::new(422, "validation_error", message).with_details(details)
    }

    pub fn internal(message: impl Into<String>) -> Self {
        Self::new(500, "internal_error", message)
    }

    pub fn conflict(message: impl Into<String>) -> Self {
        Self::new(409, "conflict", message)
    }

    pub fn unauthorized(message: impl Into<String>) -> Self {
        Self::new(401, "unauthorized", message)
    }

    pub fn forbidden(message: impl Into<String>) -> Self {
        Self::new(403, "forbidden", message)
    }
}

impl std::fmt::Display for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.error, self.message)
    }
}

impl std::error::Error for ApiError {}

// ── axum IntoResponse ──────────────────────────────────────────────────

#[cfg(feature = "axum")]
impl axum::response::IntoResponse for ApiError {
    fn into_response(self) -> axum::response::Response {
        let status = http::StatusCode::from_u16(self.status)
            .unwrap_or(http::StatusCode::INTERNAL_SERVER_ERROR);
        (status, axum::Json(self)).into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn api_error_serializes_without_optional_fields() -> Result<(), Box<dyn std::error::Error>> {
        let err = ApiError::new(404, "not_found", "Item not found");
        let json = serde_json::to_value(&err)?;

        assert_eq!(json["error"], "not_found");
        assert_eq!(json["message"], "Item not found");
        assert!(json.get("request_id").is_none());
        assert!(json.get("details").is_none());
        // status field is skip — must not appear
        assert!(json.get("status").is_none());
        Ok(())
    }

    #[test]
    fn api_error_serializes_with_all_fields() -> Result<(), Box<dyn std::error::Error>> {
        let err = ApiError::validation_error(
            "Validation failed",
            vec![
                FieldError { field: "email".into(), message: "invalid format".into() },
                FieldError { field: "name".into(), message: "required".into() },
            ],
        )
        .with_request_id("req-abc-123");

        let json = serde_json::to_value(&err)?;

        assert_eq!(json["error"], "validation_error");
        assert_eq!(json["request_id"], "req-abc-123");
        let details = json["details"].as_array().expect("details should be array");
        assert_eq!(details.len(), 2);
        assert_eq!(details[0]["field"], "email");
        Ok(())
    }

    #[test]
    fn api_error_round_trip() -> Result<(), Box<dyn std::error::Error>> {
        let err = ApiError::not_found("Gone").with_request_id("r1");
        let json = serde_json::to_string(&err)?;
        let deser: ApiError = serde_json::from_str(&json)?;

        assert_eq!(deser.error, "not_found");
        assert_eq!(deser.message, "Gone");
        assert_eq!(deser.request_id.as_deref(), Some("r1"));
        // status is skipped in serialization, so deserialized value is default (0)
        assert_eq!(deser.status, 0);
        Ok(())
    }

    #[test]
    fn field_error_construction() -> Result<(), Box<dyn std::error::Error>> {
        let fe = FieldError {
            field: "quantity".into(),
            message: "must be positive".into(),
        };
        let json = serde_json::to_value(&fe)?;
        assert_eq!(json["field"], "quantity");
        assert_eq!(json["message"], "must be positive");
        Ok(())
    }

    #[test]
    fn common_constructors_set_correct_status() {
        assert_eq!(ApiError::not_found("x").status_code(), 404);
        assert_eq!(ApiError::bad_request("x").status_code(), 400);
        assert_eq!(ApiError::validation_error("x", vec![]).status_code(), 422);
        assert_eq!(ApiError::internal("x").status_code(), 500);
        assert_eq!(ApiError::conflict("x").status_code(), 409);
        assert_eq!(ApiError::unauthorized("x").status_code(), 401);
        assert_eq!(ApiError::forbidden("x").status_code(), 403);
    }

    /// Test IntoResponse — only runs when the `axum` feature is active.
    #[cfg(feature = "axum")]
    #[tokio::test]
    async fn api_error_into_response_status_and_body() -> Result<(), Box<dyn std::error::Error>> {
        use axum::response::IntoResponse;
        use http::StatusCode;

        let err = ApiError::not_found("Item 42 not found").with_request_id("req-1");
        let response = err.into_response();

        assert_eq!(response.status(), StatusCode::NOT_FOUND);

        let body = axum::body::to_bytes(response.into_body(), 4096).await?;
        let json: serde_json::Value = serde_json::from_slice(&body)?;
        assert_eq!(json["error"], "not_found");
        assert_eq!(json["message"], "Item 42 not found");
        assert_eq!(json["request_id"], "req-1");
        Ok(())
    }
}
