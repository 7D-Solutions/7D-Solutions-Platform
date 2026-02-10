use thiserror::Error;

#[derive(Error, Debug)]
pub enum TilledError {
    #[error("Configuration error: {0}")]
    ConfigError(String),

    #[error("HTTP error: {0}")]
    HttpError(String),

    #[error("API error (status {status_code}): {message}")]
    ApiError {
        status_code: u16,
        message: String,
    },

    #[error("Parse error: {0}")]
    ParseError(String),

    #[error("Webhook signature verification failed")]
    WebhookVerificationFailed,
}

impl TilledError {
    /// Check if this is a client error (4xx)
    pub fn is_client_error(&self) -> bool {
        matches!(self, TilledError::ApiError { status_code, .. } if (400..500).contains(status_code))
    }

    /// Check if this is a server error (5xx)
    pub fn is_server_error(&self) -> bool {
        matches!(self, TilledError::ApiError { status_code, .. } if (500..600).contains(status_code))
    }
}
