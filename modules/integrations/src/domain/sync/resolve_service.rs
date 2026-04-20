//! Resolve service: orchestrates sync push operations against QBO.
//!
//! Receives a push request, runs the authority-guarded state machine
//! (accepted → pre-call → inflight → terminal), and returns one of the
//! `PushOutcome` taxonomy variants.  Per-entity routing is explicit —
//! no trait dispatch.

use std::future::Future;
use std::sync::Arc;

use serde::Serialize;
use serde_json::Value;
use sqlx::PgPool;
use uuid::Uuid;

use crate::domain::qbo::client::{
    QboClient, QboCustomerPayload, QboInvoicePayload, QboPaymentPayload,
};
use crate::domain::qbo::QboError;
use super::authority_repo;
use super::push_attempts::{self, PreCallOutcome, ReconcileOutcome};

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
    StaleAuthorityAutoClosed {
        attempt_id: Uuid,
        entity_id: String,
    },
    /// Write completed under stale authority; values diverged, a conflict row
    /// was opened for manual resolution.
    StaleAuthorityConflictOpened {
        attempt_id: Uuid,
        entity_id: String,
        conflict_id: Uuid,
    },
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
        QboError::Http(e) => QboCallResult::Unknown { message: e.to_string() },
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

    // 5. Execute the QBO call.
    let qbo_result = qbo_fn(attempt.id).await;

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
        QboCallResult::Succeeded { external_value, provider_entity_id } => {
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
                push_attempts::complete_attempt(pool, attempt.id, "succeeded", None)
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
            let _ = push_attempts::complete_attempt(pool, attempt.id, "failed", Some(&message))
                .await;
            Ok(PushOutcome::Failed {
                attempt_id: attempt.id,
                entity_id: entity_id.to_string(),
                error_code: code,
                error_message: message,
            })
        }
        QboCallResult::Unknown { message } => {
            let _ =
                push_attempts::complete_attempt(pool, attempt.id, "unknown_failure", Some(&message))
                    .await;
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
                    "create" => {
                        match serde_json::from_value::<QboCustomerPayload>(payload_owned) {
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
                        }
                    }
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
                    "create" => {
                        match serde_json::from_value::<QboInvoicePayload>(payload_owned) {
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
                        }
                    }
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
                        let sync_token =
                            payload_owned["SyncToken"].as_str().unwrap_or("0").to_string();
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
                    "create" => {
                        match serde_json::from_value::<QboPaymentPayload>(payload_owned) {
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
                        }
                    }
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
                        let sync_token =
                            payload_owned["SyncToken"].as_str().unwrap_or("").to_string();
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
