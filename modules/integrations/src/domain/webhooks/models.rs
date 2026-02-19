//! Webhook domain models.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;

// ============================================================================
// Ingest Record
// ============================================================================

/// Row from `integrations_webhook_ingest` after a raw payload is persisted.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct WebhookIngest {
    pub id: i64,
    pub app_id: String,
    pub system: String,
    pub event_type: Option<String>,
    pub raw_payload: serde_json::Value,
    pub headers: serde_json::Value,
    pub received_at: DateTime<Utc>,
    pub processed_at: Option<DateTime<Utc>>,
    pub idempotency_key: Option<String>,
}

// ============================================================================
// Request Types
// ============================================================================

/// Input to `WebhookService::ingest()`.
#[derive(Debug, Clone)]
pub struct IngestWebhookRequest {
    /// Tenant / app identifier (from auth context or path param).
    pub app_id: String,
    /// Source system name (from URL path, e.g. "stripe").
    pub system: String,
    /// Event type if detectable before routing (e.g. from `event` field in payload).
    pub event_type: Option<String>,
    /// Idempotency key from source system (e.g. Stripe event ID).
    pub idempotency_key: Option<String>,
    /// Verbatim parsed JSON body (stored as-is).
    pub raw_payload: serde_json::Value,
    /// HTTP headers captured at receipt (lowercase names).
    pub headers: std::collections::HashMap<String, String>,
}

// ============================================================================
// Ingest Result
// ============================================================================

/// Result of a successful ingest (new record written).
#[derive(Debug, Clone)]
pub struct IngestResult {
    pub ingest_id: i64,
    pub is_duplicate: bool,
}

// ============================================================================
// Errors
// ============================================================================

#[derive(Debug, Error)]
pub enum WebhookError {
    #[error("Signature verification failed: {0}")]
    SignatureVerification(String),

    #[error("Unsupported system: {system}")]
    UnsupportedSystem { system: String },

    #[error("Malformed payload: {0}")]
    MalformedPayload(String),

    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),

    #[error("Serialization error: {0}")]
    Serialization(String),
}
