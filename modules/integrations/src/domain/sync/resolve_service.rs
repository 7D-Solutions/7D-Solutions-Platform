//! Resolve service: orchestrates sync push operations against QBO.
//!
//! Receives a push request, runs the authority-guarded state machine
//! (accepted → pre-call → inflight → terminal), and returns one of the
//! `PushOutcome` taxonomy variants.  Per-entity routing is explicit —
//! no trait dispatch.

use std::future::Future;
use std::sync::Arc;

use chrono::Utc;
use serde::Serialize;
use serde_json::Value;
use sqlx::PgPool;
use uuid::Uuid;

use super::authority_repo;
use super::conflicts::{ConflictError, ConflictStatus};
use super::conflicts_repo::{
    close_conflict_with_key, get_conflict, resolve_conflict_tx, resolve_conflict_with_key_tx,
};
use super::dedupe::compute_resolve_det_key;
use super::dedupe::{compute_fingerprint, truncate_to_millis};
use super::push_attempts::{self, PreCallOutcome, ReconcileOutcome};
use crate::domain::oauth::repo as oauth_repo;
use crate::domain::qbo::client::{
    QboClient, QboCustomerPayload, QboInvoicePayload, QboPaymentPayload,
};
use crate::domain::qbo::QboError;
use crate::events::{
    build_sync_conflict_resolved_envelope, build_sync_push_failed_envelope,
    SyncConflictResolvedPayload, SyncPushFailedPayload, EVENT_TYPE_SYNC_CONFLICT_RESOLVED,
    EVENT_TYPE_SYNC_PUSH_FAILED,
};
use crate::outbox::enqueue_event_tx;

// ── Public outcome taxonomy ───────────────────────────────────────────────────

/// All possible results of a synchronous push operation.
///
/// Serialises with `"outcome"` as the discriminant tag so callers can
/// always branch on a single field in the JSON body.
#[derive(Debug, Serialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum PushOutcome {
    /// External write confirmed; the provider accepted the entity.
    Succeeded {
        attempt_id: Uuid,
        entity_id: String,
        provider_entity_id: Option<String>,
    },
    /// External write rejected; the provider returned a classifiable error.
    Failed {
        attempt_id: Uuid,
        entity_id: String,
        error_code: String,
        error_message: String,
    },
    /// Network or parse error; write completion state is unknown.
    UnknownFailure {
        attempt_id: Uuid,
        entity_id: String,
        error_message: String,
    },
    /// Authority version advanced before dispatch; no write was sent.
    Superseded {
        attempt_id: Uuid,
        entity_id: String,
        current_authority_version: i64,
    },
    /// Write completed under stale authority; values were equal, no conflict
    /// was opened.
    StaleAuthorityAutoClosed { attempt_id: Uuid, entity_id: String },
    /// Write completed under stale authority; values diverged, a conflict row
    /// was opened for manual resolution.
    StaleAuthorityConflictOpened {
        attempt_id: Uuid,
        entity_id: String,
        conflict_id: Uuid,
    },
}

// ── Conflict resolve error ────────────────────────────────────────────────────

#[derive(Debug, thiserror::Error)]
pub enum ResolveConflictError {
    #[error("conflict not found: {0}")]
    NotFound(Uuid),
    #[error("invalid status transition: {0} → {1}")]
    InvalidTransition(String, String),
    #[error("resolved status requires a non-empty internal_id")]
    MissingInternalId,
    #[error("unsupported entity type for conflict resolution: {0}")]
    UnsupportedEntityType(String),
    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),
}

impl From<ConflictError> for ResolveConflictError {
    fn from(e: ConflictError) -> Self {
        match e {
            ConflictError::NotFound(id) => ResolveConflictError::NotFound(id),
            ConflictError::InvalidTransition(from, to) => {
                ResolveConflictError::InvalidTransition(from, to)
            }
            ConflictError::Database(db) => ResolveConflictError::Database(db),
            other => ResolveConflictError::Database(sqlx::Error::Protocol(other.to_string())),
        }
    }
}

