//! EDI transaction set domain models.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

// ============================================================================
// Validation statuses
// ============================================================================

/// Inbound initial state.
pub const STATUS_INGESTED: &str = "ingested";
/// Outbound initial state.
pub const STATUS_CREATED: &str = "created";
/// After successful parse.
pub const STATUS_PARSED: &str = "parsed";
/// After successful validation.
pub const STATUS_VALIDATED: &str = "validated";
/// Inbound terminal success.
pub const STATUS_ACCEPTED: &str = "accepted";
/// Terminal failure (inbound or outbound).
pub const STATUS_REJECTED: &str = "rejected";
/// Outbound terminal success.
pub const STATUS_EMITTED: &str = "emitted";

// ============================================================================
// Directions
// ============================================================================

pub const DIRECTION_INBOUND: &str = "inbound";
pub const DIRECTION_OUTBOUND: &str = "outbound";

// ============================================================================
// Core model
// ============================================================================

/// A durable EDI transaction record.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct EdiTransaction {
    pub id: Uuid,
    pub tenant_id: String,
    pub transaction_type: String,
    pub version: String,
    pub direction: String,
    pub raw_payload: Option<String>,
    pub parsed_payload: Option<serde_json::Value>,
    pub validation_status: String,
    pub error_details: Option<String>,
    pub idempotency_key: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

// ============================================================================
// Requests
// ============================================================================

/// Ingest a new inbound EDI document.
#[derive(Debug, Clone)]
pub struct IngestEdiRequest {
    pub tenant_id: String,
    pub transaction_type: String,
    pub version: String,
    pub raw_payload: String,
    pub idempotency_key: Option<String>,
}

/// Create a new outbound EDI record.
#[derive(Debug, Clone)]
pub struct CreateOutboundEdiRequest {
    pub tenant_id: String,
    pub transaction_type: String,
    pub version: String,
    pub parsed_payload: serde_json::Value,
    pub idempotency_key: Option<String>,
}

/// Transition an EDI transaction to a new pipeline status.
#[derive(Debug, Clone)]
pub struct TransitionEdiRequest {
    pub transaction_id: Uuid,
    pub tenant_id: String,
    pub new_status: String,
    pub parsed_payload: Option<serde_json::Value>,
    pub error_details: Option<String>,
}

// ============================================================================
// Errors
// ============================================================================

#[derive(Debug, Error)]
pub enum EdiTransactionError {
    #[error("EDI transaction not found")]
    NotFound,

    #[error("Validation failed: {0}")]
    Validation(String),

    #[error("Invalid status transition from '{from}' to '{to}'")]
    InvalidTransition { from: String, to: String },

    #[error("Duplicate idempotency key")]
    DuplicateIdempotencyKey,

    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),
}
