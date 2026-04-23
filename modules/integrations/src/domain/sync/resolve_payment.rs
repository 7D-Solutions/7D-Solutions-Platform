//! Payment push handler: create/update/delete operations against QBO.
//!
//! All paths run through the push-attempt state machine
//! (insert → authority version check → inflight → complete) so every write is
//! auditable and idempotent across transport-layer retries.
//!
//! QBO delete semantics: payments use POST with `?operation=delete`, not
//! `Active: false` like customers. Partial-apply invariant: `line_applications`
//! are written atomically in a single QBO call — never split across handlers.

use serde_json::Value;
use sqlx::PgPool;
use uuid::Uuid;

use super::{
    authority_repo,
    push_attempts::{self, PreCallOutcome, PushAttemptRow},
    resolve_service::ResolveService,
};
use crate::domain::qbo::{
    client::{QboClient, QboPaymentPayload},
    QboError,
};

// ── Action ────────────────────────────────────────────────────────────────────

/// The write intent for a single payment push to QBO.
#[derive(Debug)]
pub enum PaymentAction {
    /// Create a new QBO payment. `line_applications` inside the payload are
    /// written atomically — if any invoice allocation is present, all must be.
    Create(QboPaymentPayload),
    /// Update an existing QBO payment. `sync_token` is the latest SyncToken
    /// from QBO — the client retries on stale-token automatically.
    Update {
        qbo_id: String,
        sync_token: String,
        payload: QboPaymentPayload,
    },
    /// Delete a QBO payment via POST with `?operation=delete`.
    /// QBO does not support hard-delete via Active=false for payments.
    Delete { qbo_id: String, sync_token: String },
}

fn operation_name(action: &PaymentAction) -> &'static str {
    match action {
        PaymentAction::Create(_) => "create",
        PaymentAction::Update { .. } => "update",
        PaymentAction::Delete { .. } => "delete",
    }
}

// ── Push request ──────────────────────────────────────────────────────────────

/// Input for a single payment push.
pub struct PaymentPushRequest {
    pub app_id: String,
    /// Platform-side entity ID (stored in the push-attempt ledger).
    pub entity_id: String,
    /// Authority version at the time the push was enqueued.
    pub authority_version: i64,
    /// Deterministic ID from the platform ledger row. Reused across retries so
    /// QBO deduplicates via its `?requestid=` parameter.
    pub request_id: Uuid,
    pub action: PaymentAction,
}

// ── Outcome ───────────────────────────────────────────────────────────────────

/// Result of a successful payment push dispatch.
#[derive(Debug)]
pub enum PaymentPushOutcome {
    /// QBO accepted the write. `qbo_entity` is the `Payment` object from the
    /// QBO response body.
    Succeeded {
        attempt: PushAttemptRow,
        qbo_entity: Value,
    },
    /// Authority version advanced between enqueue and dispatch; the attempt was
    /// recorded as `superseded` and no external call was made.
    Superseded(PushAttemptRow),
}

// ── Error ─────────────────────────────────────────────────────────────────────

#[derive(Debug, thiserror::Error)]
pub enum PaymentPushError {
    #[error("QBO fault: {0}")]
    Qbo(#[from] QboError),
    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),
}

// ── Public entry point ────────────────────────────────────────────────────────

/// Execute one payment push through the push-attempt state machine.
///
/// Steps:
/// 1. Record intent as `accepted`.
/// 2. Re-read current authority version; supersede if it has advanced.
/// 3. Transition to `inflight`.
/// 4. Dispatch to QBO (create / update / delete).
/// 5. Transition to `succeeded` or `failed`.
pub async fn push_payment(
    pool: &PgPool,
    svc: &ResolveService,
    req: PaymentPushRequest,
) -> Result<PaymentPushOutcome, PaymentPushError> {
    // 1. Record intent.
    let attempt = push_attempts::insert_attempt(
        pool,
        &req.app_id,
        "quickbooks",
        "payment",
        &req.entity_id,
        operation_name(&req.action),
        req.authority_version,
        &req.request_id.to_string(),
    )
    .await?;

    // 2. Re-read current authority version and check for supersession.
    let current_auth_version =
        match authority_repo::get_authority(pool, &req.app_id, "quickbooks", "payment").await? {
            Some(row) => row.authority_version,
            // No authority record yet — version can't have advanced.
            None => req.authority_version,
        };

    match push_attempts::pre_call_version_check(pool, attempt.id, current_auth_version).await? {
        PreCallOutcome::Superseded(row) => return Ok(PaymentPushOutcome::Superseded(row)),
        PreCallOutcome::ReadyForInflight => {}
    }

    // 3. Transition to inflight.
    let attempt = push_attempts::transition_to_inflight(pool, attempt.id)
        .await?
        .unwrap_or(attempt);

    // 4. Dispatch to QBO.
    let qbo_result = dispatch_to_qbo(&svc.qbo, &req.action, req.request_id).await;

    // 5. Record outcome.
    match qbo_result {
        Ok(qbo_entity) => {
            let completed = push_attempts::complete_attempt(pool, attempt.id, "succeeded", None)
                .await?
                .unwrap_or(attempt);
            Ok(PaymentPushOutcome::Succeeded {
                attempt: completed,
                qbo_entity,
            })
        }
        Err(ref e) => {
            let msg = e.to_string();
            push_attempts::complete_attempt(pool, attempt.id, "failed", Some(&msg)).await?;
            Err(PaymentPushError::Qbo(qbo_result.unwrap_err()))
        }
    }
}

// ── Internal helpers ──────────────────────────────────────────────────────────

async fn dispatch_to_qbo(
    qbo: &QboClient,
    action: &PaymentAction,
    request_id: Uuid,
) -> Result<Value, QboError> {
    match action {
        PaymentAction::Create(payload) => qbo.create_payment(payload, request_id).await,

        PaymentAction::Update {
            qbo_id,
            sync_token,
            payload,
        } => {
            let mut body = payload.to_qbo_json();
            body["Id"] = Value::String(qbo_id.clone());
            body["SyncToken"] = Value::String(sync_token.clone());
            let resp = qbo.update_entity("Payment", body, request_id).await?;
            Ok(resp["Payment"].clone())
        }

        PaymentAction::Delete { qbo_id, sync_token } => {
            qbo.delete_payment(qbo_id, sync_token, request_id).await
        }
    }
}