/// Resolve a single conflict: explicit (entity_type, conflict_class) dispatch,
/// transactional DB update, and `integrations.sync.conflict.resolved` outbox
/// enqueue in the same commit.
///
/// The outbox relay fires AFTER the commit, so the event never precedes the
/// ledger transition.
pub async fn resolve_conflict_transactional(
    pool: &PgPool,
    app_id: &str,
    conflict_id: Uuid,
    resolved_by: &str,
    internal_id: &str,
    resolution_note: Option<&str>,
) -> Result<crate::domain::sync::conflicts::ConflictRow, ResolveConflictError> {
    if internal_id.is_empty() {
        return Err(ResolveConflictError::MissingInternalId);
    }

    // Pre-tx guard: load current row and validate the status transition.
    let conflict = get_conflict(pool, app_id, conflict_id)
        .await
        .map_err(ResolveConflictError::from)?
        .ok_or(ResolveConflictError::NotFound(conflict_id))?;

    let current_status =
        ConflictStatus::from_str(&conflict.status).unwrap_or(ConflictStatus::Pending);
    if current_status != ConflictStatus::Pending {
        return Err(ResolveConflictError::InvalidTransition(
            conflict.status.clone(),
            "resolved".to_string(),
        ));
    }

    // Explicit (entity_type, conflict_class) dispatch — all arms currently
    // share the same DB path; the match makes routing explicit and extensible
    // per-entity without trait dispatch.
    match (
        conflict.entity_type.as_str(),
        conflict.conflict_class.as_str(),
    ) {
        ("customer", "edit") | ("customer", "creation") | ("customer", "deletion") => {}
        ("invoice", "edit") | ("invoice", "creation") | ("invoice", "deletion") => {}
        ("payment", "edit") | ("payment", "creation") | ("payment", "deletion") => {}
        _ => {
            return Err(ResolveConflictError::UnsupportedEntityType(
                conflict.entity_type.clone(),
            ));
        }
    }

    // Atomic: DB status transition + outbox event enqueue.
    let mut tx = pool.begin().await.map_err(ResolveConflictError::Database)?;

    let resolved = resolve_conflict_tx(
        &mut tx,
        app_id,
        conflict_id,
        internal_id,
        resolved_by,
        resolution_note,
    )
    .await
    .map_err(ResolveConflictError::Database)?
    .ok_or(ResolveConflictError::NotFound(conflict_id))?;

    let event_id = Uuid::new_v4();
    let payload = SyncConflictResolvedPayload {
        app_id: app_id.to_string(),
        conflict_id,
        provider: resolved.provider.clone(),
        entity_type: resolved.entity_type.clone(),
        entity_id: resolved.entity_id.clone(),
        conflict_class: resolved.conflict_class.clone(),
        resolved_by: resolved_by.to_string(),
        internal_id: internal_id.to_string(),
        resolution_note: resolution_note.map(str::to_string),
    };
    let envelope = build_sync_conflict_resolved_envelope(
        event_id,
        app_id.to_string(),
        event_id.to_string(),
        None,
        payload,
    );
    enqueue_event_tx(
        &mut tx,
        event_id,
        EVENT_TYPE_SYNC_CONFLICT_RESOLVED,
        "sync_conflict",
        &conflict_id.to_string(),
        app_id,
        &envelope,
    )
    .await
    .map_err(ResolveConflictError::Database)?;

    tx.commit().await.map_err(ResolveConflictError::Database)?;

    Ok(resolved)
}

// ── Bulk resolve ──────────────────────────────────────────────────────────────

/// Maximum items allowed in a single bulk-resolve call.
pub const BULK_RESOLVE_CAP: usize = 100;

/// Input for one item in a bulk-resolve request.
#[derive(Debug)]
pub struct BulkResolveItem {
    pub conflict_id: Uuid,
    /// "resolve" | "ignore" | "unresolvable"
    pub action: String,
    /// Caller's believed authority version — used in deterministic key only.
    pub authority_version: i64,
    /// Required when action = "resolve".
    pub internal_id: Option<String>,
    pub resolution_note: Option<String>,
    /// Caller-supplied alias stored for tracking; never drives server dedupe.
    pub caller_idempotency_key: Option<String>,
}

/// Per-item outcome from a bulk-resolve operation.
///
/// Serialises with `"outcome"` as the tag so callers can always branch on one field.
#[derive(Debug, Serialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum BulkResolveOutcome {
    /// Conflict newly resolved in this call.
    Resolved {
        conflict_id: Uuid,
        deterministic_key: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        caller_idempotency_key: Option<String>,
    },
    /// Idempotent replay — same deterministic key already applied; conflict already resolved.
    AlreadyResolved {
        conflict_id: Uuid,
        deterministic_key: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        caller_idempotency_key: Option<String>,
    },
    /// Conflict newly set to ignored in this call.
    Ignored {
        conflict_id: Uuid,
        deterministic_key: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        caller_idempotency_key: Option<String>,
    },
    /// Idempotent replay — conflict already ignored by same deterministic key.
    AlreadyIgnored {
        conflict_id: Uuid,
        deterministic_key: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        caller_idempotency_key: Option<String>,
    },
    /// Conflict newly marked unresolvable in this call.
    MarkedUnresolvable {
        conflict_id: Uuid,
        deterministic_key: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        caller_idempotency_key: Option<String>,
    },
    /// Idempotent replay — conflict already marked unresolvable by same deterministic key.
    AlreadyUnresolvable {
        conflict_id: Uuid,
        deterministic_key: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        caller_idempotency_key: Option<String>,
    },
    /// Conflict is already in a terminal state reached by a different operation.
    TerminalByOther {
        conflict_id: Uuid,
        current_status: String,
    },
    /// Conflict not found or belongs to a different tenant.
    NotFound { conflict_id: Uuid },
    /// Entity type is not supported for conflict resolution.
    UnsupportedEntityType {
        conflict_id: Uuid,
        entity_type: String,
    },
    /// action string is not one of resolve / ignore / unresolvable.
    InvalidAction { conflict_id: Uuid, action: String },
    /// action = resolve but internal_id is absent or empty.
    MissingInternalId { conflict_id: Uuid },
    /// Unexpected error processing this item.
    Error { conflict_id: Uuid, message: String },
}

