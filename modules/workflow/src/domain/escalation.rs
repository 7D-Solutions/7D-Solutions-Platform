//! Escalation timers — deterministic, exactly-once timer-driven escalations.
//!
//! Invariants:
//! - Only one active timer per (instance, rule) at a time.
//! - Firing a timer is atomic: Guard (unfired + due) → Mutation (set fired_at) → Outbox event.
//! - Idempotent: re-firing an already-fired timer is a no-op.
//! - Cancelled timers are never fired.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

pub use super::escalation_repo::EscalationRepo;

// ── Domain models ────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct EscalationRule {
    pub id: Uuid,
    pub tenant_id: String,
    pub definition_id: Uuid,
    pub step_id: String,
    pub timeout_seconds: i32,
    pub escalate_to_step: Option<String>,
    pub notify_actor_ids: Vec<Uuid>,
    pub notify_template: Option<String>,
    pub max_escalations: i32,
    pub is_active: bool,
    pub metadata: Option<serde_json::Value>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct EscalationTimer {
    pub id: Uuid,
    pub tenant_id: String,
    pub instance_id: Uuid,
    pub rule_id: Uuid,
    pub step_id: String,
    pub due_at: DateTime<Utc>,
    pub fired_at: Option<DateTime<Utc>>,
    pub cancelled_at: Option<DateTime<Utc>>,
    pub escalation_count: i32,
    pub idempotency_key: Option<String>,
    pub created_at: DateTime<Utc>,
}

// ── Request types ────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct CreateEscalationRuleRequest {
    pub tenant_id: String,
    pub definition_id: Uuid,
    pub step_id: String,
    pub timeout_seconds: i32,
    pub escalate_to_step: Option<String>,
    pub notify_actor_ids: Vec<Uuid>,
    pub notify_template: Option<String>,
    pub max_escalations: Option<i32>,
    pub metadata: Option<serde_json::Value>,
}

// ── Errors ───────────────────────────────────────────────────

#[derive(Debug, Error)]
pub enum EscalationError {
    #[error("Rule not found")]
    RuleNotFound,

    #[error("Timer not found")]
    TimerNotFound,

    #[error("Timer already fired")]
    AlreadyFired,

    #[error("Timer already cancelled")]
    AlreadyCancelled,

    #[error("Max escalations reached ({0})")]
    MaxEscalationsReached(i32),

    #[error("Validation error: {0}")]
    Validation(String),

    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),
}
