//! Sync conflict detector — marker-correlation with orphaned-write recovery.
//!
//! `run_detector` correlates an incoming observation against push attempt result
//! markers.  Three outcomes are possible:
//!
//! 1. **SelfEchoSuppressed** — markers match a `succeeded` attempt; the
//!    observation is the echo of our own push, no conflict needed.
//! 2. **OrphanedWriteRecovered** — markers match a `failed`/`unknown_failure`
//!    attempt; QBO applied the write but our success transition did not
//!    complete.  The attempt is promoted to `succeeded` and no conflict is opened.
//! 3. **ConflictOpened** — no marker match; this is genuine external drift.
//!    A conflict row and a `sync.conflict.detected` event are written atomically.

use chrono::{DateTime, TimeZone, Utc};
use serde_json::Value;
use sqlx::PgPool;
use uuid::Uuid;

use super::conflicts::{ConflictClass, ConflictRow, MAX_VALUE_BYTES};
use super::push_attempts::{find_attempt_by_markers, promote_orphaned_write_tx};
use crate::events::sync_conflict_detected::{
    build_sync_conflict_detected_envelope, SyncConflictDetectedPayload,
    EVENT_TYPE_SYNC_CONFLICT_DETECTED,
};
use crate::outbox::enqueue_event_tx;

// ── Public outcome type ───────────────────────────────────────────────────────

#[derive(Debug)]
pub enum DetectorOutcome {
    /// Observation markers match a `succeeded` push attempt — self-echo suppressed.
    SelfEchoSuppressed { attempt_id: Uuid },
    /// Markers matched a `failed`/`unknown_failure` attempt — write was orphaned;
    /// attempt promoted to `succeeded` and conflict suppressed.
    OrphanedWriteRecovered { attempt_id: Uuid },
    /// No marker match found; observation is genuine external drift.
    /// Conflict row was created and `sync.conflict.detected` was enqueued.
    ConflictOpened(ConflictRow),
}

#[derive(Debug, thiserror::Error)]
pub enum DetectorError {
    #[error("conflict value blob exceeds 256 KB limit")]
    ValueTooLarge,
    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),
}

// ── Fingerprint parsing ───────────────────────────────────────────────────────

struct ObservationMarkers {
    sync_token: Option<String>,
    last_updated_time: Option<DateTime<Utc>>,
}

/// Extract sync_token / last_updated_time from an observation fingerprint.
///
/// Fingerprint formats (from `dedupe::compute_fingerprint`):
///   `st:<token>`   — sync token; maps to `result_sync_token`.
///   `ts:<epoch_ms>` — millisecond epoch; maps to `result_last_updated_time`.
///   `ph:<sha256>`  — payload hash; no direct marker match (use projection_hash arg).
fn parse_fingerprint(fingerprint: &str) -> ObservationMarkers {
    if let Some(token) = fingerprint.strip_prefix("st:") {
        return ObservationMarkers {
            sync_token: Some(token.to_string()),
            last_updated_time: None,
        };
    }
    if let Some(ms_str) = fingerprint.strip_prefix("ts:") {
        if let Ok(ms) = ms_str.parse::<i64>() {
            let ts = Utc.timestamp_millis_opt(ms).single();
            return ObservationMarkers {
                sync_token: None,
                last_updated_time: ts,
            };
        }
    }
    ObservationMarkers {
        sync_token: None,
        last_updated_time: None,
    }
}

// ── Public entry-point ────────────────────────────────────────────────────────