/// Top-level error from `bulk_resolve_conflicts` (not per-item).
#[derive(Debug, thiserror::Error)]
pub enum BulkResolveError {
    #[error("items count {0} exceeds maximum of 100")]
    ExceedsCapacity(usize),
}

/// Resolve, ignore, or mark-unresolvable up to `BULK_RESOLVE_CAP` conflicts in
/// best-effort fashion.  Each item is processed independently; the operation is
/// not transactional across items.  Items are processed in submission order and
/// outcomes are returned in the same order.
pub async fn bulk_resolve_conflicts(
    pool: &PgPool,
    app_id: &str,
    resolved_by: &str,
    items: Vec<BulkResolveItem>,
) -> Result<Vec<BulkResolveOutcome>, BulkResolveError> {
    if items.len() > BULK_RESOLVE_CAP {
        return Err(BulkResolveError::ExceedsCapacity(items.len()));
    }
    let mut outcomes = Vec::with_capacity(items.len());
    for item in items {
        let det_key =
            compute_resolve_det_key(item.conflict_id, &item.action, item.authority_version);
        outcomes.push(process_bulk_item(pool, app_id, resolved_by, &item, &det_key).await);
    }
    Ok(outcomes)
}

async fn process_bulk_item(
    pool: &PgPool,
    app_id: &str,
    resolved_by: &str,
    item: &BulkResolveItem,
    det_key: &str,
) -> BulkResolveOutcome {
    let cid = item.conflict_id;

    // Validate action.
    if !matches!(item.action.as_str(), "resolve" | "ignore" | "unresolvable") {
        return BulkResolveOutcome::InvalidAction {
            conflict_id: cid,
            action: item.action.clone(),
        };
    }
    if item.action == "resolve" && item.internal_id.as_ref().map_or(true, |s| s.is_empty()) {
        return BulkResolveOutcome::MissingInternalId { conflict_id: cid };
    }

    // Load conflict — tenant-scoped.
    let conflict = match get_conflict(pool, app_id, cid).await {
        Ok(Some(c)) => c,
        Ok(None) => return BulkResolveOutcome::NotFound { conflict_id: cid },
        Err(e) => {
            tracing::error!(error = %e, conflict_id = %cid, "bulk_resolve: DB error loading conflict");
            return BulkResolveOutcome::Error {
                conflict_id: cid,
                message: "database error".to_string(),
            };
        }
    };

    // Explicit (entity_type, conflict_class) dispatch — same surface as single-item resolve.
    match (
        conflict.entity_type.as_str(),
        conflict.conflict_class.as_str(),
    ) {
        ("customer" | "invoice" | "payment", "edit" | "creation" | "deletion") => {}
        _ => {
            return BulkResolveOutcome::UnsupportedEntityType {
                conflict_id: cid,
                entity_type: conflict.entity_type.clone(),
            }
        }
    }

    let current_status =
        ConflictStatus::from_str(&conflict.status).unwrap_or(ConflictStatus::Pending);

    // Already terminal: idempotent replay if same det_key; TerminalByOther otherwise.
    if current_status.is_terminal() {
        let same_key = conflict.resolution_idempotency_key.as_deref() == Some(det_key);
        return if same_key {
            match current_status {
                ConflictStatus::Resolved => BulkResolveOutcome::AlreadyResolved {
                    conflict_id: cid,
                    deterministic_key: det_key.to_string(),
                    caller_idempotency_key: item.caller_idempotency_key.clone(),
                },
                ConflictStatus::Ignored => BulkResolveOutcome::AlreadyIgnored {
                    conflict_id: cid,
                    deterministic_key: det_key.to_string(),
                    caller_idempotency_key: item.caller_idempotency_key.clone(),
                },
                ConflictStatus::Unresolvable => BulkResolveOutcome::AlreadyUnresolvable {
                    conflict_id: cid,
                    deterministic_key: det_key.to_string(),
                    caller_idempotency_key: item.caller_idempotency_key.clone(),
                },
                ConflictStatus::Pending => unreachable!(),
            }
        } else {
            BulkResolveOutcome::TerminalByOther {
                conflict_id: cid,
                current_status: conflict.status,
            }
        };
    }

    match item.action.as_str() {
        "resolve" => {
            let internal_id = match item.internal_id.as_deref() {
                Some(id) => id,
                None => {
                    return BulkResolveOutcome::Error {
                        conflict_id: cid,
                        message: "internal_id required for resolve action".to_string(),
                    };
                }
            };
            let mut tx = match pool.begin().await {
                Ok(t) => t,
                Err(e) => {
                    tracing::error!(error = %e, conflict_id = %cid, "bulk_resolve: begin tx failed");
                    return BulkResolveOutcome::Error {
                        conflict_id: cid,
                        message: "database error".to_string(),
                    };
                }
            };
            let resolved = match resolve_conflict_with_key_tx(
                &mut tx,
                app_id,
                cid,
                internal_id,
                resolved_by,
                item.resolution_note.as_deref(),
                det_key,
            )
            .await
            {
                Ok(Some(r)) => r,
                Ok(None) => {
                    let _ = tx.rollback().await;
                    return BulkResolveOutcome::TerminalByOther {
                        conflict_id: cid,
                        current_status: "unknown".to_string(),
                    };
                }
                Err(e) => {
                    let _ = tx.rollback().await;
                    tracing::error!(error = %e, conflict_id = %cid, "bulk_resolve: resolve_tx failed");
                    return BulkResolveOutcome::Error {
                        conflict_id: cid,
                        message: "database error".to_string(),
                    };
                }
            };
            let event_id = Uuid::new_v4();
            let payload = SyncConflictResolvedPayload {
                app_id: app_id.to_string(),
                conflict_id: cid,
                provider: resolved.provider.clone(),
                entity_type: resolved.entity_type.clone(),
                entity_id: resolved.entity_id.clone(),
                conflict_class: resolved.conflict_class.clone(),
                resolved_by: resolved_by.to_string(),
                internal_id: internal_id.to_string(),
                resolution_note: item.resolution_note.clone(),
            };
            let envelope = build_sync_conflict_resolved_envelope(
                event_id,
                app_id.to_string(),
                event_id.to_string(),
                None,
                payload,
            );
            if let Err(e) = enqueue_event_tx(
                &mut tx,
                event_id,
                EVENT_TYPE_SYNC_CONFLICT_RESOLVED,
                "sync_conflict",
                &cid.to_string(),
                app_id,
                &envelope,
            )
            .await
            {
                let _ = tx.rollback().await;
                tracing::error!(error = %e, conflict_id = %cid, "bulk_resolve: enqueue event failed");
                return BulkResolveOutcome::Error {
                    conflict_id: cid,
                    message: "database error".to_string(),
                };
            }
            if let Err(e) = tx.commit().await {
                tracing::error!(error = %e, conflict_id = %cid, "bulk_resolve: commit failed");
                return BulkResolveOutcome::Error {
                    conflict_id: cid,
                    message: "database error".to_string(),
                };
            }
            BulkResolveOutcome::Resolved {
                conflict_id: cid,
                deterministic_key: det_key.to_string(),
                caller_idempotency_key: item.caller_idempotency_key.clone(),
            }
        }
        "ignore" => {
            match close_conflict_with_key(
                pool,
                app_id,
                cid,
                ConflictStatus::Ignored,
                resolved_by,
                item.resolution_note.as_deref(),
                det_key,
            )
            .await
            {
                Ok(Some(_)) => BulkResolveOutcome::Ignored {
                    conflict_id: cid,
                    deterministic_key: det_key.to_string(),
                    caller_idempotency_key: item.caller_idempotency_key.clone(),
                },
                Ok(None) => BulkResolveOutcome::TerminalByOther {
                    conflict_id: cid,
                    current_status: "unknown".to_string(),
                },
                Err(e) => {
                    tracing::error!(error = %e, conflict_id = %cid, "bulk_resolve: close ignore failed");
                    BulkResolveOutcome::Error {
                        conflict_id: cid,
                        message: "database error".to_string(),
                    }
                }
            }
        }
        "unresolvable" => {
            match close_conflict_with_key(
                pool,
                app_id,
                cid,
                ConflictStatus::Unresolvable,
                resolved_by,
                item.resolution_note.as_deref(),
                det_key,
            )
            .await
            {
                Ok(Some(_)) => BulkResolveOutcome::MarkedUnresolvable {
                    conflict_id: cid,
                    deterministic_key: det_key.to_string(),
                    caller_idempotency_key: item.caller_idempotency_key.clone(),
                },
                Ok(None) => BulkResolveOutcome::TerminalByOther {
                    conflict_id: cid,
                    current_status: "unknown".to_string(),
                },
                Err(e) => {
                    tracing::error!(error = %e, conflict_id = %cid, "bulk_resolve: close unresolvable failed");
                    BulkResolveOutcome::Error {
                        conflict_id: cid,
                        message: "database error".to_string(),
                    }
                }
            }
        }
        _ => unreachable!("action validated at top of process_bulk_item"),
    }
}

