//! Contact role service — Guard→Mutation→Outbox atomicity.

use chrono::Utc;
use sqlx::PgPool;
use uuid::Uuid;

use crate::db::{
    contact_repo, contact_role_repo, contact_role_repo::UpdateContactRoleData, party_repo,
};
use crate::events::{
    build_contact_role_created_envelope, build_contact_role_updated_envelope, ContactRolePayload,
    EVENT_TYPE_CONTACT_ROLE_CREATED, EVENT_TYPE_CONTACT_ROLE_UPDATED,
};
use crate::outbox::enqueue_event_tx;

use super::contact_role::{ContactRole, CreateContactRoleRequest, UpdateContactRoleRequest};
use super::party::PartyError;

// ============================================================================
// Reads
// ============================================================================

/// List contact roles for a party, scoped to app_id.
pub async fn list_contact_roles(
    pool: &PgPool,
    app_id: &str,
    party_id: Uuid,
) -> Result<Vec<ContactRole>, PartyError> {
    contact_role_repo::list_contact_roles(pool, app_id, party_id).await
}

/// Get a single contact role by ID, scoped to app_id.
pub async fn get_contact_role(
    pool: &PgPool,
    app_id: &str,
    role_id: Uuid,
) -> Result<Option<ContactRole>, PartyError> {
    contact_role_repo::get_contact_role(pool, app_id, role_id).await
}

// ============================================================================
// Writes
// ============================================================================

/// Create a contact role. Idempotent via idempotency_key.
/// Enforces: only one primary per role_type per party.
/// Emits `party.contact_role.created` via the outbox.
pub async fn create_contact_role(
    pool: &PgPool,
    app_id: &str,
    party_id: Uuid,
    req: &CreateContactRoleRequest,
    correlation_id: String,
) -> Result<ContactRole, PartyError> {
    req.validate()?;

    // Idempotency check
    if let Some(ref idem_key) = req.idempotency_key {
        if let Some(existing) =
            contact_role_repo::find_contact_role_by_idempotency_key(pool, app_id, idem_key).await?
        {
            return Ok(existing);
        }
    }

    let role_id = Uuid::new_v4();
    let event_id = Uuid::new_v4();
    let now = Utc::now();
    let is_primary = req.is_primary.unwrap_or(false);

    let mut tx = pool.begin().await?;

    party_repo::guard_party_exists_tx(&mut tx, app_id, party_id).await?;
    contact_repo::guard_contact_belongs_to_party_tx(&mut tx, app_id, party_id, req.contact_id)
        .await?;

    if is_primary {
        contact_role_repo::clear_primary_for_role_type_tx(
            &mut tx,
            app_id,
            party_id,
            &req.role_type,
        )
        .await?;
    }

    let role = contact_role_repo::insert_contact_role_tx(
        &mut tx, role_id, party_id, app_id, req, is_primary, now,
    )
    .await?;

    let payload = ContactRolePayload {
        contact_role_id: role_id,
        party_id,
        contact_id: req.contact_id,
        app_id: app_id.to_string(),
        role_type: role.role_type.clone(),
        is_primary: role.is_primary,
    };

    let envelope = build_contact_role_created_envelope(
        event_id,
        app_id.to_string(),
        correlation_id,
        None,
        payload,
    );

    enqueue_event_tx(
        &mut tx,
        event_id,
        EVENT_TYPE_CONTACT_ROLE_CREATED,
        "contact_role",
        &role_id.to_string(),
        app_id,
        &envelope,
    )
    .await?;

    tx.commit().await?;
    Ok(role)
}

/// Update a contact role. Emits `party.contact_role.updated`.
pub async fn update_contact_role(
    pool: &PgPool,
    app_id: &str,
    role_id: Uuid,
    req: &UpdateContactRoleRequest,
    correlation_id: String,
) -> Result<ContactRole, PartyError> {
    req.validate()?;

    let event_id = Uuid::new_v4();
    let now = Utc::now();

    let mut tx = pool.begin().await?;

    let current = contact_role_repo::fetch_contact_role_for_update_tx(&mut tx, app_id, role_id)
        .await?
        .ok_or(PartyError::NotFound(role_id))?;

    let new_role_type = req
        .role_type
        .as_deref()
        .map(|r| r.trim().to_string())
        .unwrap_or(current.role_type);
    let new_primary = req.is_primary.unwrap_or(current.is_primary);
    let new_from = req.effective_from.unwrap_or(current.effective_from);
    let new_to = if req.effective_to.is_some() {
        req.effective_to
    } else {
        current.effective_to
    };
    let new_metadata = if req.metadata.is_some() {
        req.metadata.clone()
    } else {
        current.metadata
    };

    if new_primary && !current.is_primary {
        contact_role_repo::clear_primary_for_role_type_tx(
            &mut tx,
            app_id,
            current.party_id,
            &new_role_type,
        )
        .await?;
    }

    let updated = contact_role_repo::update_contact_role_row_tx(
        &mut tx,
        &UpdateContactRoleData {
            role_id,
            app_id,
            role_type: new_role_type,
            is_primary: new_primary,
            effective_from: new_from,
            effective_to: new_to,
            metadata: new_metadata,
            updated_at: now,
        },
    )
    .await?;

    let payload = ContactRolePayload {
        contact_role_id: role_id,
        party_id: updated.party_id,
        contact_id: updated.contact_id,
        app_id: app_id.to_string(),
        role_type: updated.role_type.clone(),
        is_primary: updated.is_primary,
    };

    let envelope = build_contact_role_updated_envelope(
        event_id,
        app_id.to_string(),
        correlation_id,
        None,
        payload,
    );

    enqueue_event_tx(
        &mut tx,
        event_id,
        EVENT_TYPE_CONTACT_ROLE_UPDATED,
        "contact_role",
        &role_id.to_string(),
        app_id,
        &envelope,
    )
    .await?;

    tx.commit().await?;
    Ok(updated)
}
