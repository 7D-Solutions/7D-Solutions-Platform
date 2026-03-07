//! Dead-letter queue (DLQ) persistence and failure classification.
//!
//! When an event cannot be processed, it is classified as one of three failure
//! kinds and written to the `event_dlq` table for later investigation or replay.
//!
//! Payloads are **redacted** before persistence: sensitive keys (passwords,
//! tokens, SSNs, etc.) are replaced with `"[REDACTED]"`.  A SHA-256 hash of
//! the original payload is stored alongside for forensic correlation.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use uuid::Uuid;

/// Keys whose values must never be persisted in cleartext.
/// Matching is case-insensitive and checks whether the key *contains* the term.
const SENSITIVE_KEY_FRAGMENTS: &[&str] = &[
    "password",
    "passwd",
    "token",
    "secret",
    "authorization",
    "api_key",
    "apikey",
    "credit_card",
    "creditcard",
    "card_number",
    "ssn",
    "social_security",
    "private_key",
    "privatekey",
    "access_key",
    "accesskey",
    "session_id",
    "sessionid",
    "cookie",
    "auth_header",
];

const REDACTED: &str = "[REDACTED]";

/// Classification of why an event ended up in the DLQ.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FailureKind {
    /// Temporary failure (e.g. DB timeout, network blip). Eligible for retry.
    Retryable,
    /// Permanent failure (e.g. schema mismatch, business-rule violation).
    /// Do not retry automatically — requires human intervention.
    Fatal,
    /// Message is structurally unparseable or violates envelope invariants.
    /// Never retry.
    Poison,
}

impl FailureKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            FailureKind::Retryable => "retryable",
            FailureKind::Fatal => "fatal",
            FailureKind::Poison => "poison",
        }
    }
}

impl std::fmt::Display for FailureKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A DLQ entry as stored in the database.
#[derive(Debug, Clone)]
pub struct DlqEntry {
    pub event_id: Uuid,
    pub subject: String,
    pub failure_kind: FailureKind,
    pub error_message: String,
    pub payload: serde_json::Value,
    pub payload_hash: String,
    pub created_at: DateTime<Utc>,
}

/// Recursively redact sensitive keys from a JSON value.
///
/// Object keys are matched case-insensitively against [`SENSITIVE_KEY_FRAGMENTS`].
/// If a key *contains* any fragment, its value is replaced with `"[REDACTED]"`.
/// Arrays and nested objects are walked recursively.
pub fn redact_payload(value: &serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::Object(map) => {
            let redacted = map
                .iter()
                .map(|(k, v)| {
                    if is_sensitive_key(k) {
                        (k.clone(), serde_json::Value::String(REDACTED.to_string()))
                    } else {
                        (k.clone(), redact_payload(v))
                    }
                })
                .collect();
            serde_json::Value::Object(redacted)
        }
        serde_json::Value::Array(arr) => {
            serde_json::Value::Array(arr.iter().map(redact_payload).collect())
        }
        other => other.clone(),
    }
}

/// Compute a hex-encoded SHA-256 hash of a JSON value's canonical bytes.
fn payload_sha256(value: &serde_json::Value) -> String {
    let bytes = serde_json::to_vec(value).unwrap_or_default();
    let hash = Sha256::digest(&bytes);
    format!("{hash:x}")
}

fn is_sensitive_key(key: &str) -> bool {
    let lower = key.to_ascii_lowercase();
    SENSITIVE_KEY_FRAGMENTS
        .iter()
        .any(|frag| lower.contains(frag))
}

/// Write a failed event to the `event_dlq` table.
///
/// The payload is **redacted** before persistence and a SHA-256 hash of the
/// original payload is stored for forensic correlation.
pub async fn write_dlq_entry(
    pool: &PgPool,
    event_id: Uuid,
    subject: &str,
    failure_kind: FailureKind,
    error_message: &str,
    payload: &serde_json::Value,
) -> Result<(), DlqError> {
    let now = Utc::now();
    let hash = payload_sha256(payload);
    let redacted = redact_payload(payload);

    sqlx::query(
        "INSERT INTO event_dlq (event_id, subject, failure_kind, error_message, payload, payload_hash, created_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7)
         ON CONFLICT (event_id) DO UPDATE
           SET failure_kind = $3, error_message = $4, payload = $5, payload_hash = $6, created_at = $7",
    )
    .bind(event_id)
    .bind(subject)
    .bind(failure_kind.as_str())
    .bind(error_message)
    .bind(&redacted)
    .bind(&hash)
    .bind(now)
    .execute(pool)
    .await
    .map_err(|e| DlqError::Database(e.to_string()))?;

    tracing::warn!(
        event_id = %event_id,
        subject = %subject,
        failure_kind = %failure_kind,
        "Event written to DLQ"
    );

    Ok(())
}