// ── Push error ────────────────────────────────────────────────────────────────

#[derive(Debug, thiserror::Error)]
pub enum PushError {
    /// An identical push attempt (same app/provider/entity/operation/fingerprint)
    /// is already accepted or inflight.
    #[error("duplicate push intent: an equivalent attempt is already pending")]
    DuplicateIntent,
    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),
}

// ── Private QBO call result ───────────────────────────────────────────────────

enum QboCallResult {
    Succeeded {
        external_value: Value,
        provider_entity_id: Option<String>,
    },
    Fault {
        code: String,
        message: String,
    },
    Unknown {
        message: String,
    },
}

fn classify_qbo_error(e: QboError) -> QboCallResult {
    match e {
        QboError::ApiFault { message, code, .. } => QboCallResult::Fault { code, message },
        QboError::RateLimited { .. } => QboCallResult::Fault {
            code: "rate_limited".into(),
            message: "QBO rate limit exceeded".into(),
        },
        QboError::AuthFailed => QboCallResult::Fault {
            code: "auth_failed".into(),
            message: "QBO authentication failed".into(),
        },
        QboError::SyncTokenExhausted(n) => QboCallResult::Fault {
            code: "sync_token_exhausted".into(),
            message: format!("SyncToken conflict after {} retries", n),
        },
        QboError::TokenError(msg) => QboCallResult::Fault {
            code: "token_error".into(),
            message: msg,
        },
        QboError::Http(e) => QboCallResult::Unknown {
            message: e.to_string(),
        },
        QboError::Deserialize(msg) => QboCallResult::Unknown { message: msg },
        QboError::ConflictDetected { entity_id, .. } => QboCallResult::Fault {
            code: "concurrent_edit_conflict".into(),
            message: format!(
                "touched field changed in QBO while inflight for entity {}",
                entity_id
            ),
        },
    }
}

