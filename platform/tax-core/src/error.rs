//! Tax provider error type.

use thiserror::Error;

/// Errors returned by [`crate::TaxProvider`] implementations.
#[derive(Debug, Error)]
pub enum TaxProviderError {
    #[error("provider unavailable: {0}")]
    Unavailable(String),
    #[error("invalid request: {0}")]
    InvalidRequest(String),
    #[error("commit rejected: {0}")]
    CommitRejected(String),
    #[error("void rejected: {0}")]
    VoidRejected(String),
    #[error("provider error: {0}")]
    Provider(String),
}
