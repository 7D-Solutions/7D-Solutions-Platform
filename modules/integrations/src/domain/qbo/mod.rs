//! QuickBooks Online REST API client.
//!
//! Handles QBO-specific conventions: `?minorversion=75` on all requests,
//! `?requestid=UUID` on writes, SyncToken optimistic locking with retry,
//! SQL-like query pagination, and CDC polling.

pub mod cdc;
pub mod client;
pub mod outbound;
pub mod repo;
pub mod sync;

use serde::Deserialize;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum QboError {
    #[error("HTTP request failed: {0}")]
    Http(#[from] reqwest::Error),
    #[error("QBO API fault ({fault_type}): {message}")]
    ApiFault {
        fault_type: String,
        message: String,
        code: String,
        detail: String,
    },
    #[error("SyncToken stale after {0} retries")]
    SyncTokenExhausted(u32),
    #[error("Rate limited — retry after {retry_after:?}")]
    RateLimited {
        retry_after: Option<std::time::Duration>,
    },
    #[error("Authentication failed — token needs refresh")]
    AuthFailed,
    #[error("Token provider error: {0}")]
    TokenError(String),
    #[error("Deserialization error: {0}")]
    Deserialize(String),
    /// A touched business field changed in QBO between our read and stale retry.
    ///
    /// Indicates a concurrent edit conflict. The caller should record a conflict
    /// row and stop retrying.
    #[error("concurrent edit conflict on entity {entity_id}: touched field changed in QBO")]
    ConflictDetected {
        entity_id: String,
        fresh_entity: serde_json::Value,
    },
}

/// What action to take based on a QBO error response.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QboApiAction {
    /// Token expired — refresh and retry.
    RefreshToken,
    /// Rate limited — back off and retry.
    Backoff,
    /// SyncToken stale — re-fetch entity and retry.
    RetryWithFreshSyncToken,
    /// Unrecoverable — propagate error.
    Fail,
}

#[derive(Debug, Deserialize)]
struct FaultEnvelope {
    #[serde(rename = "Fault")]
    fault: Option<Fault>,
}

#[derive(Debug, Deserialize)]
struct Fault {
    #[serde(rename = "Error", default)]
    errors: Vec<FaultError>,
    #[serde(rename = "type", default)]
    fault_type: String,
}

#[derive(Debug, Deserialize)]
struct FaultError {
    #[serde(rename = "Message", default)]
    message: String,
    #[serde(rename = "Detail", default)]
    detail: String,
    #[serde(default)]
    code: String,
}

/// Classify a QBO error response into an actionable category.
pub fn classify_error(status: u16, body: &str) -> QboApiAction {
    if let Ok(env) = serde_json::from_str::<FaultEnvelope>(body) {
        if let Some(fault) = env.fault {
            let ft = fault.fault_type.to_lowercase();
            if ft.contains("throttl") {
                return QboApiAction::Backoff;
            }
            if ft.contains("authentication") {
                return QboApiAction::RefreshToken;
            }
            if ft.contains("validation") {
                for err in &fault.errors {
                    let lc = err.detail.to_lowercase();
                    if err.code == "5010" || lc.contains("stale object") || lc.contains("synctoken")
                    {
                        return QboApiAction::RetryWithFreshSyncToken;
                    }
                }
            }
        }
    }
    if status == 401 {
        let lower = body.to_lowercase();
        if lower.contains("throttl") || lower.contains("rate") || lower.contains("too many") {
            return QboApiAction::Backoff;
        }
        return QboApiAction::RefreshToken;
    }
    if status == 429 {
        return QboApiAction::Backoff;
    }
    QboApiAction::Fail
}

/// Parse a QBO error body into a QboError.
pub(crate) fn parse_api_error(body: &str) -> QboError {
    if let Ok(env) = serde_json::from_str::<FaultEnvelope>(body) {
        if let Some(fault) = env.fault {
            let first = fault.errors.first();
            return QboError::ApiFault {
                fault_type: fault.fault_type,
                message: first.map(|e| e.message.clone()).unwrap_or_default(),
                code: first.map(|e| e.code.clone()).unwrap_or_default(),
                detail: first.map(|e| e.detail.clone()).unwrap_or_default(),
            };
        }
    }
    QboError::Deserialize(format!("Unknown error: {}", &body[..body.len().min(200)]))
}

/// Trait for providing access tokens to the QBO client.
#[async_trait::async_trait]
pub trait TokenProvider: Send + Sync {
    /// Return the current access token.
    async fn get_token(&self) -> Result<String, QboError>;
    /// Force a token refresh and return the new access token.
    async fn refresh_token(&self) -> Result<String, QboError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_sync_token_stale() {
        let body = r#"{"Fault":{"Error":[{"Message":"Stale Object Error","Detail":"Object 123 SyncToken mismatch","code":"5010"}],"type":"ValidationFault"}}"#;
        assert_eq!(
            classify_error(400, body),
            QboApiAction::RetryWithFreshSyncToken
        );
    }

    #[test]
    fn classify_throttling_fault() {
        let body = r#"{"Fault":{"Error":[{"Message":"Throttled","Detail":"Rate limit","code":"3001"}],"type":"THROTTLING"}}"#;
        assert_eq!(classify_error(401, body), QboApiAction::Backoff);
    }

    #[test]
    fn classify_auth_fault() {
        let body = r#"{"Fault":{"Error":[{"Message":"Token expired","Detail":"auth","code":"3200"}],"type":"AUTHENTICATION"}}"#;
        assert_eq!(classify_error(401, body), QboApiAction::RefreshToken);
    }

    #[test]
    fn classify_401_empty_body_defaults_to_refresh() {
        assert_eq!(classify_error(401, ""), QboApiAction::RefreshToken);
    }

    #[test]
    fn classify_401_rate_limit_in_body_text() {
        assert_eq!(
            classify_error(401, r#"{"message":"too many requests"}"#),
            QboApiAction::Backoff
        );
    }

    #[test]
    fn classify_429_is_backoff() {
        assert_eq!(classify_error(429, "anything"), QboApiAction::Backoff);
    }

    #[test]
    fn classify_500_is_fail() {
        assert_eq!(classify_error(500, "server error"), QboApiAction::Fail);
    }
}
