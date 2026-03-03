//! Contact role service — Guard→Mutation→Outbox atomicity.

use chrono::Utc;
use sqlx::PgPool;
use uuid::Uuid;

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
    let rows: Vec<ContactRole> = sqlx::query_as(
        r#"
        SELECT id, party_id, contact_id, app_id, role_type, is_primary,
               effective_from, effective_to, idempotency_key, metadata,
               created_at, updated_at
        FROM party_contact_roles
        WHERE party_id = $1 AND app_id = $2
        ORDER BY role_type ASC, is_primary DESC
        "#,
    )
    .bind(party_id)
    .bind(app_id)
    .fetch_all(pool)
    .await?;

    Ok(rows)
}

/// Get a single contact role by ID, scoped to app_id.
pub async fn get_contact_role(
    pool: &PgPool,
    app_id: &str,
    role_id: Uuid,
) -> Result<Option<ContactRole>, PartyError> {
    let row: Option<ContactRole> = sqlx::query_as(
        r#"
        SELECT id, party_id, contact_id, app_id, role_type, is_primary,
               effective_from, effective_to, idempotency_key, metadata,
               created_at, updated_at
        FROM party_contact_roles
        WHERE id = $1 AND app_id = $2
        "#,
    )
    .bind(role_id)
    .bind(app_id)
    .fetch_optional(pool)
    .await?;

    Ok(row)
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
        let existing: Option<ContactRole> = sqlx::query_as(
            r#"
            SELECT id, party_id, contact_id, app_id, role_type, is_primary,
                   effective_from, effective_to, idempotency_key, metadata,
                   created_at, updated_at
            FROM party_contact_roles
            WHERE app_id = $1 AND idempotency_key = $2
            "#,
        )
        .bind(app_id)
        .bind(idem_key)
        .fetch_optional(pool)
        .await?;

        if let Some(existing) = existing {
            return Ok(existing);
        }
    }

    let role_id = Uuid::new_v4();
    let event_id = Uuid::new_v4();
    let now = Utc::now();
    let is_primary = req.is_primary.unwrap_or(false);

    let mut tx = pool.begin().await?;

    // Guard: party must exist
    guard_party_exists_tx(&mut tx, app_id, party_id).await?;

    // Guard: contact must exist and belong to this party
    guard_contact_belongs_to_party(&mut tx, app_id, party_id, req.contact_id).await?;

    // If marking as primary, clear existing primary for this role_type
    if is_primary {
        clear_primary_for_role_type(&mut tx, app_id, party_id, &req.role_type).await?;
    }

    // Mutation
    let role: ContactRole = sqlx::query_as(
        r#"
        INSERT INTO party_contact_roles (
            id, party_id, contact_id, app_id, role_type, is_primary,
            effective_from, effective_to, idempotency_key, metadata,
            created_at, updated_at
        )
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $11)
        RETURNING id, party_id, contact_id, app_id, role_type, is_primary,
                  effective_from, effective_to, idempotency_key, metadata,
                  created_at, updated_at
        "#,
    )
    .bind(role_id)
    .bind(party_id)
    .bind(req.contact_id)
    .bind(app_id)
    .bind(req.role_type.trim())
    .bind(is_primary)
    .bind(req.effective_from)
    .bind(req.effective_to)
    .bind(&req.idempotency_key)
    .bind(&req.metadata)
    .bind(now)
    .fetch_one(&mut *tx)
    .await?;

    // Outbox
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

    // Guard
    let existing: Option<ContactRole> = sqlx::query_as(
        r#"
        SELECT id, party_id, contact_id, app_id, role_type, is_primary,
               effective_from, effective_to, idempotency_key, metadata,
               created_at, updated_at
        FROM party_contact_roles
        WHERE id = $1 AND app_id = $2
        FOR UPDATE
        "#,
    )
    .bind(role_id)
    .bind(app_id)
    .fetch_optional(&mut *tx)
    .await?;

    let current = existing.ok_or(PartyError::NotFound(role_id))?;

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

    // If newly marking as primary, clear existing primary for this role_type
    if new_primary && !current.is_primary {
        clear_primary_for_role_type(&mut tx, app_id, current.party_id, &new_role_type).await?;
    }

    // Mutation
    let updated: ContactRole = sqlx::query_as(
        r#"
        UPDATE party_contact_roles
        SET role_type = $1, is_primary = $2, effective_from = $3,
            effective_to = $4, metadata = $5, updated_at = $6
        WHERE id = $7 AND app_id = $8
        RETURNING id, party_id, contact_id, app_id, role_type, is_primary,
                  effective_from, effective_to, idempotency_key, metadata,
                  created_at, updated_at
        "#,
    )
    .bind(&new_role_type)
    .bind(new_primary)
    .bind(new_from)
    .bind(new_to)
    .bind(&new_metadata)
    .bind(now)
    .bind(role_id)
    .bind(app_id)
    .fetch_one(&mut *tx)
    .await?;

    // Outbox
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

// ============================================================================
// Helpers
// ============================================================================

async fn guard_party_exists_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    app_id: &str,
    party_id: Uuid,
) -> Result<(), PartyError> {
    let exists: Option<(Uuid,)> =
        sqlx::query_as("SELECT id FROM party_parties WHERE id = $1 AND app_id = $2")
            .bind(party_id)
            .bind(app_id)
            .fetch_optional(&mut **tx)
            .await?;

    if exists.is_none() {
        return Err(PartyError::NotFound(party_id));
    }
    Ok(())
}

async fn guard_contact_belongs_to_party(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    app_id: &str,
    party_id: Uuid,
    contact_id: Uuid,
) -> Result<(), PartyError> {
    let exists: Option<(Uuid,)> = sqlx::query_as(
        "SELECT id FROM party_contacts WHERE id = $1 AND party_id = $2 AND app_id = $3",
    )
    .bind(contact_id)
    .bind(party_id)
    .bind(app_id)
    .fetch_optional(&mut **tx)
    .await?;

    if exists.is_none() {
        return Err(PartyError::NotFound(contact_id));
    }
    Ok(())
}

async fn clear_primary_for_role_type(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    app_id: &str,
    party_id: Uuid,
    role_type: &str,
) -> Result<(), PartyError> {
    sqlx::query(
        r#"
        UPDATE party_contact_roles
        SET is_primary = false
        WHERE party_id = $1 AND app_id = $2 AND role_type = $3 AND is_primary = true
        "#,
    )
    .bind(party_id)
    .bind(app_id)
    .bind(role_type)
    .execute(&mut **tx)
    .await?;
    Ok(())
}
