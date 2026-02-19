use thiserror::Error;

/// Errors that can occur when calling platform REST APIs.
#[derive(Debug, Error)]
pub enum PlatformClientError {
    /// HTTP transport / network error (DNS, timeout, connection refused, etc.)
    #[error("network error: {0}")]
    Network(#[from] reqwest::Error),

    /// The server returned a non-2xx status code.
    #[error("API error (HTTP {status}): {body}")]
    ApiError {
        status: u16,
        body: String,
    },

    /// Failed to deserialize a successful response body.
    #[error("deserialization error: {0}")]
    Deserialization(String),
}

impl PlatformClientError {
    /// Build an ApiError from a status code and body string.
    pub fn api(status: u16, body: impl Into<String>) -> Self {
        Self::ApiError {
            status,
            body: body.into(),
        }
    }
}
