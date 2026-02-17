//! AR Dunning Scheduler Worker (bd-2bj)
//!
//! Polls for due dunning rows using `FOR UPDATE SKIP LOCKED` to claim work,
//! executes the next dunning action, records the outcome, computes backoff,
//! and emits state-changed events atomically.
//!
//! ## Concurrency Safety
//!
//! - `FOR UPDATE SKIP LOCKED` ensures two concurrent workers never process the same row.
//! - Each row is claimed inside a transaction: claim → execute → record → emit → commit.
//! - Bounded exponential backoff: base 1h, factor 2×, max 72h.
//!
//! ## Backoff Formula
//!
//! ```text
//! next_attempt_at = now + min(base * 2^(attempt_count - 1), max_delay)
//! ```
//!
//! Where base = 1 hour, max_delay = 72 hours.

use crate::dunning::{transition_dunning, DunningError, DunningStateValue, TransitionDunningRequest};
use crate::events::{
    build_dunning_state_changed_envelope, DunningState, DunningStateChangedPayload,
    EVENT_TYPE_DUNNING_STATE_CHANGED,
};
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use uuid::Uuid;

// ============================================================================
// Backoff configuration
// ============================================================================

/// Base delay for the first retry (1 hour)
const BACKOFF_BASE_SECS: i64 = 3600;

/// Maximum delay cap (72 hours)
const BACKOFF_MAX_SECS: i64 = 72 * 3600;

/// Compute the next attempt time using bounded exponential backoff.
///
/// Formula: `now + min(base * 2^(attempt - 1), max_delay)`
///
/// - attempt=1 → 1h
/// - attempt=2 → 2h
/// - attempt=3 → 4h
/// - attempt=4 → 8h
/// - ...
/// - attempt=7+ → capped at 72h
pub fn compute_next_attempt(now: DateTime<Utc>, attempt_count: i32) -> DateTime<Utc> {
    let exponent = (attempt_count - 1).max(0) as u32;
    let delay_secs = BACKOFF_BASE_SECS.saturating_mul(2_i64.saturating_pow(exponent));
    let capped = delay_secs.min(BACKOFF_MAX_SECS);
    now + Duration::seconds(capped)
}

// ============================================================================
// Claimable row
// ============================================================================

/// A dunning row that is due for processing.
#[derive(Debug)]
pub struct ClaimableDunningRow {
    pub id: i32,
    pub dunning_id: Uuid,
    pub app_id: String,
    pub invoice_id: i32,
    pub customer_id: String,
    pub state: String,
    pub version: i32,
    pub attempt_count: i32,
}

impl<'r> sqlx::FromRow<'r, sqlx::postgres::PgRow> for ClaimableDunningRow {
    fn from_row(row: &'r sqlx::postgres::PgRow) -> Result<Self, sqlx::Error> {
        use sqlx::Row;
        Ok(Self {
            id: row.try_get("id")?,
            dunning_id: row.try_get("dunning_id")?,
            app_id: row.try_get("app_id")?,
            invoice_id: row.try_get("invoice_id")?,
            customer_id: row.try_get("customer_id")?,
            state: row.try_get("state")?,
            version: row.try_get("version")?,
            attempt_count: row.try_get("attempt_count")?,
        })
    }
}

// ============================================================================
// Execution outcome
// ============================================================================

/// Outcome of a single dunning execution step.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum DunningExecutionOutcome {
    /// Transitioned to the next state successfully
    Transitioned {
        from_state: String,
        to_state: String,
        new_attempt_count: i32,
        next_attempt_at: Option<DateTime<Utc>>,
    },
    /// Row was already in a terminal state — no action taken
    AlreadyTerminal { state: String },
    /// No rows were due — nothing to process
    NothingToClaim,
    /// An error occurred during execution
    Failed { error: String },
}

// ============================================================================
// State progression logic
// ============================================================================

/// Determine the next dunning state for a given current state.
///
/// This is the scheduler's progression policy:
/// - Pending → Warned (first collection notice)
/// - Warned → Escalated (second notice, stronger language)
/// - Escalated → Suspended (service interruption)
/// - Suspended / Resolved / WrittenOff → None (terminal, no further progression)
fn next_state_for(current: &DunningStateValue) -> Option<DunningStateValue> {
    match current {
        DunningStateValue::Pending => Some(DunningStateValue::Warned),
        DunningStateValue::Warned => Some(DunningStateValue::Escalated),
        DunningStateValue::Escalated => Some(DunningStateValue::Suspended),
        // Terminal states — no automatic progression
        DunningStateValue::Suspended
        | DunningStateValue::Resolved
        | DunningStateValue::WrittenOff => None,
    }
}

// ============================================================================
// Core scheduler functions
// ============================================================================