fn is_unique_violation(e: &sqlx::Error) -> bool {
    matches!(
        e,
        sqlx::Error::Database(db) if db.code().map_or(false, |c| c == "23505")
    )
}

/// Extract result markers from a QBO provider response.
///
/// Tries both the unwrapped entity shape (create responses) and the entity-type-wrapped
/// shape (update responses).  All fields are optional; callers should handle None gracefully.
fn extract_qbo_markers(
    external_value: &serde_json::Value,
) -> (Option<String>, Option<chrono::DateTime<Utc>>, String) {
    // SyncToken: try top-level first (create shape), then first nested object (update shape).
    let sync_token = external_value["SyncToken"]
        .as_str()
        .or_else(|| {
            external_value
                .as_object()
                .and_then(|m| m.values().find_map(|v| v["SyncToken"].as_str()))
        })
        .map(|s| s.to_string());

    // MetaData.LastUpdatedTime: same two-shape strategy.
    let lut_str = external_value["MetaData"]["LastUpdatedTime"]
        .as_str()
        .or_else(|| {
            external_value.as_object().and_then(|m| {
                m.values()
                    .find_map(|v| v["MetaData"]["LastUpdatedTime"].as_str())
            })
        });

    let last_updated_time = lut_str.and_then(|s| {
        chrono::DateTime::parse_from_rfc3339(s)
            .ok()
            .map(|dt| truncate_to_millis(dt.with_timezone(&Utc)))
    });

    // Projection hash: stable fingerprint of the external state for drift correlation.
    let projection_hash =
        compute_fingerprint(sync_token.as_deref(), last_updated_time, external_value);

    (sync_token, last_updated_time, projection_hash)
}

/// Whether a failure code is eligible for automatic retry.
fn is_retryable(code: &str) -> bool {
    matches!(code, "rate_limited" | "token_error" | "inflight_timeout")
}

// ── Dev-local rate-limit fixture ─────────────────────────────────────────────

/// Returns true when both `APP_PROFILE=dev-local` and `QBO_FORCE_RATE_LIMIT=1`
/// are set.  Used to short-circuit `orchestrate_push` with an exact replica of
/// the `rate_limited` fault taxonomy, enabling deterministic E2E testing of
/// retry/backoff logic without hitting Intuit's sandbox.
///
/// The double-gate (profile + flag) ensures the fixture can never fire in
/// staging or production even if `QBO_FORCE_RATE_LIMIT` is accidentally set.
fn is_rate_limit_fixture_active() -> bool {
    std::env::var("APP_PROFILE").unwrap_or_default() == "dev-local"
        && std::env::var("QBO_FORCE_RATE_LIMIT").unwrap_or_default() == "1"
}

// ── Core orchestration ────────────────────────────────────────────────────────

