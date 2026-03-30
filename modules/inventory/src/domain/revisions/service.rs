//! Revision write services: create, activate, and update policy.
//!
//! All writes follow Guard → Mutation → Outbox pattern in a single transaction.

use chrono::Duration;
use chrono::Utc;
use sqlx::PgPool;
use uuid::Uuid;

use crate::domain::history::change_history::{record_change_in_tx, RecordChangeRequest};
use crate::events::{
    build_item_revision_activated_envelope, build_item_revision_created_envelope,
    build_item_revision_policy_updated_envelope, ItemRevisionActivatedPayload,
    ItemRevisionCreatedPayload, ItemRevisionPolicyUpdatedPayload,
    EVENT_TYPE_ITEM_REVISION_ACTIVATED, EVENT_TYPE_ITEM_REVISION_CREATED,
    EVENT_TYPE_ITEM_REVISION_POLICY_UPDATED,
};

use super::models::{
    find_idempotency_key, guard_item_exists_active, insert_outbox_event,
    normalize_traceability_level, store_idempotency_key, validate_activate_request,
    validate_create_request, validate_update_policy_request, ActivateRevisionRequest,
    CreateRevisionRequest, ItemRevision, RevisionError, UpdateRevisionPolicyRequest,
};

/// Create a new item revision (draft state, not yet effective).
///
/// Pattern: Guard → Mutation → Outbox (single transaction).
/// Returns `(ItemRevision, is_replay)`.
pub async fn create_revision(
    pool: &PgPool,
    req: &CreateRevisionRequest,
) -> Result<(ItemRevision, bool), RevisionError> {
    // --- Guard: validate inputs ---
    validate_create_request(req)?;

    // --- Guard: idempotency check ---
    let request_hash = serde_json::to_string(req)?;
    if let Some(record) = find_idempotency_key(pool, &req.tenant_id, &req.idempotency_key).await? {
        if record.request_hash != request_hash {
            return Err(RevisionError::ConflictingIdempotencyKey);
        }
        let result: ItemRevision = serde_json::from_str(&record.response_body)?;
        return Ok((result, true));
    }

    // --- Guard: item must exist and be active ---
    guard_item_exists_active(pool, req.item_id, &req.tenant_id).await?;

    // --- Mutation + Outbox in single transaction ---
    let event_id = Uuid::new_v4();
    let now = Utc::now();
    let correlation_id = req
        .correlation_id
        .clone()
        .unwrap_or_else(|| Uuid::new_v4().to_string());

    let mut tx = pool.begin().await?;

    // Auto-increment revision_number
    let revision = sqlx::query_as::<_, ItemRevision>(
        r#"
        INSERT INTO item_revisions
            (tenant_id, item_id, revision_number,
             name, description, uom,
             inventory_account_ref, cogs_account_ref, variance_account_ref,
             traceability_level, inspection_required, shelf_life_days, shelf_life_enforced,
             change_reason, idempotency_key, created_at)
        VALUES
            ($1, $2,
             (SELECT COALESCE(MAX(revision_number), 0) + 1
              FROM item_revisions WHERE tenant_id = $1 AND item_id = $2),
             $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15)
        RETURNING *
        "#,
    )
    .bind(&req.tenant_id)
    .bind(req.item_id)
    .bind(req.name.trim())
    .bind(req.description.as_deref())
    .bind(req.uom.trim())
    .bind(req.inventory_account_ref.trim())
    .bind(req.cogs_account_ref.trim())
    .bind(req.variance_account_ref.trim())
    .bind(normalize_traceability_level(&req.traceability_level))
    .bind(req.inspection_required)
    .bind(req.shelf_life_days)
    .bind(req.shelf_life_enforced)
    .bind(req.change_reason.trim())
    .bind(&req.idempotency_key)
    .bind(now)
    .fetch_one(&mut *tx)
    .await
    .map_err(|e| {
        if let sqlx::Error::Database(ref dbe) = e {
            if dbe.code().as_deref() == Some("23505") {
                return RevisionError::ConflictingIdempotencyKey;
            }
        }
        RevisionError::Database(e)
    })?;

    // Outbox event
    let payload = ItemRevisionCreatedPayload {
        revision_id: revision.id,
        tenant_id: req.tenant_id.clone(),
        item_id: req.item_id,
        revision_number: revision.revision_number,
        name: revision.name.clone(),
        uom: revision.uom.clone(),
        traceability_level: revision.traceability_level.clone(),
        inspection_required: revision.inspection_required,
        shelf_life_days: revision.shelf_life_days,
        shelf_life_enforced: revision.shelf_life_enforced,
        change_reason: revision.change_reason.clone(),
        created_at: now,
    };
    let envelope = build_item_revision_created_envelope(
        event_id,
        req.tenant_id.clone(),
        correlation_id.clone(),
        req.causation_id.clone(),
        payload,
    );
    let envelope_json = serde_json::to_string(&envelope)?;

    insert_outbox_event(
        &mut tx, event_id, EVENT_TYPE_ITEM_REVISION_CREATED,
        &revision.id.to_string(), &req.tenant_id, &envelope_json,
        &correlation_id, &req.causation_id,
    ).await?;

    // Change history
    let actor = req.actor_id.clone().unwrap_or_else(|| "system".to_string());
    let diff = serde_json::json!({
        "name": { "after": revision.name },
        "uom": { "after": revision.uom },
        "inventory_account_ref": { "after": revision.inventory_account_ref },
        "cogs_account_ref": { "after": revision.cogs_account_ref },
        "variance_account_ref": { "after": revision.variance_account_ref },
        "traceability_level": { "after": revision.traceability_level },
        "inspection_required": { "after": revision.inspection_required },
        "shelf_life_days": { "after": revision.shelf_life_days },
        "shelf_life_enforced": { "after": revision.shelf_life_enforced },
    });
    let change_req = RecordChangeRequest {
        tenant_id: req.tenant_id.clone(),
        item_id: req.item_id,
        revision_id: Some(revision.id),
        change_type: "revision_created".to_string(),
        actor_id: actor,
        diff,
        reason: Some(req.change_reason.clone()),
        idempotency_key: format!("ch-{}", req.idempotency_key),
        correlation_id: Some(correlation_id.clone()),
        causation_id: req.causation_id.clone(),
    };
    record_change_in_tx(&mut tx, &change_req)
        .await
        .map_err(|e| RevisionError::Database(sqlx::Error::Protocol(e.to_string())))?;

    // Idempotency key
    let response_json = serde_json::to_string(&revision)?;
    store_idempotency_key(
        &mut tx, &req.tenant_id, &req.idempotency_key, &request_hash,
        &response_json, 201, now + Duration::days(7),
    ).await?;

    tx.commit().await?;
    Ok((revision, false))
}

