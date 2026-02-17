//! AR Dunning State Machine (bd-1rr)
//!
//! Dunning is a deterministic state machine for collecting overdue invoices.
//!
//! ## States
//!
//! ```text
//! [Pending] ──attempt──> [Warned] ──attempt──> [Escalated]
//!     |                     |                       |
//!     └──paid──> [Resolved] ◄──────────────────────┘
//!     |                                             |
//!     └──writeoff──> [WrittenOff]  <────────────────┘
//!                                                   |
//!                                             [Suspended]
//! ```
//!
//! ## Invariants
//!
//! - **Guard → Mutate → Emit**: transitions are validated before touching DB.
//! - **Atomic**: state update + outbox event in a single transaction.
//! - **Idempotent**: `dunning_id` is the idempotency key — duplicate transitions
//!   with the same `dunning_id` return `AlreadyProcessed` without side effects.
//! - **Race-safe**: optimistic locking via `version` prevents concurrent double-transition.
//! - **Terminal**: `Resolved` and `WrittenOff` reject further transitions.
//!
//! ## Transaction Pattern
//!
//! ```text
//! BEGIN
//!   SELECT FOR UPDATE ar_dunning_states WHERE invoice_id = $invoice_id AND app_id = $app_id
//!   Guard: validate transition (from_state → to_state)
//!   UPDATE ar_dunning_states SET state = $new, version = version + 1, ...
//!   INSERT INTO events_outbox (event_type = 'ar.dunning_state_changed', mutation_class = 'LIFECYCLE')
//! COMMIT
//! ```

mod engine;
#[cfg(test)]
mod tests;

// Re-export engine functions so callers see them at crate::dunning::*
pub use engine::{init_dunning, transition_dunning};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fmt;
use uuid::Uuid;

// ============================================================================
// State enum
// ============================================================================

/// Internal dunning state (mirrors the DB `state` column and the events contract).
///
/// These states are a subset of the events::DunningState enum — they share
/// serialization names so they can be converted infallibly.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum DunningStateValue {
    Pending,
    Warned,
    Escalated,
    Suspended,
    Resolved,
    WrittenOff,
}

impl DunningStateValue {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Warned => "warned",
            Self::Escalated => "escalated",
            Self::Suspended => "suspended",
            Self::Resolved => "resolved",
            Self::WrittenOff => "written_off",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "pending" => Some(Self::Pending),
            "warned" => Some(Self::Warned),
            "escalated" => Some(Self::Escalated),
            "suspended" => Some(Self::Suspended),
            "resolved" => Some(Self::Resolved),
            "written_off" => Some(Self::WrittenOff),
            _ => None,
        }
    }

    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Resolved | Self::WrittenOff)
    }

    /// Convert to the events contract enum
    pub fn to_event_state(&self) -> crate::events::DunningState {
        match self {
            Self::Pending => crate::events::DunningState::Pending,
            Self::Warned => crate::events::DunningState::Warned,
            Self::Escalated => crate::events::DunningState::Escalated,
            Self::Suspended => crate::events::DunningState::Suspended,
            Self::Resolved => crate::events::DunningState::Resolved,
            Self::WrittenOff => crate::events::DunningState::WrittenOff,
        }
    }
}

impl fmt::Display for DunningStateValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

// ============================================================================
// Allowed transitions
// ============================================================================

/// Validate that `from → to` is a legal state machine transition.
///
/// This is the **guard** — pure logic, zero I/O.
pub(crate) fn is_valid_transition(from: &DunningStateValue, to: &DunningStateValue) -> bool {
    use DunningStateValue::*;
    match (from, to) {
        // Progression through dunning stages
        (Pending, Warned) => true,
        (Pending, Escalated) => true,  // skip Warned for aggressive escalation
        (Warned, Escalated) => true,
        (Escalated, Suspended) => true,
        // Resolution from any non-terminal state
        (Pending, Resolved) => true,
        (Warned, Resolved) => true,
        (Escalated, Resolved) => true,
        (Suspended, Resolved) => true,
        // Write-off from any non-terminal state
        (Pending, WrittenOff) => true,
        (Warned, WrittenOff) => true,
        (Escalated, WrittenOff) => true,
        (Suspended, WrittenOff) => true,
        // Terminal states block further transitions
        (Resolved, _) | (WrittenOff, _) => false,
        // All other transitions are illegal
        _ => false,
    }
}

// ============================================================================
// Request / Response types
// ============================================================================

