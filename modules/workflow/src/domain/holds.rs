//! Hold/release primitive — embeddable by any service.
//!
//! Invariants:
//! - Only one active hold per (tenant, entity_type, entity_id, hold_type).
//! - Idempotent apply: duplicate idempotency_key returns existing hold.
//! - Idempotent release: releasing an already-released hold is a no-op.
//! - Guard→Mutation→Outbox for every apply and release.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

pub use super::holds_repo::HoldRepo;

// ── Domain model ──────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Hold {
    pub id: Uuid,
    pub tenant_id: String,
    pub entity_type: String,
    pub entity_id: String,
    pub hold_type: String,
    pub reason: Option<String>,
    pub applied_by: Option<Uuid>,
    pub applied_at: DateTime<Utc>,
    pub released_by: Option<Uuid>,
    pub released_at: Option<DateTime<Utc>>,
    pub release_reason: Option<String>,
    pub idempotency_key: Option<String>,
    pub metadata: Option<serde_json::Value>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

// ── Request types ─────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct ApplyHoldRequest {
    pub tenant_id: String,
    pub entity_type: String,
    pub entity_id: String,
    pub hold_type: String,
    pub reason: Option<String>,
    pub applied_by: Option<Uuid>,
    pub metadata: Option<serde_json::Value>,
    pub idempotency_key: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ReleaseHoldRequest {
    pub tenant_id: String,
    pub released_by: Option<Uuid>,
    pub release_reason: Option<String>,
    pub idempotency_key: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ListHoldsQuery {
    pub tenant_id: String,
    pub entity_type: Option<String>,
    pub entity_id: Option<String>,
    pub hold_type: Option<String>,
    pub active_only: Option<bool>,
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

// ── Errors ────────────────────────────────────────────────────

#[derive(Debug, Error)]
pub enum HoldError {
    #[error("Hold not found")]
    NotFound,

    #[error("Active hold already exists for this entity and hold type")]
    AlreadyHeld,

    #[error("Hold is already released")]
    AlreadyReleased,

    #[error("Validation error: {0}")]
    Validation(String),

    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),
}