/// Activate a revision for an effective window.
///
/// Automatically closes any currently open-ended revision for the same item
/// by setting its effective_to = this revision's effective_from.
///
/// Pattern: Guard → Mutation → Outbox (single transaction).
/// Returns `(ItemRevision, is_replay)`.
pub async fn activate_revision(
    pool: &PgPool,
    item_id: Uuid,
    revision_id: Uuid,
    req: &ActivateRevisionRequest,
) -> Result<(ItemRevision, bool), RevisionError> {
    // --- Guard: validate inputs ---
    validate_activate_request(req)?;

    // --- Guard: idempotency check ---
    let request_hash = serde_json::to_string(req)?;
    if let Some(record) = find_idempotency_key(pool, &req.tenant_id, &req.idempotency_key).await? {
        if record.request_hash != request_hash {
            return Err(RevisionError::ConflictingIdempotencyKey);
        }
        let result: ItemRevision = serde_json::from_str(&record.response_body)?;
        return Ok((result, true));
    }

    // --- Guard: item must exist and be active ---
    guard_item_exists_active(pool, item_id, &req.tenant_id).await?;

    // --- Mutation + Outbox in single transaction ---
    let event_id = Uuid::new_v4();
    let now = Utc::now();
    let correlation_id = req
        .correlation_id
        .clone()
        .unwrap_or_else(|| Uuid::new_v4().to_string());

    let mut tx = pool.begin().await?;

    // Lock the revision row
    let revision = sqlx::query_as::<_, ItemRevision>(
        r#"
        SELECT * FROM item_revisions
        WHERE id = $1 AND item_id = $2 AND tenant_id = $3
        FOR UPDATE
        "#,
    )
    .bind(revision_id)
    .bind(item_id)
    .bind(&req.tenant_id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(RevisionError::RevisionNotFound)?;

    if revision.effective_from.is_some() {
        return Err(RevisionError::AlreadyActivated);
    }

    // Close any open-ended predecessor revision for this item
    let superseded_id: Option<Uuid> = sqlx::query_scalar(
        r#"
        UPDATE item_revisions
        SET effective_to = $1, activated_at = COALESCE(activated_at, NOW())
        WHERE tenant_id = $2 AND item_id = $3
          AND effective_from IS NOT NULL AND effective_to IS NULL
          AND id != $4
        RETURNING id
        "#,
    )
    .bind(req.effective_from)
    .bind(&req.tenant_id)
    .bind(item_id)
    .bind(revision_id)
    .fetch_optional(&mut *tx)
    .await?;

    // Activate this revision
    let activated = sqlx::query_as::<_, ItemRevision>(
        r#"
        UPDATE item_revisions
        SET effective_from = $1, effective_to = $2, activated_at = $3
        WHERE id = $4 AND tenant_id = $5
        RETURNING *
        "#,
    )
    .bind(req.effective_from)
    .bind(req.effective_to)
    .bind(now)
    .bind(revision_id)
    .bind(&req.tenant_id)
    .fetch_one(&mut *tx)
    .await
    .map_err(|e| {
        if let sqlx::Error::Database(ref dbe) = e {
            // Exclusion constraint violation (overlapping windows)
            if dbe.code().as_deref() == Some("23P01") {
                return RevisionError::OverlappingWindow;
            }
        }
        RevisionError::Database(e)
    })?;

    // Outbox event
    let payload = ItemRevisionActivatedPayload {
        revision_id: activated.id,
        tenant_id: req.tenant_id.clone(),
        item_id,
        revision_number: activated.revision_number,
        traceability_level: activated.traceability_level.clone(),
        inspection_required: activated.inspection_required,
        shelf_life_days: activated.shelf_life_days,
        shelf_life_enforced: activated.shelf_life_enforced,
        effective_from: req.effective_from,
        effective_to: req.effective_to,
        superseded_revision_id: superseded_id,
        activated_at: now,
    };
    let envelope = build_item_revision_activated_envelope(
        event_id,
        req.tenant_id.clone(),
        correlation_id.clone(),
        req.causation_id.clone(),
        payload,
    );
    let envelope_json = serde_json::to_string(&envelope)?;

    insert_outbox_event(
        &mut tx, event_id, EVENT_TYPE_ITEM_REVISION_ACTIVATED,
        &activated.id.to_string(), &req.tenant_id, &envelope_json,
        &correlation_id, &req.causation_id,
    ).await?;

    // Change history
    let actor = req.actor_id.clone().unwrap_or_else(|| "system".to_string());
    let diff = serde_json::json!({
        "effective_from": { "before": null, "after": req.effective_from },
        "effective_to": { "before": null, "after": req.effective_to },
        "superseded_revision_id": { "after": superseded_id },
    });
    let change_req = RecordChangeRequest {
        tenant_id: req.tenant_id.clone(),
        item_id,
        revision_id: Some(activated.id),
        change_type: "revision_activated".to_string(),
        actor_id: actor,
        diff,
        reason: None,
        idempotency_key: format!("ch-{}", req.idempotency_key),
        correlation_id: Some(correlation_id.clone()),
        causation_id: req.causation_id.clone(),
    };
    record_change_in_tx(&mut tx, &change_req)
        .await
        .map_err(|e| RevisionError::Database(sqlx::Error::Protocol(e.to_string())))?;

    // Idempotency key
    let response_json = serde_json::to_string(&activated)?;
    store_idempotency_key(
        &mut tx, &req.tenant_id, &req.idempotency_key, &request_hash,
        &response_json, 200, now + Duration::days(7),
    ).await?;

    tx.commit().await?;
    Ok((activated, false))
}

/// Update policy flags on a draft revision.
///
/// Activated revisions are immutable; create a new revision to change policy.
/// Pattern: Guard → Mutation → Outbox (single transaction).
/// Returns `(ItemRevision, is_replay)`.
pub async fn update_revision_policy(
    pool: &PgPool,
    item_id: Uuid,
    revision_id: Uuid,
    req: &UpdateRevisionPolicyRequest,
) -> Result<(ItemRevision, bool), RevisionError> {
    validate_update_policy_request(req)?;

    let request_hash = serde_json::to_string(req)?;
    if let Some(record) = find_idempotency_key(pool, &req.tenant_id, &req.idempotency_key).await? {
        if record.request_hash != request_hash {
            return Err(RevisionError::ConflictingIdempotencyKey);
        }
        let result: ItemRevision = serde_json::from_str(&record.response_body)?;
        return Ok((result, true));
    }

    guard_item_exists_active(pool, item_id, &req.tenant_id).await?;

    let event_id = Uuid::new_v4();
    let now = Utc::now();
    let correlation_id = req
        .correlation_id
        .clone()
        .unwrap_or_else(|| Uuid::new_v4().to_string());

    let mut tx = pool.begin().await?;

    let current = sqlx::query_as::<_, ItemRevision>(
        r#"
        SELECT * FROM item_revisions
        WHERE id = $1 AND item_id = $2 AND tenant_id = $3
        FOR UPDATE
        "#,
    )
    .bind(revision_id)
    .bind(item_id)
    .bind(&req.tenant_id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(RevisionError::RevisionNotFound)?;

    if current.effective_from.is_some() {
        return Err(RevisionError::PolicyLockedOnActivatedRevision);
    }

    let updated = sqlx::query_as::<_, ItemRevision>(
        r#"
        UPDATE item_revisions
        SET traceability_level = $1,
            inspection_required = $2,
            shelf_life_days = $3,
            shelf_life_enforced = $4
        WHERE id = $5 AND tenant_id = $6
        RETURNING *
        "#,
    )
    .bind(normalize_traceability_level(&req.traceability_level))
    .bind(req.inspection_required)
    .bind(req.shelf_life_days)
    .bind(req.shelf_life_enforced)
    .bind(revision_id)
    .bind(&req.tenant_id)
    .fetch_one(&mut *tx)
    .await?;

    let payload = ItemRevisionPolicyUpdatedPayload {
        revision_id: updated.id,
        tenant_id: req.tenant_id.clone(),
        item_id,
        revision_number: updated.revision_number,
        traceability_level: updated.traceability_level.clone(),
        inspection_required: updated.inspection_required,
        shelf_life_days: updated.shelf_life_days,
        shelf_life_enforced: updated.shelf_life_enforced,
        updated_at: now,
    };
    let envelope = build_item_revision_policy_updated_envelope(
        event_id,
        req.tenant_id.clone(),
        correlation_id.clone(),
        req.causation_id.clone(),
        payload,
    );
    let envelope_json = serde_json::to_string(&envelope)?;

    insert_outbox_event(
        &mut tx, event_id, EVENT_TYPE_ITEM_REVISION_POLICY_UPDATED,
        &updated.id.to_string(), &req.tenant_id, &envelope_json,
        &correlation_id, &req.causation_id,
    ).await?;

    // Change history
    let actor = req.actor_id.clone().unwrap_or_else(|| "system".to_string());
    let diff = serde_json::json!({
        "traceability_level": {
            "before": current.traceability_level,
            "after": updated.traceability_level,
        },
        "inspection_required": {
            "before": current.inspection_required,
            "after": updated.inspection_required,
        },
        "shelf_life_days": {
            "before": current.shelf_life_days,
            "after": updated.shelf_life_days,
        },
        "shelf_life_enforced": {
            "before": current.shelf_life_enforced,
            "after": updated.shelf_life_enforced,
        },
    });
    let change_req = RecordChangeRequest {
        tenant_id: req.tenant_id.clone(),
        item_id,
        revision_id: Some(updated.id),
        change_type: "policy_updated".to_string(),
        actor_id: actor,
        diff,
        reason: None,
        idempotency_key: format!("ch-{}", req.idempotency_key),
        correlation_id: Some(correlation_id.clone()),
        causation_id: req.causation_id.clone(),
    };
    record_change_in_tx(&mut tx, &change_req)
        .await
        .map_err(|e| RevisionError::Database(sqlx::Error::Protocol(e.to_string())))?;

    let response_json = serde_json::to_string(&updated)?;
    store_idempotency_key(
        &mut tx, &req.tenant_id, &req.idempotency_key, &request_hash,
        &response_json, 200, now + Duration::days(7),
    ).await?;

    tx.commit().await?;
    Ok((updated, false))
}
