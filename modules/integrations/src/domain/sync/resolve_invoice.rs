//! Invoice push handler: create/update/void operations against QBO.
//!
//! All paths run through the push-attempt state machine
//! (insert → authority version check → inflight → complete) so every write is
//! auditable and idempotent across transport-layer retries.
//!
//! Invariants:
//! - QBO uses void (not hard-delete) as the canonical terminal state for invoices.
//! - Closed-period rejections (QBO code 6140) map to InvoicePushError::ClosedPeriod.
//! - Currency and locale fields are forwarded from the payload without modification.
//! - Line-item amounts are passed as f64 — never rounded or truncated at this layer.

use serde_json::Value;
use sqlx::PgPool;
use uuid::Uuid;

use super::{
    authority_repo,
    push_attempts::{self, PreCallOutcome, PushAttemptRow},
    resolve_service::ResolveService,
};
use crate::domain::qbo::{
    client::{QboClient, QboInvoicePayload},
    QboError,
};

// ── Action ────────────────────────────────────────────────────────────────────

/// The write intent for a single invoice push to QBO.
#[derive(Debug)]
pub enum InvoiceAction {
    /// Create a new QBO invoice.
    Create(QboInvoicePayload),
    /// Update an existing QBO invoice. `sync_token` is the latest SyncToken
    /// from QBO; the client runs an intent-guard check on stale-token retry.
    Update {
        qbo_id: String,
        sync_token: String,
        payload: QboInvoicePayload,
    },
    /// Void a QBO invoice. QBO does not support hard-deleting invoices; voiding
    /// sets Balance=0 and locks the invoice for further edits.
    Void { qbo_id: String, sync_token: String },
}

fn operation_name(action: &InvoiceAction) -> &'static str {
    match action {
        InvoiceAction::Create(_) => "create",
        InvoiceAction::Update { .. } => "update",
        InvoiceAction::Void { .. } => "void",
    }
}

// ── Push request ──────────────────────────────────────────────────────────────

/// Input for a single invoice push.
pub struct InvoicePushRequest {
    pub app_id: String,
    /// Platform-side entity ID (stored in the push-attempt ledger).
    pub entity_id: String,
    /// Authority version at the time the push was enqueued. Compared against
    /// the live authority row; divergence causes `Superseded`.
    pub authority_version: i64,
    /// Deterministic ID from the platform ledger row. Reused across retries so
    /// QBO deduplicates via its `?requestid=` parameter.
    pub request_id: Uuid,
    pub action: InvoiceAction,
}

// ── Outcome ───────────────────────────────────────────────────────────────────

/// Result of a successful invoice push dispatch.
#[derive(Debug)]
pub enum InvoicePushOutcome {
    /// QBO accepted the write. `qbo_entity` is the `Invoice` object from the
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
pub enum InvoicePushError {
    /// QBO rejected the write because the transaction falls in a closed
    /// accounting period (QBO code 6140). The attempt is recorded as `failed`
    /// with reason `closed_period`; the caller must decide whether to redate
    /// or abandon the push.
    #[error("QBO closed accounting period — invoice {qbo_id} cannot be modified")]
    ClosedPeriod { qbo_id: String },
    #[error("QBO fault: {0}")]
    Qbo(#[from] QboError),
    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),
}

// ── Public entry point ────────────────────────────────────────────────────────

/// Execute one invoice push through the push-attempt state machine.
///
/// Steps:
/// 1. Record intent as `accepted`.
/// 2. Re-read current authority version; supersede if it has advanced.
/// 3. Transition to `inflight`.
/// 4. Dispatch to QBO (create / update / void).
/// 5. Transition to `succeeded` or `failed`.
pub async fn push_invoice(
    pool: &PgPool,
    svc: &ResolveService,
    req: InvoicePushRequest,
) -> Result<InvoicePushOutcome, InvoicePushError> {
    // 1. Record intent.
    let attempt = push_attempts::insert_attempt(
        pool,
        &req.app_id,
        "quickbooks",
        "invoice",
        &req.entity_id,
        operation_name(&req.action),
        req.authority_version,
        &req.request_id.to_string(),
    )
    .await?;

    // 2. Re-read current authority version and check for supersession.
    let current_auth_version =
        match authority_repo::get_authority(pool, &req.app_id, "quickbooks", "invoice").await? {
            Some(row) => row.authority_version,
            // No authority record yet — version can't have advanced.
            None => req.authority_version,
        };

    match push_attempts::pre_call_version_check(pool, attempt.id, current_auth_version).await? {
        PreCallOutcome::Superseded(row) => return Ok(InvoicePushOutcome::Superseded(row)),
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
            Ok(InvoicePushOutcome::Succeeded {
                attempt: completed,
                qbo_entity,
            })
        }
        Err(ref e) => {
            if is_closed_period(e) {
                let qbo_id = qbo_id_from_action(&req.action);
                push_attempts::complete_attempt(pool, attempt.id, "failed", Some("closed_period"))
                    .await?;
                return Err(InvoicePushError::ClosedPeriod { qbo_id });
            }
            let msg = e.to_string();
            push_attempts::complete_attempt(pool, attempt.id, "failed", Some(&msg)).await?;
            Err(InvoicePushError::Qbo(qbo_result.unwrap_err()))
        }
    }
}

