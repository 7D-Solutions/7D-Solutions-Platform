//! Dead-letter queue (DLQ) persistence and failure classification.
//!
//! When an event cannot be processed, it is classified as one of three failure
//! kinds and written to the `event_dlq` table for later investigation or replay.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use uuid::Uuid;

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
    pub created_at: DateTime<Utc>,
}

/// Write a failed event to the `event_dlq` table.
pub async fn write_dlq_entry(
    pool: &PgPool,
    event_id: Uuid,
    subject: &str,
    failure_kind: FailureKind,
    error_message: &str,
    payload: &serde_json::Value,
) -> Result<(), DlqError> {
    let now = Utc::now();

    sqlx::query(
        "INSERT INTO event_dlq (event_id, subject, failure_kind, error_message, payload, created_at)
         VALUES ($1, $2, $3, $4, $5, $6)
         ON CONFLICT (event_id) DO UPDATE
           SET failure_kind = $3, error_message = $4, payload = $5, created_at = $6",
    )
    .bind(event_id)
    .bind(subject)
    .bind(failure_kind.as_str())
    .bind(error_message)
    .bind(payload)
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
                "SELECT event_id, subject, failure_kind, error_message, payload, created_at
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
                "SELECT event_id, subject, failure_kind, error_message, payload, created_at
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