/// Run the full push state machine for one entity write.
///
/// Steps:
///   1. Read current authority version.
///   2. Insert push attempt stamped with `authority_version`.
///   3. Pre-call version check — supersede if stale.
///   4. Transition to inflight.
///   5. Execute `qbo_fn(attempt_id)`.
///   6. Re-read authority version; route to stale-authority or normal terminal.
///   7. Return `PushOutcome`.
async fn orchestrate_push<F, Fut>(
    pool: &PgPool,
    app_id: &str,
    provider: &str,
    entity_type: &str,
    entity_id: &str,
    operation: &str,
    authority_version: i64,
    request_fingerprint: &str,
    qbo_fn: F,
) -> Result<PushOutcome, PushError>
where
    F: FnOnce(Uuid) -> Fut,
    Fut: Future<Output = QboCallResult>,
{
    // 1. Current authority version (0 = row not yet created).
    let current_auth = authority_repo::get_authority(pool, app_id, provider, entity_type)
        .await
        .map_err(PushError::Database)?
        .map(|r| r.authority_version)
        .unwrap_or(0);

    // 2. Insert attempt stamped with caller's believed version.
    let attempt = push_attempts::insert_attempt(
        pool,
        app_id,
        provider,
        entity_type,
        entity_id,
        operation,
        authority_version,
        request_fingerprint,
    )
    .await
    .map_err(|e| {
        if is_unique_violation(&e) {
            PushError::DuplicateIntent
        } else {
            PushError::Database(e)
        }
    })?;

    // 3. Pre-call version check.
    let pre = push_attempts::pre_call_version_check(pool, attempt.id, current_auth)
        .await
        .map_err(PushError::Database)?;

    if let PreCallOutcome::Superseded(_) = pre {
        return Ok(PushOutcome::Superseded {
            attempt_id: attempt.id,
            entity_id: entity_id.to_string(),
            current_authority_version: current_auth,
        });
    }

    // 4. Transition to inflight.
    push_attempts::transition_to_inflight(pool, attempt.id)
        .await
        .map_err(PushError::Database)?;

    // 5. Execute the QBO call — or inject rate-limit fixture in dev-local.
    let qbo_result = if is_rate_limit_fixture_active() {
        QboCallResult::Fault {
            code: "rate_limited".into(),
            message: "Forced 429 (QBO_FORCE_RATE_LIMIT=1, APP_PROFILE=dev-local)".into(),
        }
    } else {
        qbo_fn(attempt.id).await
    };

    // 6. Re-read authority version for post-call stale detection.
    let post_auth = authority_repo::get_authority(pool, app_id, provider, entity_type)
        .await
        .ok()
        .flatten()
        .map(|r| r.authority_version)
        .unwrap_or(attempt.authority_version);

    let authority_stale = post_auth != attempt.authority_version;

    // 7. Map QBO result → terminal outcome.
    match qbo_result {
        QboCallResult::Succeeded {
            external_value,
            provider_entity_id,
        } => {
            if authority_stale {
                // Authority flipped while inflight: reconcile without a platform
                // snapshot (HTTP context has no DB entity reader).  Both sides
                // None → AutoClosed per the reconcile invariant.
                match push_attempts::post_call_reconcile(
                    pool,
                    attempt.id,
                    app_id,
                    provider,
                    entity_type,
                    entity_id,
                    None,
                    Some(external_value),
                )
                .await
                .map_err(|e| match e {
                    super::push_attempts::ReconcileError::Database(db) => PushError::Database(db),
                    super::push_attempts::ReconcileError::ValueTooLarge => {
                        // Treat as unknown failure: log and fall through.
                        tracing::error!("push reconcile: external_value exceeds 256 KB limit");
                        PushError::Database(sqlx::Error::RowNotFound) // sentinel
                    }
                })? {
                    ReconcileOutcome::AutoClosed => Ok(PushOutcome::StaleAuthorityAutoClosed {
                        attempt_id: attempt.id,
                        entity_id: entity_id.to_string(),
                    }),
                    ReconcileOutcome::ConflictOpened(conflict) => {
                        Ok(PushOutcome::StaleAuthorityConflictOpened {
                            attempt_id: attempt.id,
                            entity_id: entity_id.to_string(),
                            conflict_id: conflict.id,
                        })
                    }
                }
            } else {
                // Extract and normalize result markers from the provider response.
                let (result_sync_token, result_last_updated_time, projection_hash) =
                    extract_qbo_markers(&external_value);
                push_attempts::complete_attempt_with_markers(
                    pool,
                    attempt.id,
                    result_sync_token.as_deref(),
                    result_last_updated_time,
                    Some(&projection_hash),
                    provider_entity_id.as_deref(),
                )
                .await
                .map_err(PushError::Database)?;
                Ok(PushOutcome::Succeeded {
                    attempt_id: attempt.id,
                    entity_id: entity_id.to_string(),
                    provider_entity_id,
                })
            }
        }
        QboCallResult::Fault { code, message } => {
            let connector_id = oauth_repo::get_connection(pool, app_id, provider)
                .await
                .ok()
                .flatten()
                .map(|c| c.id)
                .unwrap_or_else(Uuid::nil);

            let retryable = is_retryable(&code);
            let event_id = Uuid::new_v4();
            let payload = SyncPushFailedPayload {
                app_id: app_id.to_string(),
                connector_id,
                entity_type: entity_type.to_string(),
                entity_id: entity_id.to_string(),
                attempt_number: 1,
                failure_reason: message.clone(),
                failure_code: code.clone(),
                retryable,
                external_error: Some(message.clone()),
            };
            let envelope = build_sync_push_failed_envelope(
                event_id,
                app_id.to_string(),
                event_id.to_string(),
                None,
                payload,
            );

            let mut tx = pool.begin().await.map_err(PushError::Database)?;
            let _ =
                push_attempts::fail_attempt_tx(&mut tx, attempt.id, "failed", Some(&message)).await;
            let _ = enqueue_event_tx(
                &mut tx,
                event_id,
                EVENT_TYPE_SYNC_PUSH_FAILED,
                "sync_push_attempt",
                &attempt.id.to_string(),
                app_id,
                &envelope,
            )
            .await;
            tx.commit().await.map_err(PushError::Database)?;

            Ok(PushOutcome::Failed {
                attempt_id: attempt.id,
                entity_id: entity_id.to_string(),
                error_code: code,
                error_message: message,
            })
        }
        QboCallResult::Unknown { message } => {
            let connector_id = oauth_repo::get_connection(pool, app_id, provider)
                .await
                .ok()
                .flatten()
                .map(|c| c.id)
                .unwrap_or_else(Uuid::nil);

            let event_id = Uuid::new_v4();
            let payload = SyncPushFailedPayload {
                app_id: app_id.to_string(),
                connector_id,
                entity_type: entity_type.to_string(),
                entity_id: entity_id.to_string(),
                attempt_number: 1,
                failure_reason: message.clone(),
                failure_code: "unknown_failure".to_string(),
                retryable: true,
                external_error: Some(message.clone()),
            };
            let envelope = build_sync_push_failed_envelope(
                event_id,
                app_id.to_string(),
                event_id.to_string(),
                None,
                payload,
            );

            let mut tx = pool.begin().await.map_err(PushError::Database)?;
            let _ = push_attempts::fail_attempt_tx(
                &mut tx,
                attempt.id,
                "unknown_failure",
                Some(&message),
            )
            .await;
            let _ = enqueue_event_tx(
                &mut tx,
                event_id,
                EVENT_TYPE_SYNC_PUSH_FAILED,
                "sync_push_attempt",
                &attempt.id.to_string(),
                app_id,
                &envelope,
            )
            .await;
            tx.commit().await.map_err(PushError::Database)?;

            Ok(PushOutcome::UnknownFailure {
                attempt_id: attempt.id,
                entity_id: entity_id.to_string(),
                error_message: message,
            })
        }
    }
}

