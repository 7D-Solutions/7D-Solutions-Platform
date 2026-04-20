//! Customer push handler: create/update/deactivate operations against QBO.
//!
//! All paths run through the push-attempt state machine
//! (insert → authority version check → inflight → complete) so every write is
//! auditable and idempotent across transport-layer retries.
//!
//! Invariant: the authority version is re-read from the authority table
//! immediately before dispatch, so a flip that happened after the push was
//! enqueued always causes the attempt to be superseded rather than dispatched.
//!
//! ## Duplicate-customer remap policy
//!
//! When a new external QBO customer appears that shares normalized fields
//! (email / phone / tax_id) with an internal entity that already has an
//! `external_ref` mapping, the platform MUST NOT auto-remap.  Instead:
//!
//! 1. Call `raise_creation_conflict` — opens a pending `creation` conflict
//!    and embeds deterministic candidate hints in `internal_value`.
//! 2. An admin resolves the conflict by calling `execute_customer_remap`,
//!    which atomically tombstones the old mapping and installs the new one.
//!
//! No fuzzy name-similarity logic is used at any step.

use serde_json::{json, Value};
use sqlx::PgPool;
use uuid::Uuid;

use crate::domain::qbo::{
    client::{QboClient, QboCustomerPayload},
    QboError,
};
use crate::domain::external_refs::repo as ext_repo;
use crate::events::{
    build_sync_conflict_detected_envelope, build_sync_conflict_resolved_envelope,
    SyncConflictDetectedPayload, SyncConflictResolvedPayload,
    EVENT_TYPE_SYNC_CONFLICT_DETECTED, EVENT_TYPE_SYNC_CONFLICT_RESOLVED,
};
use crate::outbox::enqueue_event_tx;
use super::{
    authority_repo,
    conflicts::{ConflictClass, ConflictRow, ConflictStatus},
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

// ── Duplicate-customer remap policy ──────────────────────────────────────────
//
// Policy invariant: a new external QBO customer that shares normalized fields
// (email / phone / tax_id) with an internally-mapped entity NEVER triggers
// automatic remapping.  All remaps are explicit, admin-initiated, and auditable.

// ── Normalization helpers ─────────────────────────────────────────────────────

/// Normalize an email address for deterministic comparison.
pub fn normalize_email(s: &str) -> String {
    s.trim().to_lowercase()
}

/// Normalize a phone number to digits only for deterministic comparison.
pub fn normalize_phone(s: &str) -> String {
    s.chars().filter(|c| c.is_ascii_digit()).collect()
}

/// Normalize a tax/EIN ID: keep alphanumeric characters, uppercase.
pub fn normalize_tax_id(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_alphanumeric())
        .collect::<String>()
        .to_uppercase()
}

// ── Creation conflict request / outcome ──────────────────────────────────────

/// One candidate internal entity returned as a deterministic hint.
#[derive(Debug, Clone, serde::Serialize)]
pub struct CandidateHint {
    pub entity_id: String,
    pub external_id: String,
    pub matched_on: Vec<String>,
}

/// Input for `raise_creation_conflict`.
pub struct CustomerCreationConflictRequest {
    pub app_id: String,
    pub provider: String,
    pub entity_type: String,
    /// The internal entity ID whose stale mapping triggered this conflict.
    pub entity_id: String,
    /// The new external (QBO) customer ID that just appeared.
    pub new_external_id: String,
    /// Pre-normalized email for candidate matching (use `normalize_email`).
    pub normalized_email: Option<String>,
    /// Pre-normalized phone for candidate matching (use `normalize_phone`).
    pub normalized_phone: Option<String>,
    /// Pre-normalized tax/EIN for candidate matching (use `normalize_tax_id`).
    pub normalized_tax_id: Option<String>,
    /// Full snapshot of the new external customer (stored as conflict external_value).
    pub external_value: Value,
    /// Full snapshot of the current internal entity state (augmented with candidate
    /// hints before being stored as conflict internal_value).
    pub internal_value: Value,
}