/// Request to initialize dunning for an invoice (creates the initial Pending record).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InitDunningRequest {
    /// Stable business key for this dunning sequence (idempotency anchor).
    /// Deterministic from (app_id, invoice_id) + a timestamp anchor.
    pub dunning_id: Uuid,
    /// Tenant identifier
    pub app_id: String,
    /// Internal invoice ID
    pub invoice_id: i32,
    /// Customer identifier (for denormalized lookups)
    pub customer_id: String,
    /// When to first attempt collection
    pub next_attempt_at: Option<DateTime<Utc>>,
    /// Distributed trace correlation ID
    pub correlation_id: String,
    /// Causation ID (event/action that triggered dunning initiation)
    pub causation_id: Option<String>,
}

/// Request to transition a dunning record to a new state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransitionDunningRequest {
    /// Tenant identifier
    pub app_id: String,
    /// Internal invoice ID (the dunning record key)
    pub invoice_id: i32,
    /// Target state (where the machine should transition to)
    pub to_state: DunningStateValue,
    /// Human-readable reason for this transition
    pub reason: String,
    /// When to schedule the next attempt (None = terminal or caller-decides)
    pub next_attempt_at: Option<DateTime<Utc>>,
    /// Error from the last attempt (if transitioning due to failure)
    pub last_error: Option<String>,
    /// Distributed trace correlation ID
    pub correlation_id: String,
    /// Causation ID (event/action that triggered this transition)
    pub causation_id: Option<String>,
}

/// Result of initializing dunning
#[derive(Debug, Clone)]
pub enum InitDunningResult {
    /// Dunning initialized — record created, outbox event enqueued
    Initialized {
        dunning_row_id: i32,
        dunning_id: Uuid,
    },
    /// A dunning record already exists for this invoice (idempotency)
    AlreadyExists { existing_row_id: i32 },
}

/// Result of transitioning dunning state
#[derive(Debug, Clone)]
pub enum TransitionDunningResult {
    /// Transition applied — state updated, outbox event enqueued
    Transitioned {
        dunning_row_id: i32,
        from_state: DunningStateValue,
        to_state: DunningStateValue,
        new_version: i32,
        new_attempt_count: i32,
    },
}

// ============================================================================
// Error types
// ============================================================================

#[derive(Debug)]
pub enum DunningError {
    /// No dunning record found for this invoice
    DunningNotFound { invoice_id: i32, app_id: String },
    /// The requested transition is not valid from the current state
    IllegalTransition {
        from_state: String,
        to_state: String,
    },
    /// The record is in a terminal state — no further transitions
    TerminalState { state: String },
    /// Concurrent modification detected (optimistic lock failure)
    ConcurrentModification { invoice_id: i32 },
    /// Database error
    DatabaseError(String),
}

impl fmt::Display for DunningError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DunningNotFound { invoice_id, app_id } => {
                write!(f, "Dunning record not found for invoice {} (tenant {})", invoice_id, app_id)
            }
            Self::IllegalTransition { from_state, to_state } => {
                write!(f, "Illegal dunning transition: {} → {}", from_state, to_state)
            }
            Self::TerminalState { state } => {
                write!(f, "Dunning is in terminal state '{}' — no further transitions", state)
            }
            Self::ConcurrentModification { invoice_id } => {
                write!(f, "Concurrent modification for invoice {} — retry", invoice_id)
            }
            Self::DatabaseError(msg) => write!(f, "Database error: {}", msg),
        }
    }
}

impl std::error::Error for DunningError {}

impl From<sqlx::Error> for DunningError {
    fn from(e: sqlx::Error) -> Self {
        Self::DatabaseError(e.to_string())
    }
}

// ============================================================================
// Row type (for SELECT queries)
// ============================================================================

#[derive(Debug)]
pub(crate) struct DunningStateRow {
    pub(crate) id: i32,
    pub(crate) dunning_id: Uuid,
    pub(crate) state: String,
    pub(crate) version: i32,
    pub(crate) attempt_count: i32,
    pub(crate) customer_id: String,
}

// ============================================================================
// SQLx row mapping
// ============================================================================

impl<'r> sqlx::FromRow<'r, sqlx::postgres::PgRow> for DunningStateRow {
    fn from_row(row: &'r sqlx::postgres::PgRow) -> Result<Self, sqlx::Error> {
        use sqlx::Row;
        Ok(Self {
            id: row.try_get("id")?,
            dunning_id: row.try_get("dunning_id")?,
            state: row.try_get("state")?,
            version: row.try_get("version")?,
            attempt_count: row.try_get("attempt_count")?,
            customer_id: row.try_get("customer_id")?,
        })
    }
}