// ── ResolveService ────────────────────────────────────────────────────────────

/// Central router for QBO entity-write operations.
///
/// Each entity type has a dedicated method (`push_customer`, `push_invoice`,
/// `push_payment`) that runs the full push state machine and returns a
/// `PushOutcome`.  No trait dispatch — each handler path is explicit.
pub struct ResolveService {
    pub(crate) qbo: Arc<QboClient>,
}

impl ResolveService {
    pub fn new(qbo: Arc<QboClient>) -> Self {
        Self { qbo }
    }

    /// Push a customer entity to QBO.
    pub async fn push_customer(
        &self,
        pool: &PgPool,
        app_id: &str,
        entity_id: &str,
        operation: &str,
        authority_version: i64,
        request_fingerprint: &str,
        payload: &Value,
    ) -> Result<PushOutcome, PushError> {
        let qbo = self.qbo.clone();
        let payload_owned = payload.clone();
        let op = operation.to_string();

        orchestrate_push(
            pool,
            app_id,
            "quickbooks",
            "customer",
            entity_id,
            operation,
            authority_version,
            request_fingerprint,
            move |attempt_id| async move {
                match op.as_str() {
                    "create" => match serde_json::from_value::<QboCustomerPayload>(payload_owned) {
                        Ok(p) => match qbo.create_customer(&p, attempt_id).await {
                            Ok(val) => {
                                let pid = val["Id"].as_str().map(String::from);
                                QboCallResult::Succeeded {
                                    external_value: val,
                                    provider_entity_id: pid,
                                }
                            }
                            Err(e) => classify_qbo_error(e),
                        },
                        Err(e) => QboCallResult::Fault {
                            code: "invalid_payload".into(),
                            message: e.to_string(),
                        },
                    },
                    "update" => {
                        let eid = payload_owned["Id"].as_str().unwrap_or("").to_string();
                        let baseline = qbo
                            .get_entity("Customer", &eid)
                            .await
                            .ok()
                            .map(|v| v["Customer"].clone());
                        match qbo
                            .update_entity_with_guard(
                                "Customer",
                                payload_owned,
                                baseline.as_ref(),
                                attempt_id,
                            )
                            .await
                        {
                            Ok(val) => {
                                let pid = val["Customer"]["Id"].as_str().map(String::from);
                                QboCallResult::Succeeded {
                                    external_value: val,
                                    provider_entity_id: pid,
                                }
                            }
                            Err(e) => classify_qbo_error(e),
                        }
                    }
                    _ => QboCallResult::Fault {
                        code: "invalid_operation".into(),
                        message: format!("Unknown operation: {}", op),
                    },
                }
            },
        )
        .await
    }