/// Read DLQ entries, optionally filtered by failure kind. Most recent first.
pub async fn list_dlq_entries(
    pool: &PgPool,
    failure_kind: Option<FailureKind>,
    limit: i64,
) -> Result<Vec<DlqEntry>, DlqError> {
    let rows = match failure_kind {
        Some(kind) => {
            sqlx::query_as::<_, DlqRow>(
                "SELECT event_id, subject, failure_kind, error_message, payload, payload_hash, created_at
                 FROM event_dlq
                 WHERE failure_kind = $1
                 ORDER BY created_at DESC
                 LIMIT $2",
            )
            .bind(kind.as_str())
            .bind(limit)
            .fetch_all(pool)
            .await
        }
        None => {
            sqlx::query_as::<_, DlqRow>(
                "SELECT event_id, subject, failure_kind, error_message, payload, payload_hash, created_at
                 FROM event_dlq
                 ORDER BY created_at DESC
                 LIMIT $1",
            )
            .bind(limit)
            .fetch_all(pool)
            .await
        }
    }
    .map_err(|e| DlqError::Database(e.to_string()))?;

    Ok(rows.into_iter().map(|r| r.into_entry()).collect())
}

/// Classify a [`crate::registry::HandlerError`] into a [`FailureKind`].
pub fn classify_handler_error(err: &crate::registry::HandlerError) -> FailureKind {
    match err {
        crate::registry::HandlerError::Transient(_) => FailureKind::Retryable,
        crate::registry::HandlerError::Permanent(_) => FailureKind::Fatal,
    }
}

// Internal row type for sqlx mapping.
#[derive(sqlx::FromRow)]
struct DlqRow {
    event_id: Uuid,
    subject: String,
    failure_kind: String,
    error_message: String,
    payload: serde_json::Value,
    payload_hash: String,
    created_at: DateTime<Utc>,
}

impl DlqRow {
    fn into_entry(self) -> DlqEntry {
        let kind = match self.failure_kind.as_str() {
            "retryable" => FailureKind::Retryable,
            "fatal" => FailureKind::Fatal,
            _ => FailureKind::Poison,
        };
        DlqEntry {
            event_id: self.event_id,
            subject: self.subject,
            failure_kind: kind,
            error_message: self.error_message,
            payload: self.payload,
            payload_hash: self.payload_hash,
            created_at: self.created_at,
        }
    }
}

/// Errors from the DLQ layer.
#[derive(Debug, thiserror::Error)]
pub enum DlqError {
    /// Database connectivity or query failure.
    #[error("database error: {0}")]
    Database(String),
}

#[cfg(test)]
mod dlq_redaction {
    use super::*;
    use serde_json::json;

    #[test]
    fn redact_top_level_password() {
        let input = json!({"user": "alice", "password": "s3cret"});
        let out = redact_payload(&input);
        assert_eq!(out["user"], "alice");
        assert_eq!(out["password"], REDACTED);
    }

    #[test]
    fn redact_nested_token() {
        let input = json!({
            "auth": {
                "access_token": "abc123",
                "refresh_token": "def456",
                "scope": "read"
            }
        });
        let out = redact_payload(&input);
        assert_eq!(out["auth"]["access_token"], REDACTED);
        assert_eq!(out["auth"]["refresh_token"], REDACTED);
        assert_eq!(out["auth"]["scope"], "read");
    }

    #[test]
    fn redact_deeply_nested() {
        let input = json!({
            "level1": {
                "level2": {
                    "api_key": "KEY-123",
                    "name": "test"
                }
            }
        });
        let out = redact_payload(&input);
        assert_eq!(out["level1"]["level2"]["api_key"], REDACTED);
        assert_eq!(out["level1"]["level2"]["name"], "test");
    }

    #[test]
    fn redact_in_array() {
        let input = json!([
            {"secret": "hidden", "id": 1},
            {"secret": "also-hidden", "id": 2}
        ]);
        let out = redact_payload(&input);
        assert_eq!(out[0]["secret"], REDACTED);
        assert_eq!(out[0]["id"], 1);
        assert_eq!(out[1]["secret"], REDACTED);
        assert_eq!(out[1]["id"], 2);
    }

    #[test]
    fn redact_case_insensitive() {
        let input = json!({
            "PASSWORD": "upper",
            "Api_Key": "mixed",
            "Credit_Card": "4111-1111"
        });
        let out = redact_payload(&input);
        assert_eq!(out["PASSWORD"], REDACTED);
        assert_eq!(out["Api_Key"], REDACTED);
        assert_eq!(out["Credit_Card"], REDACTED);
    }