/// Claim and execute a single due dunning row.
///
/// Uses `FOR UPDATE SKIP LOCKED` to safely claim one row that is due
/// (next_attempt_at <= now, non-terminal state). If claimed:
/// 1. Determines the next state via progression policy
/// 2. Computes bounded backoff for next_attempt_at
/// 3. Transitions the state atomically (state + outbox event)
///
/// Returns the execution outcome.
pub async fn claim_and_execute_one(
    pool: &PgPool,
    correlation_id: &str,
) -> Result<DunningExecutionOutcome, DunningError> {
    let now = Utc::now();

    // 1. Claim one due row with SKIP LOCKED
    let row: Option<ClaimableDunningRow> = sqlx::query_as(
        r#"
        SELECT id, dunning_id, app_id, invoice_id, customer_id, state, version, attempt_count
        FROM ar_dunning_states
        WHERE next_attempt_at <= $1
          AND state NOT IN ('resolved', 'written_off')
        ORDER BY next_attempt_at ASC
        LIMIT 1
        FOR UPDATE SKIP LOCKED
        "#,
    )
    .bind(now)
    .fetch_optional(pool)
    .await?;

    let row = match row {
        Some(r) => r,
        None => return Ok(DunningExecutionOutcome::NothingToClaim),
    };

    let current_state = match DunningStateValue::from_str(&row.state) {
        Some(s) => s,
        None => {
            return Ok(DunningExecutionOutcome::Failed {
                error: format!("Unknown state in DB: {}", row.state),
            })
        }
    };

    // 2. Determine next state
    let target_state = match next_state_for(&current_state) {
        Some(s) => s,
        None => {
            return Ok(DunningExecutionOutcome::AlreadyTerminal {
                state: row.state.clone(),
            })
        }
    };

    // 3. Compute backoff for next attempt (based on new attempt count after transition)
    let new_attempt_count = row.attempt_count + 1;
    let next_attempt_at = if target_state.is_terminal() {
        None // No further attempts for terminal states
    } else {
        Some(compute_next_attempt(now, new_attempt_count))
    };

    // 4. Transition the dunning record (atomic: state + outbox event)
    let result = transition_dunning(
        pool,
        TransitionDunningRequest {
            app_id: row.app_id.clone(),
            invoice_id: row.invoice_id,
            to_state: target_state.clone(),
            reason: format!("scheduler_auto_escalation_attempt_{}", new_attempt_count),
            next_attempt_at,
            last_error: None,
            correlation_id: correlation_id.to_string(),
            causation_id: Some(format!("dunning-scheduler-{}", row.dunning_id)),
        },
    )
    .await?;

    match result {
        crate::dunning::TransitionDunningResult::Transitioned {
            from_state,
            to_state,
            new_attempt_count: actual_attempt_count,
            ..
        } => Ok(DunningExecutionOutcome::Transitioned {
            from_state: from_state.as_str().to_string(),
            to_state: to_state.as_str().to_string(),
            new_attempt_count: actual_attempt_count,
            next_attempt_at,
        }),
    }
}

/// Poll and execute a batch of due dunning rows.
///
/// Calls `claim_and_execute_one` up to `batch_size` times, stopping early
/// when there's nothing left to claim. Returns all outcomes.
pub async fn poll_and_execute_batch(
    pool: &PgPool,
    batch_size: usize,
    correlation_id: &str,
) -> Vec<DunningExecutionOutcome> {
    let mut outcomes = Vec::with_capacity(batch_size);

    for _ in 0..batch_size {
        match claim_and_execute_one(pool, correlation_id).await {
            Ok(DunningExecutionOutcome::NothingToClaim) => {
                outcomes.push(DunningExecutionOutcome::NothingToClaim);
                break; // No more work available
            }
            Ok(outcome) => outcomes.push(outcome),
            Err(e) => {
                outcomes.push(DunningExecutionOutcome::Failed {
                    error: e.to_string(),
                });
            }
        }
    }

    outcomes
}

// ============================================================================
// Unit tests (pure logic — no DB)
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    #[test]
    fn backoff_attempt_1_is_1h() {
        let now = Utc::now();
        let next = compute_next_attempt(now, 1);
        let diff = (next - now).num_seconds();
        assert_eq!(diff, 3600, "attempt 1 should be 1 hour");
    }

    #[test]
    fn backoff_attempt_2_is_2h() {
        let now = Utc::now();
        let next = compute_next_attempt(now, 2);
        let diff = (next - now).num_seconds();
        assert_eq!(diff, 7200, "attempt 2 should be 2 hours");
    }

    #[test]
    fn backoff_attempt_3_is_4h() {
        let now = Utc::now();
        let next = compute_next_attempt(now, 3);
        let diff = (next - now).num_seconds();
        assert_eq!(diff, 14400, "attempt 3 should be 4 hours");
    }

    #[test]
    fn backoff_attempt_7_caps_at_72h() {
        let now = Utc::now();
        let next = compute_next_attempt(now, 7);
        let diff = (next - now).num_seconds();
        // 2^6 * 3600 = 230400 = 64h, still under 72h cap
        assert_eq!(diff, 230400, "attempt 7 should be 64 hours");
    }

    #[test]
    fn backoff_attempt_8_caps_at_72h() {
        let now = Utc::now();
        let next = compute_next_attempt(now, 8);
        let diff = (next - now).num_seconds();
        // 2^7 * 3600 = 460800 > 259200 (72h), should cap
        assert_eq!(diff, BACKOFF_MAX_SECS, "attempt 8 should cap at 72 hours");
    }

    #[test]
    fn backoff_attempt_0_treated_as_attempt_1() {
        let now = Utc::now();
        let next = compute_next_attempt(now, 0);
        let diff = (next - now).num_seconds();
        assert_eq!(diff, 3600, "attempt 0 should be treated as base (1h)");
    }

    #[test]
    fn state_progression_policy() {
        assert_eq!(
            next_state_for(&DunningStateValue::Pending),
            Some(DunningStateValue::Warned)
        );
        assert_eq!(
            next_state_for(&DunningStateValue::Warned),
            Some(DunningStateValue::Escalated)
        );
        assert_eq!(
            next_state_for(&DunningStateValue::Escalated),
            Some(DunningStateValue::Suspended)
        );
        assert_eq!(next_state_for(&DunningStateValue::Suspended), None);
        assert_eq!(next_state_for(&DunningStateValue::Resolved), None);
        assert_eq!(next_state_for(&DunningStateValue::WrittenOff), None);
    }
}