/// Run the sync conflict detector for an incoming observation.
///
/// # Parameters
/// - `fingerprint` — observation fingerprint (`st:`, `ts:`, or `ph:` prefixed).
/// - `comparable_hash` — SHA-256 of comparable fields + ms timestamp; compared
///   against `result_projection_hash` on push attempts (tertiary marker).
/// - `internal_value` / `external_value` — current platform and provider state
///   snapshots, used only if a new conflict row must be created.
pub async fn run_detector(
    pool: &PgPool,
    app_id: &str,
    provider: &str,
    entity_type: &str,
    entity_id: &str,
    fingerprint: &str,
    comparable_hash: &str,
    internal_value: Option<Value>,
    external_value: Option<Value>,
) -> Result<DetectorOutcome, DetectorError> {
    if let Some(v) = &internal_value {
        if v.to_string().len() > MAX_VALUE_BYTES {
            return Err(DetectorError::ValueTooLarge);
        }
    }
    if let Some(v) = &external_value {
        if v.to_string().len() > MAX_VALUE_BYTES {
            return Err(DetectorError::ValueTooLarge);
        }
    }

    let markers = parse_fingerprint(fingerprint);

    // comparable_hash is used as the projection_hash correlation key (tertiary).
    let ph = if comparable_hash.is_empty() {
        None
    } else {
        Some(comparable_hash)
    };

    let attempt = find_attempt_by_markers(
        pool,
        app_id,
        provider,
        entity_type,
        entity_id,
        markers.sync_token.as_deref(),
        markers.last_updated_time,
        ph,
    )
    .await?;

    if let Some(attempt) = attempt {
        match attempt.status.as_str() {
            "succeeded" => {
                return Ok(DetectorOutcome::SelfEchoSuppressed {
                    attempt_id: attempt.id,
                });
            }
            "failed" | "unknown_failure" => {
                // Orphaned write: QBO applied the write, our success transition
                // did not complete.  Promote to succeeded and suppress the conflict.
                let mut tx = pool.begin().await?;
                promote_orphaned_write_tx(&mut tx, attempt.id).await?;
                tx.commit().await?;
                return Ok(DetectorOutcome::OrphanedWriteRecovered {
                    attempt_id: attempt.id,
                });
            }
            _ => {
                // inflight / accepted / superseded — not terminal; fall through
                // to conflict creation.
            }
        }
    }

    // ── True drift: open conflict + emit event ────────────────────────────────
    //
    // Mirror post_call_reconcile semantics: Edit when both snapshots present,
    // Deletion otherwise.  Creation is reserved for upstream callers that
    // explicitly supply both a draft internal_value and a divergent external_value.
    let conflict_class = match (&internal_value, &external_value) {
        (Some(_), Some(_)) => ConflictClass::Edit,
        _ => ConflictClass::Deletion,
    };

    let event_id = Uuid::new_v4();

    let mut tx = pool.begin().await?;

    let conflict = sqlx::query_as::<_, ConflictRow>(
        r#"
        INSERT INTO integrations_sync_conflicts (
            app_id, provider, entity_type, entity_id,
            conflict_class, detected_by,
            internal_value, external_value
        )
        VALUES ($1, $2, $3, $4, $5, 'detector', $6, $7)
        RETURNING
            id, app_id, provider, entity_type, entity_id,
            conflict_class, status, detected_by, detected_at,
            internal_value, external_value, internal_id,
            resolved_by, resolved_at, resolution_note,
            created_at, updated_at
        "#,
    )
    .bind(app_id)
    .bind(provider)
    .bind(entity_type)
    .bind(entity_id)
    .bind(conflict_class.as_str())
    .bind(&internal_value)
    .bind(&external_value)
    .fetch_one(&mut *tx)
    .await?;

    let payload = SyncConflictDetectedPayload {
        app_id: app_id.to_string(),
        conflict_id: conflict.id,
        provider: provider.to_string(),
        entity_type: entity_type.to_string(),
        entity_id: entity_id.to_string(),
        conflict_class: conflict_class.as_str().to_string(),
        detected_by: "detector".to_string(),
    };
    let envelope = build_sync_conflict_detected_envelope(
        event_id,
        app_id.to_string(),
        event_id.to_string(),
        None,
        payload,
    );
    let _ = enqueue_event_tx(
        &mut tx,
        event_id,
        EVENT_TYPE_SYNC_CONFLICT_DETECTED,
        "sync_conflict",
        &conflict.id.to_string(),
        app_id,
        &envelope,
    )
    .await;

    tx.commit().await?;

    Ok(DetectorOutcome::ConflictOpened(conflict))
}