    #[test]
    fn redact_ssn_and_social_security() {
        let input = json!({"ssn": "123-45-6789", "social_security_number": "987-65-4321"});
        let out = redact_payload(&input);
        assert_eq!(out["ssn"], REDACTED);
        assert_eq!(out["social_security_number"], REDACTED);
    }

    #[test]
    fn redact_credit_card_variants() {
        let input = json!({
            "credit_card": "4111111111111111",
            "creditcard_number": "5500000000000004",
            "card_number": "340000000000009"
        });
        let out = redact_payload(&input);
        assert_eq!(out["credit_card"], REDACTED);
        assert_eq!(out["creditcard_number"], REDACTED);
        assert_eq!(out["card_number"], REDACTED);
    }

    #[test]
    fn redact_authorization_header() {
        let input = json!({"authorization": "Bearer xyz", "content_type": "application/json"});
        let out = redact_payload(&input);
        assert_eq!(out["authorization"], REDACTED);
        assert_eq!(out["content_type"], "application/json");
    }

    #[test]
    fn redact_private_key() {
        let input = json!({"private_key": "-----BEGIN RSA PRIVATE KEY-----\nMII..."});
        let out = redact_payload(&input);
        assert_eq!(out["private_key"], REDACTED);
    }

    #[test]
    fn redact_session_and_cookie() {
        let input = json!({
            "session_id": "sess-abc",
            "sessionid": "sess-def",
            "cookie": "jwt=xxx"
        });
        let out = redact_payload(&input);
        assert_eq!(out["session_id"], REDACTED);
        assert_eq!(out["sessionid"], REDACTED);
        assert_eq!(out["cookie"], REDACTED);
    }

    #[test]
    fn no_redaction_on_safe_keys() {
        let input = json!({"order_id": "ORD-1", "amount": 99.99, "status": "paid"});
        let out = redact_payload(&input);
        assert_eq!(out, input);
    }

    #[test]
    fn redact_preserves_structure() {
        let input = json!({
            "event_type": "payment.completed",
            "data": {
                "amount": 100,
                "currency": "USD",
                "card_number": "4111111111111111",
                "items": [
                    {"sku": "WIDGET-1", "qty": 2},
                    {"sku": "GADGET-3", "qty": 1, "token": "promo-abc"}
                ]
            }
        });
        let out = redact_payload(&input);
        assert_eq!(out["event_type"], "payment.completed");
        assert_eq!(out["data"]["amount"], 100);
        assert_eq!(out["data"]["currency"], "USD");
        assert_eq!(out["data"]["card_number"], REDACTED);
        assert_eq!(out["data"]["items"][0]["sku"], "WIDGET-1");
        assert_eq!(out["data"]["items"][1]["token"], REDACTED);
        assert_eq!(out["data"]["items"][1]["sku"], "GADGET-3");
    }

    #[test]
    fn payload_hash_is_deterministic() {
        let payload = json!({"key": "value"});
        let h1 = payload_sha256(&payload);
        let h2 = payload_sha256(&payload);
        assert_eq!(h1, h2);
        assert_eq!(h1.len(), 64); // 256 bits = 64 hex chars
    }

    #[test]
    fn payload_hash_changes_with_content() {
        let p1 = json!({"key": "value1"});
        let p2 = json!({"key": "value2"});
        assert_ne!(payload_sha256(&p1), payload_sha256(&p2));
    }

    #[test]
    fn redact_does_not_alter_original() {
        let input = json!({"password": "secret123", "name": "test"});
        let _redacted = redact_payload(&input);
        assert_eq!(input["password"], "secret123");
    }

    #[test]
    fn redact_handles_null_and_numbers() {
        let input = json!({"password": null, "token": 12345, "name": "ok"});
        let out = redact_payload(&input);
        assert_eq!(out["password"], REDACTED);
        assert_eq!(out["token"], REDACTED);
        assert_eq!(out["name"], "ok");
    }

    #[test]
    fn redact_empty_object() {
        let input = json!({});
        let out = redact_payload(&input);
        assert_eq!(out, json!({}));
    }

    #[test]
    fn redact_scalar_passthrough() {
        let input = json!("just a string");
        let out = redact_payload(&input);
        assert_eq!(out, json!("just a string"));
    }

    #[test]
    fn redact_key_containing_fragment() {
        let input = json!({
            "user_password_hash": "abc",
            "reset_token_expiry": "2026-01-01",
            "my_secret_value": "shh"
        });
        let out = redact_payload(&input);
        assert_eq!(out["user_password_hash"], REDACTED);
        assert_eq!(out["reset_token_expiry"], REDACTED);
        assert_eq!(out["my_secret_value"], REDACTED);
    }
}