/// Result of `raise_creation_conflict`.
pub struct CreationConflictOutcome {
    pub conflict: ConflictRow,
    /// Deterministic candidate hints embedded in the conflict's internal_value.
    pub candidates: Vec<CandidateHint>,
}

#[derive(Debug, thiserror::Error)]
pub enum CustomerCreationConflictError {
    #[error("value blob exceeds 256 KB limit")]
    ValueTooLarge,
    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),
}

/// Raise a `creation` conflict for the stale-ref + new-external-customer scenario.
///
/// Finds deterministic candidate hints by querying for external_refs whose
/// metadata contains matching normalized email / phone / tax_id values.
/// No fuzzy name matching occurs.
///
/// Atomically inserts the conflict row and enqueues a `sync.conflict.detected`
/// event via the transactional outbox.
pub async fn raise_creation_conflict(
    pool: &PgPool,
    req: &CustomerCreationConflictRequest,
) -> Result<CreationConflictOutcome, CustomerCreationConflictError> {
    use super::conflicts::MAX_VALUE_BYTES;

    if req.external_value.to_string().len() > MAX_VALUE_BYTES {
        return Err(CustomerCreationConflictError::ValueTooLarge);
    }
    if req.internal_value.to_string().len() > MAX_VALUE_BYTES {
        return Err(CustomerCreationConflictError::ValueTooLarge);
    }

    // Deterministic candidate hints: exact normalized field matches only.
    let candidate_refs = ext_repo::find_candidates_by_normalized_fields(
        pool,
        &req.app_id,
        &req.entity_type,
        &req.provider,
        req.normalized_email.as_deref(),
        req.normalized_phone.as_deref(),
        req.normalized_tax_id.as_deref(),
        20,
    )
    .await
    .map_err(CustomerCreationConflictError::Database)?;

    let candidates: Vec<CandidateHint> = candidate_refs
        .iter()
        .map(|r| {
            let mut matched_on = Vec::new();
            if let Some(em) = &req.normalized_email {
                if r.metadata
                    .as_ref()
                    .and_then(|m| m["normalized_email"].as_str())
                    == Some(em.as_str())
                {
                    matched_on.push("email".to_string());
                }
            }
            if let Some(ph) = &req.normalized_phone {
                if r.metadata
                    .as_ref()
                    .and_then(|m| m["normalized_phone"].as_str())
                    == Some(ph.as_str())
                {
                    matched_on.push("phone".to_string());
                }
            }
            if let Some(tx) = &req.normalized_tax_id {
                if r.metadata
                    .as_ref()
                    .and_then(|m| m["normalized_tax_id"].as_str())
                    == Some(tx.as_str())
                {
                    matched_on.push("tax_id".to_string());
                }
            }
            CandidateHint {
                entity_id: r.entity_id.clone(),
                external_id: r.external_id.clone(),
                matched_on,
            }
        })
        .collect();

    // Augment internal_value with candidate hints JSON.
    let mut internal_value = req.internal_value.clone();
    if let Some(obj) = internal_value.as_object_mut() {
        obj.insert(
            "candidate_hints".to_string(),
            serde_json::to_value(&candidates).unwrap_or(Value::Array(vec![])),
        );
    }

    let event_id = Uuid::new_v4();
    let mut tx = pool.begin().await.map_err(CustomerCreationConflictError::Database)?;

    let conflict = sqlx::query_as::<_, ConflictRow>(
        r#"
        INSERT INTO integrations_sync_conflicts (
            app_id, provider, entity_type, entity_id,
            conflict_class, detected_by,
            internal_value, external_value
        )
        VALUES ($1, $2, $3, $4, 'creation', 'customer_resolver', $5, $6)
        RETURNING
            id, app_id, provider, entity_type, entity_id,
            conflict_class, status, detected_by, detected_at,
            internal_value, external_value, internal_id,
            resolved_by, resolved_at, resolution_note,
            created_at, updated_at
        "#,
    )
    .bind(&req.app_id)
    .bind(&req.provider)
    .bind(&req.entity_type)
    .bind(&req.entity_id)
    .bind(&internal_value)
    .bind(&req.external_value)
    .fetch_one(&mut *tx)
    .await
    .map_err(CustomerCreationConflictError::Database)?;

    let detected_payload = SyncConflictDetectedPayload {
        app_id: req.app_id.clone(),
        conflict_id: conflict.id,
        provider: req.provider.clone(),
        entity_type: req.entity_type.clone(),
        entity_id: req.entity_id.clone(),
        conflict_class: ConflictClass::Creation.as_str().to_string(),
        detected_by: "customer_resolver".to_string(),
    };
    let envelope = build_sync_conflict_detected_envelope(
        event_id,
        req.app_id.clone(),
        event_id.to_string(),
        None,
        detected_payload,
    );
    let _ = enqueue_event_tx(
        &mut tx,
        event_id,
        EVENT_TYPE_SYNC_CONFLICT_DETECTED,
        "sync_conflict",
        &conflict.id.to_string(),
        &req.app_id,
        &envelope,
    )
    .await;

    tx.commit().await.map_err(CustomerCreationConflictError::Database)?;

    Ok(CreationConflictOutcome { conflict, candidates })
}