// ── Internal helpers ──────────────────────────────────────────────────────────

async fn dispatch_to_qbo(
    qbo: &QboClient,
    action: &InvoiceAction,
    request_id: Uuid,
) -> Result<Value, QboError> {
    match action {
        InvoiceAction::Create(payload) => qbo.create_invoice(payload, request_id).await,

        InvoiceAction::Update {
            qbo_id,
            sync_token,
            payload,
        } => {
            let mut body = payload.to_qbo_json();
            body["Id"] = Value::String(qbo_id.clone());
            body["SyncToken"] = Value::String(sync_token.clone());
            // Fetch baseline so the intent guard can detect concurrent edits
            // on stale-token retry.
            let baseline = qbo
                .get_entity("Invoice", qbo_id)
                .await
                .ok()
                .map(|v| v["Invoice"].clone());
            let resp = qbo
                .update_entity_with_guard("Invoice", body, baseline.as_ref(), request_id)
                .await?;
            Ok(resp["Invoice"].clone())
        }

        InvoiceAction::Void { qbo_id, sync_token } => {
            qbo.void_invoice(qbo_id, sync_token, request_id).await
        }
    }
}

fn qbo_id_from_action(action: &InvoiceAction) -> String {
    match action {
        InvoiceAction::Create(_) => String::new(),
        InvoiceAction::Update { qbo_id, .. } | InvoiceAction::Void { qbo_id, .. } => qbo_id.clone(),
    }
}

/// Detect QBO error code 6140 (closed accounting period).
fn is_closed_period(err: &QboError) -> bool {
    if let QboError::ApiFault { code, detail, .. } = err {
        let lc = detail.to_lowercase();
        return code == "6140" || lc.contains("closed period") || lc.contains("accounting period");
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_fault(code: &str, detail: &str) -> QboError {
        QboError::ApiFault {
            fault_type: "ValidationFault".into(),
            message: "Business validation error".into(),
            code: code.into(),
            detail: detail.into(),
        }
    }

    #[test]
    fn is_closed_period_matches_code_6140() {
        assert!(is_closed_period(&make_fault(
            "6140",
            "Transaction in closed period"
        )));
    }

    #[test]
    fn is_closed_period_matches_detail_text() {
        assert!(is_closed_period(&make_fault(
            "6000",
            "Cannot modify transaction in closed accounting period"
        )));
    }

    #[test]
    fn is_closed_period_false_for_other_codes() {
        assert!(!is_closed_period(&make_fault("5010", "SyncToken mismatch")));
        assert!(!is_closed_period(&make_fault(
            "6240",
            "Duplicate DisplayName"
        )));
    }

    #[test]
    fn is_closed_period_false_for_non_fault_errors() {
        assert!(!is_closed_period(&QboError::SyncTokenExhausted(3)));
        assert!(!is_closed_period(&QboError::AuthFailed));
    }

    #[test]
    fn operation_name_covers_all_variants() {
        let create = InvoiceAction::Create(QboInvoicePayload {
            customer_ref: "1".into(),
            line_items: vec![],
            due_date: None,
            doc_number: None,
            currency_ref: None,
            txn_tax_detail: None,
            bill_addr: None,
            ship_addr: None,
            department_ref: None,
        });
        assert_eq!(operation_name(&create), "create");

        let update = InvoiceAction::Update {
            qbo_id: "42".into(),
            sync_token: "1".into(),
            payload: QboInvoicePayload {
                customer_ref: "1".into(),
                line_items: vec![],
                due_date: None,
                doc_number: None,
                currency_ref: None,
                txn_tax_detail: None,
                bill_addr: None,
                ship_addr: None,
                department_ref: None,
            },
        };
        assert_eq!(operation_name(&update), "update");

        let void = InvoiceAction::Void {
            qbo_id: "42".into(),
            sync_token: "1".into(),
        };
        assert_eq!(operation_name(&void), "void");
    }
}
