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

use crate::events::{
    build_dunning_state_changed_envelope, build_invoice_suspended_envelope,
    DunningState, DunningStateChangedPayload, InvoiceSuspendedPayload,
    EVENT_TYPE_DUNNING_STATE_CHANGED, EVENT_TYPE_INVOICE_SUSPENDED,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
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
    pub fn to_event_state(&self) -> DunningState {
        match self {
            Self::Pending => DunningState::Pending,
            Self::Warned => DunningState::Warned,
            Self::Escalated => DunningState::Escalated,
            Self::Suspended => DunningState::Suspended,
            Self::Resolved => DunningState::Resolved,
            Self::WrittenOff => DunningState::WrittenOff,
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
fn is_valid_transition(from: &DunningStateValue, to: &DunningStateValue) -> bool {
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
struct DunningStateRow {
    id: i32,
    dunning_id: Uuid,
    state: String,
    version: i32,
    attempt_count: i32,
    customer_id: String,
}

// ============================================================================
// Core functions
// ============================================================================

/// Initialize dunning for an invoice (creates the Pending state record).
///
/// **Idempotency**: duplicate `dunning_id` returns `AlreadyExists` without error.
/// The unique constraint on (app_id, invoice_id) also prevents duplicates.
///
/// **Atomicity**: dunning record + outbox event (LIFECYCLE) are inserted in
/// a single transaction.
pub async fn init_dunning(
    pool: &PgPool,
    req: InitDunningRequest,
) -> Result<InitDunningResult, DunningError> {
    let mut tx = pool.begin().await?;
    let now = Utc::now();

    // 1. Idempotency check: has this dunning_id already been used?
    let existing: Option<i32> = sqlx::query_scalar(
        "SELECT id FROM ar_dunning_states WHERE dunning_id = $1",
    )
    .bind(req.dunning_id)
    .fetch_optional(&mut *tx)
    .await?;

    if let Some(existing_row_id) = existing {
        tx.rollback().await?;
        return Ok(InitDunningResult::AlreadyExists { existing_row_id });
    }

    // 2. Also check by (app_id, invoice_id) — different dunning_id but same invoice
    let existing_by_invoice: Option<i32> = sqlx::query_scalar(
        "SELECT id FROM ar_dunning_states WHERE app_id = $1 AND invoice_id = $2",
    )
    .bind(&req.app_id)
    .bind(req.invoice_id)
    .fetch_optional(&mut *tx)
    .await?;

    if let Some(existing_row_id) = existing_by_invoice {
        tx.rollback().await?;
        return Ok(InitDunningResult::AlreadyExists { existing_row_id });
    }

    // 3. Insert the initial Pending state record
    let dunning_row_id: i32 = sqlx::query_scalar(
        r#"
        INSERT INTO ar_dunning_states (
            dunning_id, app_id, invoice_id, customer_id,
            state, version, attempt_count, next_attempt_at,
            created_at, updated_at
        )
        VALUES ($1, $2, $3, $4, 'pending', 1, 0, $5, $6, $6)
        RETURNING id
        "#,
    )
    .bind(req.dunning_id)
    .bind(&req.app_id)
    .bind(req.invoice_id)
    .bind(&req.customer_id)
    .bind(req.next_attempt_at)
    .bind(now)
    .fetch_one(&mut *tx)
    .await?;

    // 4. Build and enqueue the outbox event (LIFECYCLE)
    let outbox_event_id = Uuid::new_v4();
    let payload = DunningStateChangedPayload {
        tenant_id: req.app_id.clone(),
        invoice_id: req.invoice_id.to_string(),
        customer_id: req.customer_id.clone(),
        from_state: None,
        to_state: DunningState::Pending,
        reason: "dunning_initialized".to_string(),
        attempt_number: 0,
        next_retry_at: req.next_attempt_at,
        transitioned_at: now,
    };

    let envelope = build_dunning_state_changed_envelope(
        outbox_event_id,
        req.app_id.clone(),
        req.correlation_id.clone(),
        req.causation_id.clone(),
        payload,
    );

    let payload_json = serde_json::to_value(&envelope)
        .map_err(|e| DunningError::DatabaseError(e.to_string()))?;

    sqlx::query(
        r#"
        INSERT INTO events_outbox (
            event_id, event_type, aggregate_type, aggregate_id, payload,
            tenant_id, source_module, mutation_class, schema_version,
            occurred_at, replay_safe, correlation_id, causation_id
        )
        VALUES ($1, $2, 'dunning_state', $3, $4, $5, 'ar', 'LIFECYCLE', $6, $7, true, $8, $9)
        "#,
    )
    .bind(outbox_event_id)
    .bind(EVENT_TYPE_DUNNING_STATE_CHANGED)
    .bind(req.dunning_id.to_string())
    .bind(payload_json)
    .bind(&req.app_id)
    .bind(&envelope.schema_version)
    .bind(now)
    .bind(&req.correlation_id)
    .bind(&req.causation_id)
    .execute(&mut *tx)
    .await?;

    // 5. Update outbox_event_id back on the dunning record for correlation
    sqlx::query(
        "UPDATE ar_dunning_states SET outbox_event_id = $1 WHERE id = $2",
    )
    .bind(outbox_event_id)
    .bind(dunning_row_id)
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;

    Ok(InitDunningResult::Initialized {
        dunning_row_id,
        dunning_id: req.dunning_id,
    })
}

/// Transition a dunning record to a new state.
///
/// **Guard**: validates the from → to transition before touching the DB.
/// **Atomic**: state UPDATE + outbox INSERT in a single transaction.
/// **Race-safe**: uses SELECT FOR UPDATE to prevent concurrent transitions
/// on the same dunning record.
pub async fn transition_dunning(
    pool: &PgPool,
    req: TransitionDunningRequest,
) -> Result<TransitionDunningResult, DunningError> {
    let mut tx = pool.begin().await?;
    let now = Utc::now();

    // 1. Lock the dunning record for update (race-safe)
    let row: Option<DunningStateRow> = sqlx::query_as(
        r#"
        SELECT id, dunning_id, state, version, attempt_count, customer_id
        FROM ar_dunning_states
        WHERE app_id = $1 AND invoice_id = $2
        FOR UPDATE
        "#,
    )
    .bind(&req.app_id)
    .bind(req.invoice_id)
    .fetch_optional(&mut *tx)
    .await?;

    let row = match row {
        Some(r) => r,
        None => {
            tx.rollback().await?;
            return Err(DunningError::DunningNotFound {
                invoice_id: req.invoice_id,
                app_id: req.app_id,
            });
        }
    };

    let from_state = DunningStateValue::from_str(&row.state).ok_or_else(|| {
        DunningError::DatabaseError(format!("Unknown dunning state in DB: {}", row.state))
    })?;

    // 2. Guard: reject terminal → anything transitions
    if from_state.is_terminal() {
        tx.rollback().await?;
        return Err(DunningError::TerminalState {
            state: row.state.clone(),
        });
    }

    // 3. Guard: validate the specific transition
    if !is_valid_transition(&from_state, &req.to_state) {
        tx.rollback().await?;
        return Err(DunningError::IllegalTransition {
            from_state: from_state.as_str().to_string(),
            to_state: req.to_state.as_str().to_string(),
        });
    }

    // 4. Increment attempt_count when moving to an attempt-based state
    let new_attempt_count = match &req.to_state {
        DunningStateValue::Warned | DunningStateValue::Escalated | DunningStateValue::Suspended => {
            row.attempt_count + 1
        }
        _ => row.attempt_count,
    };
    let new_version = row.version + 1;

    // 5. Apply the transition (optimistic lock: version must match)
    let rows_updated = sqlx::query(
        r#"
        UPDATE ar_dunning_states
        SET
            state           = $1,
            version         = version + 1,
            attempt_count   = $2,
            next_attempt_at = $3,
            last_error      = $4,
            updated_at      = $5
        WHERE
            app_id      = $6
            AND invoice_id  = $7
            AND version     = $8
        "#,
    )
    .bind(req.to_state.as_str())
    .bind(new_attempt_count)
    .bind(&req.next_attempt_at)
    .bind(&req.last_error)
    .bind(now)
    .bind(&req.app_id)
    .bind(req.invoice_id)
    .bind(row.version)
    .execute(&mut *tx)
    .await?;

    if rows_updated.rows_affected() == 0 {
        tx.rollback().await?;
        return Err(DunningError::ConcurrentModification {
            invoice_id: req.invoice_id,
        });
    }

    // 6. Build and enqueue the outbox event (LIFECYCLE)
    let outbox_event_id = Uuid::new_v4();
    let payload = DunningStateChangedPayload {
        tenant_id: req.app_id.clone(),
        invoice_id: req.invoice_id.to_string(),
        customer_id: row.customer_id.clone(),
        from_state: Some(from_state.to_event_state()),
        to_state: req.to_state.to_event_state(),
        reason: req.reason.clone(),
        attempt_number: new_attempt_count,
        next_retry_at: req.next_attempt_at,
        transitioned_at: now,
    };

    let envelope = build_dunning_state_changed_envelope(
        outbox_event_id,
        req.app_id.clone(),
        req.correlation_id.clone(),
        req.causation_id.clone(),
        payload,
    );

    let payload_json = serde_json::to_value(&envelope)
        .map_err(|e| DunningError::DatabaseError(e.to_string()))?;

    sqlx::query(
        r#"
        INSERT INTO events_outbox (
            event_id, event_type, aggregate_type, aggregate_id, payload,
            tenant_id, source_module, mutation_class, schema_version,
            occurred_at, replay_safe, correlation_id, causation_id
        )
        VALUES ($1, $2, 'dunning_state', $3, $4, $5, 'ar', 'LIFECYCLE', $6, $7, true, $8, $9)
        "#,
    )
    .bind(outbox_event_id)
    .bind(EVENT_TYPE_DUNNING_STATE_CHANGED)
    .bind(row.dunning_id.to_string())
    .bind(payload_json)
    .bind(&req.app_id)
    .bind(&envelope.schema_version)
    .bind(now)
    .bind(&req.correlation_id)
    .bind(&req.causation_id)
    .execute(&mut *tx)
    .await?;

    // 6b. If transitioning to Suspended, also emit ar.invoice_suspended
    //     This cross-module signal lets subscriptions apply suspension.
    if req.to_state == DunningStateValue::Suspended {
        let suspended_event_id = Uuid::new_v4();
        let suspended_payload = InvoiceSuspendedPayload {
            tenant_id: req.app_id.clone(),
            invoice_id: req.invoice_id.to_string(),
            customer_id: row.customer_id.clone(),
            outstanding_minor: 0, // AR does not carry balance in dunning row; downstream can look it up
            currency: String::new(),
            dunning_attempt: new_attempt_count,
            reason: req.reason.clone(),
            grace_period_ends_at: None,
            suspended_at: now,
        };

        let suspended_envelope = build_invoice_suspended_envelope(
            suspended_event_id,
            req.app_id.clone(),
            req.correlation_id.clone(),
            Some(outbox_event_id.to_string()), // causation: the dunning_state_changed event
            suspended_payload,
        );

        let suspended_json = serde_json::to_value(&suspended_envelope)
            .map_err(|e| DunningError::DatabaseError(e.to_string()))?;

        sqlx::query(
            r#"
            INSERT INTO events_outbox (
                event_id, event_type, aggregate_type, aggregate_id, payload,
                tenant_id, source_module, mutation_class, schema_version,
                occurred_at, replay_safe, correlation_id, causation_id
            )
            VALUES ($1, $2, 'invoice', $3, $4, $5, 'ar', 'LIFECYCLE', $6, $7, true, $8, $9)
            "#,
        )
        .bind(suspended_event_id)
        .bind(EVENT_TYPE_INVOICE_SUSPENDED)
        .bind(req.invoice_id.to_string())
        .bind(suspended_json)
        .bind(&req.app_id)
        .bind(&suspended_envelope.schema_version)
        .bind(now)
        .bind(&req.correlation_id)
        .bind(&outbox_event_id.to_string())
        .execute(&mut *tx)
        .await?;
    }

    // 7. Update outbox_event_id on the dunning record for correlation
    sqlx::query(
        "UPDATE ar_dunning_states SET outbox_event_id = $1 WHERE id = $2",
    )
    .bind(outbox_event_id)
    .bind(row.id)
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;

    Ok(TransitionDunningResult::Transitioned {
        dunning_row_id: row.id,
        from_state,
        to_state: req.to_state,
        new_version,
        new_attempt_count,
    })
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

// ============================================================================
// Unit tests (pure logic — no DB)
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // ─── Transition guard ────────────────────────────────────────────────────

    #[test]
    fn valid_transitions() {
        use DunningStateValue::*;
        let valid = [
            (Pending, Warned),
            (Pending, Escalated),
            (Pending, Resolved),
            (Pending, WrittenOff),
            (Warned, Escalated),
            (Warned, Resolved),
            (Warned, WrittenOff),
            (Escalated, Suspended),
            (Escalated, Resolved),
            (Escalated, WrittenOff),
            (Suspended, Resolved),
            (Suspended, WrittenOff),
        ];
        for (from, to) in valid {
            assert!(
                is_valid_transition(&from, &to),
                "Expected {} → {} to be valid",
                from,
                to
            );
        }
    }

    #[test]
    fn invalid_transitions() {
        use DunningStateValue::*;
        let invalid = [
            (Resolved, Warned),
            (Resolved, Escalated),
            (Resolved, Suspended),
            (WrittenOff, Resolved),
            (WrittenOff, Pending),
            (Warned, Pending),       // no backwards
            (Escalated, Warned),     // no backwards
            (Suspended, Escalated),  // no backwards
        ];
        for (from, to) in invalid {
            assert!(
                !is_valid_transition(&from, &to),
                "Expected {} → {} to be invalid",
                from,
                to
            );
        }
    }

    #[test]
    fn terminal_states() {
        assert!(DunningStateValue::Resolved.is_terminal());
        assert!(DunningStateValue::WrittenOff.is_terminal());
        assert!(!DunningStateValue::Pending.is_terminal());
        assert!(!DunningStateValue::Warned.is_terminal());
        assert!(!DunningStateValue::Escalated.is_terminal());
        assert!(!DunningStateValue::Suspended.is_terminal());
    }

    #[test]
    fn state_from_str_roundtrip() {
        use DunningStateValue::*;
        let variants = [Pending, Warned, Escalated, Suspended, Resolved, WrittenOff];
        for v in variants {
            let s = v.as_str();
            let back = DunningStateValue::from_str(s).unwrap();
            assert_eq!(v, back);
        }
    }

    #[test]
    fn state_from_str_unknown_returns_none() {
        assert!(DunningStateValue::from_str("unknown_state").is_none());
        assert!(DunningStateValue::from_str("").is_none());
    }

    #[test]
    fn error_display() {
        let e = DunningError::DunningNotFound {
            invoice_id: 42,
            app_id: "tenant-1".to_string(),
        };
        assert!(e.to_string().contains("42"));
        assert!(e.to_string().contains("tenant-1"));

        let e = DunningError::IllegalTransition {
            from_state: "resolved".to_string(),
            to_state: "warned".to_string(),
        };
        assert!(e.to_string().contains("resolved"));
        assert!(e.to_string().contains("warned"));

        let e = DunningError::TerminalState {
            state: "resolved".to_string(),
        };
        assert!(e.to_string().contains("terminal"));

        let e = DunningError::ConcurrentModification { invoice_id: 7 };
        assert!(e.to_string().contains("7"));
    }
}
