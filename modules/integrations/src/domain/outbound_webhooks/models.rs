//! Outbound webhook domain models.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

/// Outbound webhook subscription row.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct OutboundWebhook {
    pub id: Uuid,
    pub tenant_id: String,
    pub url: String,
    pub event_types: serde_json::Value,
    pub signing_secret_hash: String,
    pub status: String,
    pub idempotency_key: Option<String>,
    pub description: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub deleted_at: Option<DateTime<Utc>>,
}

/// Delivery attempt audit record.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct OutboundWebhookDelivery {
    pub id: Uuid,
    pub webhook_id: Uuid,
    pub tenant_id: String,
    pub event_type: String,
    pub payload: serde_json::Value,
    pub status_code: Option<i32>,
    pub response_body: Option<String>,
    pub error_message: Option<String>,
    pub attempt_number: i32,
    pub next_retry_at: Option<DateTime<Utc>>,
    pub delivered_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

// ============================================================================
// Request types
// ============================================================================

#[derive(Debug, Clone)]
pub struct CreateOutboundWebhookRequest {
    pub tenant_id: String,
    pub url: String,
    pub event_types: Vec<String>,
    pub description: Option<String>,
    pub idempotency_key: Option<String>,
}

#[derive(Debug, Clone)]
pub struct UpdateOutboundWebhookRequest {
    pub id: Uuid,
    pub tenant_id: String,
    pub url: Option<String>,
    pub event_types: Option<Vec<String>>,
    pub status: Option<String>,
    pub description: Option<String>,
}

#[derive(Debug, Clone)]
pub struct RecordDeliveryRequest {
    pub webhook_id: Uuid,
    pub tenant_id: String,
    pub event_type: String,
    pub payload: serde_json::Value,
    pub status_code: Option<i32>,
    pub response_body: Option<String>,
    pub error_message: Option<String>,
    pub attempt_number: i32,
    pub next_retry_at: Option<DateTime<Utc>>,
    pub delivered_at: Option<DateTime<Utc>>,
}

// ============================================================================
// Errors
// ============================================================================

#[derive(Debug, Error)]
pub enum OutboundWebhookError {
    #[error("Webhook not found")]
    NotFound,

    #[error("Validation failed: {0}")]
    Validation(String),

    #[error("Duplicate idempotency key: {0}")]
    DuplicateIdempotencyKey(String),

    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),
}
