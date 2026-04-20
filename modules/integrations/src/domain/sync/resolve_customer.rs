//! Customer push handler: create/update/deactivate operations against QBO.
//!
//! All paths run through the push-attempt state machine
//! (insert → authority version check → inflight → complete) so every write is
//! auditable and idempotent across transport-layer retries.
//!
//! Invariant: the authority version is re-read from the authority table
//! immediately before dispatch, so a flip that happened after the push was
//! enqueued always causes the attempt to be superseded rather than dispatched.

use serde_json::{json, Value};
use sqlx::PgPool;
use uuid::Uuid;

use crate::domain::qbo::{
    client::{QboClient, QboCustomerPayload},
    QboError,
};
use super::{
    authority_repo,
    push_attempts::{self, PreCallOutcome, PushAttemptRow},
    resolve_service::ResolveService,
};

// ── Action ────────────────────────────────────────────────────────────────────

/// The write intent for a single customer push to QBO.
#[derive(Debug)]
pub enum CustomerAction {
    /// Create a new QBO customer.
    Create(QboCustomerPayload),
    /// Update an existing QBO customer. `sync_token` is the latest SyncToken
    /// from QBO — the client retries on stale-token automatically.
    Update {
        qbo_id: String,
        sync_token: String,
        payload: QboCustomerPayload,
    },
    /// Deactivate a QBO customer (QBO does not support hard delete; setting
    /// `Active: false` is the canonical approach).
    Delete {
        qbo_id: String,
        sync_token: String,
    },
}

fn operation_name(action: &CustomerAction) -> &'static str {
    match action {
        CustomerAction::Create(_) => "create",
        CustomerAction::Update { .. } => "update",
        CustomerAction::Delete { .. } => "delete",
    }
}

// ── Push request ──────────────────────────────────────────────────────────────

/// Input for a single customer push.
pub struct CustomerPushRequest {
    pub app_id: String,
    /// Platform-side entity ID (stored in the push-attempt ledger).
    pub entity_id: String,
    /// Authority version at the time the push was enqueued. Compared against
    /// the live authority row; divergence causes `Superseded`.
    pub authority_version: i64,
    /// Deterministic ID from the platform ledger row. Reused across retries so
    /// QBO deduplicates via its `?requestid=` parameter.
    pub request_id: Uuid,
    pub action: CustomerAction,
}

// ── Outcome ───────────────────────────────────────────────────────────────────

/// Result of a successful customer push dispatch.
#[derive(Debug)]
pub enum CustomerPushOutcome {
    /// QBO accepted the write. `qbo_entity` is the `Customer` object from the
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
pub enum CustomerPushError {
    /// QBO rejected the write because a customer with the same DisplayName
    /// already exists. `existing_id` is the QBO-side Id of that customer.
    #[error("QBO duplicate DisplayName — existing QBO id: {existing_id}")]
    DuplicateDisplayName { existing_id: String },
    #[error("QBO fault: {0}")]
    Qbo(#[from] QboError),
    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),
}

// ── Public entry point ────────────────────────────────────────────────────────

/// Execute one customer push through the push-attempt state machine.
///
/// Steps:
/// 1. Record intent as `accepted`.
/// 2. Re-read current authority version; supersede if it has advanced.
/// 3. Transition to `inflight`.
/// 4. Dispatch to QBO (create / update / deactivate).
/// 5. Transition to `succeeded` or `failed`.
pub async fn push_customer(
    pool: &PgPool,
    svc: &ResolveService,
    req: CustomerPushRequest,
) -> Result<CustomerPushOutcome, CustomerPushError> {
    // 1. Record intent.
    let attempt = push_attempts::insert_attempt(
        pool,
        &req.app_id,
        "quickbooks",
        "customer",
        &req.entity_id,
        operation_name(&req.action),
        req.authority_version,
        &req.request_id.to_string(),
    )
    .await?;

    // 2. Re-read current authority version and check for supersession.
    let current_auth_version = match authority_repo::get_authority(
        pool,
        &req.app_id,
        "quickbooks",
        "customer",
    )
    .await?
    {
        Some(row) => row.authority_version,
        // No authority record yet — version can't have advanced.
        None => req.authority_version,
    };

    match push_attempts::pre_call_version_check(pool, attempt.id, current_auth_version).await? {
        PreCallOutcome::Superseded(row) => return Ok(CustomerPushOutcome::Superseded(row)),
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
            let completed =
                push_attempts::complete_attempt(pool, attempt.id, "succeeded", None)
                    .await?
                    .unwrap_or(attempt);
            Ok(CustomerPushOutcome::Succeeded {
                attempt: completed,
                qbo_entity,
            })
        }
        Err(ref e) => {
            if let Some(existing_id) = extract_duplicate_name_id(e) {
                push_attempts::complete_attempt(
                    pool,
                    attempt.id,
                    "failed",
                    Some("duplicate_display_name"),
                )
                .await?;
                return Err(CustomerPushError::DuplicateDisplayName { existing_id });
            }
            let msg = e.to_string();
            push_attempts::complete_attempt(pool, attempt.id, "failed", Some(&msg)).await?;
            Err(CustomerPushError::Qbo(qbo_result.unwrap_err()))
        }
    }
}

// ── Internal helpers ──────────────────────────────────────────────────────────

async fn dispatch_to_qbo(
    qbo: &QboClient,
    action: &CustomerAction,
    request_id: Uuid,
) -> Result<Value, QboError> {
    match action {
        CustomerAction::Create(payload) => qbo.create_customer(payload, request_id).await,

        CustomerAction::Update {
            qbo_id,
            sync_token,
            payload,
        } => {
            let mut body = payload.to_qbo_json();
            body["Id"] = Value::String(qbo_id.clone());
            body["SyncToken"] = Value::String(sync_token.clone());
            let resp = qbo.update_entity("Customer", body, request_id).await?;
            Ok(resp["Customer"].clone())
        }

        CustomerAction::Delete { qbo_id, sync_token } => {
            let body = json!({
                "Id": qbo_id,
                "SyncToken": sync_token,
                "Active": false,
            });
            let resp = qbo.update_entity("Customer", body, request_id).await?;
            Ok(resp["Customer"].clone())
        }
    }
}

/// Detect QBO error code 6240 (duplicate DisplayName) and extract the existing
/// customer's QBO Id from the Detail field if present.
fn extract_duplicate_name_id(err: &QboError) -> Option<String> {
    if let QboError::ApiFault { code, detail, .. } = err {
        if code == "6240" || detail.to_lowercase().contains("duplicate name") {
            let existing_id = detail
                .split("QuickBooksId=")
                .nth(1)
                .and_then(|s| s.split(|c: char| !c.is_ascii_digit()).next())
                .unwrap_or("")
                .to_string();
            return Some(existing_id);
        }
    }
    None
}