// ── Remap request / outcome ───────────────────────────────────────────────────

/// Input for `execute_customer_remap`.
pub struct CustomerRemapRequest {
    pub app_id: String,
    pub provider: String,
    pub entity_type: String,
    /// The pending creation conflict to resolve.
    pub conflict_id: Uuid,
    /// Row ID of the stale external_ref to tombstone.
    pub old_ref_id: i64,
    /// New QBO customer ID to link the internal entity to.
    pub new_external_id: String,
    pub resolved_by: String,
    pub resolution_note: Option<String>,
}

#[derive(Debug)]
pub struct CustomerRemapOutcome {
    pub resolved_conflict: ConflictRow,
    /// The new external_ref row created for the remapped entity.
    pub new_ref_id: i64,
}

#[derive(Debug, thiserror::Error)]
pub enum CustomerRemapError {
    #[error("conflict not found or not accessible")]
    ConflictNotFound,
    #[error("conflict must be class=creation and status=pending; got class={0} status={1}")]
    InvalidConflictState(String, String),
    #[error("stale external_ref {0} not found")]
    RefNotFound(i64),
    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),
}

/// Execute an explicit customer remap.
///
/// In a single transaction:
/// 1. Lock and validate the conflict (must be creation class + pending status).
/// 2. Tombstone the old external_ref (marks metadata, retains row for audit).
/// 3. Upsert a new external_ref for the same entity → new_external_id.
/// 4. Resolve the conflict (internal_id = entity_id of the remapped entity).
/// 5. Enqueue `sync.conflict.resolved` via the outbox.
pub async fn execute_customer_remap(
    pool: &PgPool,
    req: &CustomerRemapRequest,
) -> Result<CustomerRemapOutcome, CustomerRemapError> {
    let mut tx = pool.begin().await.map_err(CustomerRemapError::Database)?;

    // 1. Fetch and lock the conflict.
    let conflict = sqlx::query_as::<_, ConflictRow>(
        r#"
        SELECT id, app_id, provider, entity_type, entity_id,
               conflict_class, status, detected_by, detected_at,
               internal_value, external_value, internal_id,
               resolved_by, resolved_at, resolution_note,
               created_at, updated_at
        FROM integrations_sync_conflicts
        WHERE id = $1 AND app_id = $2
        FOR UPDATE
        "#,
    )
    .bind(req.conflict_id)
    .bind(&req.app_id)
    .fetch_optional(&mut *tx)
    .await
    .map_err(CustomerRemapError::Database)?
    .ok_or(CustomerRemapError::ConflictNotFound)?;

    if conflict.conflict_class != ConflictClass::Creation.as_str()
        || ConflictStatus::from_str(&conflict.status) != Some(ConflictStatus::Pending)
    {
        return Err(CustomerRemapError::InvalidConflictState(
            conflict.conflict_class.clone(),
            conflict.status.clone(),
        ));
    }

    // 2. Tombstone the old external_ref.
    ext_repo::tombstone_in_tx(&mut tx, req.old_ref_id, &req.app_id, &req.new_external_id)
        .await
        .map_err(|_| CustomerRemapError::RefNotFound(req.old_ref_id))?;

    // Fetch the stale ref's entity details so we can create the new mapping.
    let entity_id = conflict.entity_id.clone();

    // 3. Upsert the new external_ref (entity → new_external_id).
    let new_ref = sqlx::query_as::<_, crate::domain::external_refs::models::ExternalRef>(
        r#"
        INSERT INTO integrations_external_refs
            (app_id, entity_type, entity_id, system, external_id,
             label, metadata, created_at, updated_at)
        VALUES ($1, $2, $3, $4, $5, 'active', NULL, NOW(), NOW())
        ON CONFLICT (app_id, system, external_id) DO UPDATE SET
            entity_type = EXCLUDED.entity_type,
            entity_id   = EXCLUDED.entity_id,
            label       = EXCLUDED.label,
            updated_at  = NOW()
        RETURNING id, app_id, entity_type, entity_id, system, external_id,
                  label, metadata, created_at, updated_at
        "#,
    )
    .bind(&req.app_id)
    .bind(&req.entity_type)
    .bind(&entity_id)
    .bind(&req.provider)
    .bind(&req.new_external_id)
    .fetch_one(&mut *tx)
    .await
    .map_err(CustomerRemapError::Database)?;

    // 4. Resolve the conflict.
    let resolved = sqlx::query_as::<_, ConflictRow>(
        r#"
        UPDATE integrations_sync_conflicts
        SET status          = 'resolved',
            internal_id     = $3,
            resolved_by     = $4,
            resolved_at     = NOW(),
            resolution_note = $5,
            updated_at      = NOW()
        WHERE id = $1 AND app_id = $2 AND status = 'pending'
        RETURNING
            id, app_id, provider, entity_type, entity_id,
            conflict_class, status, detected_by, detected_at,
            internal_value, external_value, internal_id,
            resolved_by, resolved_at, resolution_note,
            created_at, updated_at
        "#,
    )
    .bind(req.conflict_id)
    .bind(&req.app_id)
    .bind(&entity_id)
    .bind(&req.resolved_by)
    .bind(&req.resolution_note)
    .fetch_optional(&mut *tx)
    .await
    .map_err(CustomerRemapError::Database)?
    .ok_or(CustomerRemapError::ConflictNotFound)?;

    // 5. Enqueue sync.conflict.resolved event.
    let event_id = Uuid::new_v4();
    let resolved_payload = SyncConflictResolvedPayload {
        app_id: req.app_id.clone(),
        conflict_id: req.conflict_id,
        provider: req.provider.clone(),
        entity_type: req.entity_type.clone(),
        entity_id: entity_id.clone(),
        conflict_class: ConflictClass::Creation.as_str().to_string(),
        resolved_by: req.resolved_by.clone(),
        internal_id: entity_id.clone(),
        resolution_note: req.resolution_note.clone(),
    };
    let envelope = build_sync_conflict_resolved_envelope(
        event_id,
        req.app_id.clone(),
        event_id.to_string(),
        None,
        resolved_payload,
    );
    let _ = enqueue_event_tx(
        &mut tx,
        event_id,
        EVENT_TYPE_SYNC_CONFLICT_RESOLVED,
        "sync_conflict",
        &req.conflict_id.to_string(),
        &req.app_id,
        &envelope,
    )
    .await;

    tx.commit().await.map_err(CustomerRemapError::Database)?;

    Ok(CustomerRemapOutcome {
        resolved_conflict: resolved,
        new_ref_id: new_ref.id,
    })
}