    /// Push an invoice entity to QBO.
    pub async fn push_invoice(
        &self,
        pool: &PgPool,
        app_id: &str,
        entity_id: &str,
        operation: &str,
        authority_version: i64,
        request_fingerprint: &str,
        payload: &Value,
    ) -> Result<PushOutcome, PushError> {
        let qbo = self.qbo.clone();
        let payload_owned = payload.clone();
        let op = operation.to_string();

        orchestrate_push(
            pool,
            app_id,
            "quickbooks",
            "invoice",
            entity_id,
            operation,
            authority_version,
            request_fingerprint,
            move |attempt_id| async move {
                match op.as_str() {
                    "create" => match serde_json::from_value::<QboInvoicePayload>(payload_owned) {
                        Ok(p) => match qbo.create_invoice(&p, attempt_id).await {
                            Ok(val) => {
                                let pid = val["Id"].as_str().map(String::from);
                                QboCallResult::Succeeded {
                                    external_value: val,
                                    provider_entity_id: pid,
                                }
                            }
                            Err(e) => classify_qbo_error(e),
                        },
                        Err(e) => QboCallResult::Fault {
                            code: "invalid_payload".into(),
                            message: e.to_string(),
                        },
                    },
                    "update" => {
                        let eid = payload_owned["Id"].as_str().unwrap_or("").to_string();
                        let baseline = qbo
                            .get_entity("Invoice", &eid)
                            .await
                            .ok()
                            .map(|v| v["Invoice"].clone());
                        match qbo
                            .update_entity_with_guard(
                                "Invoice",
                                payload_owned,
                                baseline.as_ref(),
                                attempt_id,
                            )
                            .await
                        {
                            Ok(val) => {
                                let pid = val["Invoice"]["Id"].as_str().map(String::from);
                                QboCallResult::Succeeded {
                                    external_value: val,
                                    provider_entity_id: pid,
                                }
                            }
                            Err(e) => classify_qbo_error(e),
                        }
                    }
                    "void" => {
                        let qbo_id = payload_owned["Id"].as_str().unwrap_or("").to_string();
                        let sync_token = payload_owned["SyncToken"]
                            .as_str()
                            .unwrap_or("0")
                            .to_string();
                        match qbo.void_invoice(&qbo_id, &sync_token, attempt_id).await {
                            Ok(val) => {
                                let pid = val["Id"].as_str().map(String::from);
                                QboCallResult::Succeeded {
                                    external_value: val,
                                    provider_entity_id: pid,
                                }
                            }
                            Err(e) => classify_qbo_error(e),
                        }
                    }
                    _ => QboCallResult::Fault {
                        code: "invalid_operation".into(),
                        message: format!("Unknown operation: {}", op),
                    },
                }
            },
        )
        .await
    }

    /// Push a payment entity to QBO.
    pub async fn push_payment(
        &self,
        pool: &PgPool,
        app_id: &str,
        entity_id: &str,
        operation: &str,
        authority_version: i64,
        request_fingerprint: &str,
        payload: &Value,
    ) -> Result<PushOutcome, PushError> {
        let qbo = self.qbo.clone();
        let payload_owned = payload.clone();
        let op = operation.to_string();

        orchestrate_push(
            pool,
            app_id,
            "quickbooks",
            "payment",
            entity_id,
            operation,
            authority_version,
            request_fingerprint,
            move |attempt_id| async move {
                match op.as_str() {
                    "create" => match serde_json::from_value::<QboPaymentPayload>(payload_owned) {
                        Ok(p) => match qbo.create_payment(&p, attempt_id).await {
                            Ok(val) => {
                                let pid = val["Id"].as_str().map(String::from);
                                QboCallResult::Succeeded {
                                    external_value: val,
                                    provider_entity_id: pid,
                                }
                            }
                            Err(e) => classify_qbo_error(e),
                        },
                        Err(e) => QboCallResult::Fault {
                            code: "invalid_payload".into(),
                            message: e.to_string(),
                        },
                    },
                    "update" => {
                        let eid = payload_owned["Id"].as_str().unwrap_or("").to_string();
                        let baseline = qbo
                            .get_entity("Payment", &eid)
                            .await
                            .ok()
                            .map(|v| v["Payment"].clone());
                        match qbo
                            .update_entity_with_guard(
                                "Payment",
                                payload_owned,
                                baseline.as_ref(),
                                attempt_id,
                            )
                            .await
                        {
                            Ok(val) => {
                                let pid = val["Payment"]["Id"].as_str().map(String::from);
                                QboCallResult::Succeeded {
                                    external_value: val,
                                    provider_entity_id: pid,
                                }
                            }
                            Err(e) => classify_qbo_error(e),
                        }
                    }
                    "delete" => {
                        let qbo_id = payload_owned["Id"].as_str().unwrap_or("").to_string();
                        let sync_token = payload_owned["SyncToken"]
                            .as_str()
                            .unwrap_or("")
                            .to_string();
                        match qbo.delete_payment(&qbo_id, &sync_token, attempt_id).await {
                            Ok(val) => {
                                let pid = val["Id"].as_str().map(String::from);
                                QboCallResult::Succeeded {
                                    external_value: val,
                                    provider_entity_id: pid,
                                }
                            }
                            Err(e) => classify_qbo_error(e),
                        }
                    }
                    _ => QboCallResult::Fault {
                        code: "invalid_operation".into(),
                        message: format!("Unknown operation: {}", op),
                    },
                }
            },
        )
        .await
    }
}
