//! External refs service — Guard→Mutation→Outbox atomicity.
//!
//! Operations:
//! - create_external_ref: upsert on (app_id, system, external_id), emit external_ref.created
//! - update_external_ref: update label/metadata, emit external_ref.updated
//! - delete_external_ref: hard delete, emit external_ref.deleted
//! - get_external_ref: fetch by id scoped to app_id
//! - list_by_entity: all refs for a given entity_type + entity_id
//! - get_by_external: lookup by system + external_id

use chrono::Utc;
use sqlx::PgPool;
use uuid::Uuid;

use crate::events::{
    build_external_ref_created_envelope, build_external_ref_deleted_envelope,
    build_external_ref_updated_envelope, ExternalRefCreatedPayload, ExternalRefDeletedPayload,
    ExternalRefUpdatedPayload, EVENT_TYPE_EXTERNAL_REF_CREATED, EVENT_TYPE_EXTERNAL_REF_DELETED,
    EVENT_TYPE_EXTERNAL_REF_UPDATED,
};
use crate::outbox::enqueue_event_tx;

use super::guards::{validate_create, validate_update};
use super::models::{
    CreateExternalRefRequest, ExternalRef, ExternalRefError, UpdateExternalRefRequest,
};
use super::repo;

// ============================================================================
// Reads
// ============================================================================

/// Fetch a single external ref by id, scoped to app_id.
pub async fn get_external_ref(
    pool: &PgPool,
    app_id: &str,
    ref_id: i64,
) -> Result<Option<ExternalRef>, ExternalRefError> {
    Ok(repo::get_by_id(pool, app_id, ref_id).await?)
}

/// List all external refs for a given internal entity.
pub async fn list_by_entity(
    pool: &PgPool,
    app_id: &str,
    entity_type: &str,
    entity_id: &str,
) -> Result<Vec<ExternalRef>, ExternalRefError> {
    Ok(repo::list_by_entity(pool, app_id, entity_type, entity_id).await?)
}

/// Look up a ref by external system + external_id, scoped to app_id.
pub async fn get_by_external(
    pool: &PgPool,
    app_id: &str,
    system: &str,
    external_id: &str,
) -> Result<Option<ExternalRef>, ExternalRefError> {
    Ok(repo::get_by_external(pool, app_id, system, external_id).await?)
}

// ============================================================================
// Writes
// ============================================================================

/// Create or update an external ref.
///
/// Idempotent: on (app_id, system, external_id) conflict, updates label and
/// metadata in place. The entity_type and entity_id of the existing mapping
/// are preserved — to remap an external ID to a different entity, delete and
/// recreate.
///
/// Emits `external_ref.created` via the transactional outbox.
pub async fn create_external_ref(
    pool: &PgPool,
    app_id: &str,
    req: &CreateExternalRefRequest,
    correlation_id: String,
) -> Result<ExternalRef, ExternalRefError> {
    validate_create(req)?;

    let event_id = Uuid::new_v4();

    let mut tx = pool.begin().await?;

    // Mutation: upsert
    let row = repo::upsert(
        &mut tx,
        app_id,
        req.entity_type.trim(),
        req.entity_id.trim(),
        req.system.trim(),
        req.external_id.trim(),
        &req.label,
        &req.metadata,
    )
    .await?;

    // Outbox: external_ref.created
    let payload = ExternalRefCreatedPayload {
        ref_id: row.id,
        app_id: app_id.to_string(),
        entity_type: row.entity_type.clone(),
        entity_id: row.entity_id.clone(),
        system: row.system.clone(),
        external_id: row.external_id.clone(),
        label: row.label.clone(),
        created_at: row.created_at,
    };
    let envelope = build_external_ref_created_envelope(
        event_id,
        app_id.to_string(),
        correlation_id,
        None,
        payload,
    );
    enqueue_event_tx(
        &mut tx,
        event_id,
        EVENT_TYPE_EXTERNAL_REF_CREATED,
        "external_ref",
        &row.id.to_string(),
        app_id,
        &envelope,
    )
    .await?;

    tx.commit().await?;
    Ok(row)
}

/// Update label and/or metadata on an existing external ref.
///
/// Guard: ref must exist and belong to app_id.
/// Emits `external_ref.updated` via the transactional outbox.
pub async fn update_external_ref(
    pool: &PgPool,
    app_id: &str,
    ref_id: i64,
    req: &UpdateExternalRefRequest,
    correlation_id: String,
) -> Result<ExternalRef, ExternalRefError> {
    validate_update(req)?;

    let event_id = Uuid::new_v4();
    let now = Utc::now();

    let mut tx = pool.begin().await?;

    // Guard: fetch + lock
    let current = repo::fetch_for_update(&mut tx, ref_id, app_id)
        .await?
        .ok_or(ExternalRefError::NotFound(ref_id))?;

    let new_label = if req.label.is_some() {
        req.label.clone()
    } else {
        current.label.clone()
    };
    let new_meta = if req.metadata.is_some() {
        req.metadata.clone()
    } else {
        current.metadata.clone()
    };

    // Mutation
    let updated = repo::update(&mut tx, &new_label, &new_meta, now, ref_id, app_id).await?;

    // Outbox: external_ref.updated
    let payload = ExternalRefUpdatedPayload {
        ref_id: updated.id,
        app_id: app_id.to_string(),
        entity_type: updated.entity_type.clone(),
        entity_id: updated.entity_id.clone(),
        system: updated.system.clone(),
        external_id: updated.external_id.clone(),
        label: updated.label.clone(),
        updated_at: updated.updated_at,
    };
    let envelope = build_external_ref_updated_envelope(
        event_id,
        app_id.to_string(),
        correlation_id,
        None,
        payload,
    );
    enqueue_event_tx(
        &mut tx,
        event_id,
        EVENT_TYPE_EXTERNAL_REF_UPDATED,
        "external_ref",
        &ref_id.to_string(),
        app_id,
        &envelope,
    )
    .await?;

    tx.commit().await?;
    Ok(updated)
}

/// Hard-delete an external ref.
///
/// Guard: ref must exist and belong to app_id.
/// Emits `external_ref.deleted` via the transactional outbox.
pub async fn delete_external_ref(
    pool: &PgPool,
    app_id: &str,
    ref_id: i64,
    correlation_id: String,
) -> Result<(), ExternalRefError> {
    let event_id = Uuid::new_v4();
    let now = Utc::now();

    let mut tx = pool.begin().await?;

    // Guard: verify existence
    let current = repo::get_by_id_in_tx(&mut tx, ref_id, app_id)
        .await?
        .ok_or(ExternalRefError::NotFound(ref_id))?;

    // Mutation: delete
    repo::delete(&mut tx, ref_id, app_id).await?;

    // Outbox: external_ref.deleted
    let payload = ExternalRefDeletedPayload {
        ref_id: current.id,
        app_id: app_id.to_string(),
        entity_type: current.entity_type,
        entity_id: current.entity_id,
        system: current.system,
        external_id: current.external_id,
        deleted_at: now,
    };
    let envelope = build_external_ref_deleted_envelope(
        event_id,
        app_id.to_string(),
        correlation_id,
        None,
        payload,
    );
    enqueue_event_tx(
        &mut tx,
        event_id,
        EVENT_TYPE_EXTERNAL_REF_DELETED,
        "external_ref",
        &ref_id.to_string(),
        app_id,
        &envelope,
    )
    .await?;

    tx.commit().await?;
    Ok(())
}

// ============================================================================
// Integrated Tests (real DB)
// ============================================================================

#[cfg(test)]
#[path = "service_tests.rs"]
mod tests;
