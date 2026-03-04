//! Contact CRUD service — Guard→Mutation→Outbox atomicity.
//!
//! All mutations emit events via the outbox. Delete is soft-delete
//! (sets deactivated_at). Primary designation is per-role per-party.

use chrono::Utc;
use sqlx::PgPool;
use uuid::Uuid;

use crate::events::{
    build_contact_created_envelope, build_contact_deactivated_envelope,
    build_contact_primary_set_envelope, build_contact_updated_envelope, ContactDeactivatedPayload,
    ContactPayload, ContactPrimarySetPayload, EVENT_TYPE_CONTACT_CREATED,
    EVENT_TYPE_CONTACT_DEACTIVATED, EVENT_TYPE_CONTACT_PRIMARY_SET, EVENT_TYPE_CONTACT_UPDATED,
};
use crate::outbox::enqueue_event_tx;

use super::contact::{Contact, CreateContactRequest, PrimaryContactEntry, UpdateContactRequest};
use super::party::PartyError;

// ============================================================================
// Reads
// ============================================================================

/// List active contacts for a party, scoped to app_id.
pub async fn list_contacts(
    pool: &PgPool,
    app_id: &str,
    party_id: Uuid,
) -> Result<Vec<Contact>, PartyError> {
    guard_party_exists(pool, app_id, party_id).await?;

    let contacts: Vec<Contact> = sqlx::query_as(
        r#"
        SELECT id, party_id, app_id, first_name, last_name, email, phone,
               role, is_primary, metadata, created_at, updated_at, deactivated_at
        FROM party_contacts
        WHERE party_id = $1 AND app_id = $2 AND deactivated_at IS NULL
        ORDER BY is_primary DESC, last_name ASC, first_name ASC
        "#,
    )
    .bind(party_id)
    .bind(app_id)
    .fetch_all(pool)
    .await?;

    Ok(contacts)
}

/// Get a single active contact by ID, scoped to app_id.
pub async fn get_contact(
    pool: &PgPool,
    app_id: &str,
    contact_id: Uuid,
) -> Result<Option<Contact>, PartyError> {
    let contact: Option<Contact> = sqlx::query_as(
        r#"
        SELECT id, party_id, app_id, first_name, last_name, email, phone,
               role, is_primary, metadata, created_at, updated_at, deactivated_at
        FROM party_contacts
        WHERE id = $1 AND app_id = $2 AND deactivated_at IS NULL
        "#,
    )
    .bind(contact_id)
    .bind(app_id)
    .fetch_optional(pool)
    .await?;

    Ok(contact)
}

/// Get primary contacts per role for a party.
pub async fn get_primary_contacts(
    pool: &PgPool,
    app_id: &str,
    party_id: Uuid,
) -> Result<Vec<PrimaryContactEntry>, PartyError> {
    guard_party_exists(pool, app_id, party_id).await?;

    let contacts: Vec<Contact> = sqlx::query_as(
        r#"
        SELECT id, party_id, app_id, first_name, last_name, email, phone,
               role, is_primary, metadata, created_at, updated_at, deactivated_at
        FROM party_contacts
        WHERE party_id = $1 AND app_id = $2
          AND is_primary = true AND deactivated_at IS NULL
        ORDER BY role ASC
        "#,
    )
    .bind(party_id)
    .bind(app_id)
    .fetch_all(pool)
    .await?;

    let entries = contacts
        .into_iter()
        .map(|c| PrimaryContactEntry {
            role: c.role.clone().unwrap_or_default(),
            contact: c,
        })
        .collect();

    Ok(entries)
}

// ============================================================================
// Writes
// ============================================================================

/// Create a contact linked to a party. Emits `contact.created` via outbox.
pub async fn create_contact(
    pool: &PgPool,
    app_id: &str,
    party_id: Uuid,
    req: &CreateContactRequest,
    correlation_id: String,
) -> Result<Contact, PartyError> {
    req.validate()?;

    let contact_id = Uuid::new_v4();
    let event_id = Uuid::new_v4();
    let now = Utc::now();
    let is_primary = req.is_primary.unwrap_or(false);

    let mut tx = pool.begin().await?;

    guard_party_exists_tx(&mut tx, app_id, party_id).await?;

    if is_primary {
        clear_primary_for_role(&mut tx, app_id, party_id, req.role.as_deref()).await?;
    }

    let contact: Contact = sqlx::query_as(
        r#"
        INSERT INTO party_contacts (
            id, party_id, app_id, first_name, last_name, email, phone,
            role, is_primary, metadata, created_at, updated_at
        )
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $11)
        RETURNING id, party_id, app_id, first_name, last_name, email, phone,
                  role, is_primary, metadata, created_at, updated_at, deactivated_at
        "#,
    )
    .bind(contact_id)
    .bind(party_id)
    .bind(app_id)
    .bind(req.first_name.trim())
    .bind(req.last_name.trim())
    .bind(&req.email)
    .bind(&req.phone)
    .bind(&req.role)
    .bind(is_primary)
    .bind(&req.metadata)
    .bind(now)
    .fetch_one(&mut *tx)
    .await?;

    // Outbox
    let payload = ContactPayload {
        contact_id,
        party_id,
        app_id: app_id.to_string(),
        first_name: contact.first_name.clone(),
        last_name: contact.last_name.clone(),
        email: contact.email.clone(),
        role: contact.role.clone(),
        is_primary: contact.is_primary,
    };

    let envelope =
        build_contact_created_envelope(event_id, app_id.to_string(), correlation_id, None, payload);

    enqueue_event_tx(
        &mut tx,
        event_id,
        EVENT_TYPE_CONTACT_CREATED,
        "contact",
        &contact_id.to_string(),
        app_id,
        &envelope,
    )
    .await?;

    tx.commit().await?;
    Ok(contact)
}

/// Update a contact. Emits `contact.updated` via outbox.
pub async fn update_contact(
    pool: &PgPool,
    app_id: &str,
    contact_id: Uuid,
    req: &UpdateContactRequest,
    correlation_id: String,
) -> Result<Contact, PartyError> {
    req.validate()?;

    let event_id = Uuid::new_v4();
    let now = Utc::now();
    let mut tx = pool.begin().await?;

    let existing: Option<Contact> = sqlx::query_as(
        r#"
        SELECT id, party_id, app_id, first_name, last_name, email, phone,
               role, is_primary, metadata, created_at, updated_at, deactivated_at
        FROM party_contacts
        WHERE id = $1 AND app_id = $2 AND deactivated_at IS NULL
        FOR UPDATE
        "#,
    )
    .bind(contact_id)
    .bind(app_id)
    .fetch_optional(&mut *tx)
    .await?;

    let current = existing.ok_or(PartyError::NotFound(contact_id))?;

    if req.is_primary == Some(true) && !current.is_primary {
        let role = req.role.as_deref().or(current.role.as_deref());
        clear_primary_for_role(&mut tx, app_id, current.party_id, role).await?;
    }

    let new_first = req
        .first_name
        .as_deref()
        .map(|n| n.trim().to_string())
        .unwrap_or(current.first_name);
    let new_last = req
        .last_name
        .as_deref()
        .map(|n| n.trim().to_string())
        .unwrap_or(current.last_name);
    let new_email = if req.email.is_some() {
        req.email.clone()
    } else {
        current.email
    };
    let new_phone = if req.phone.is_some() {
        req.phone.clone()
    } else {
        current.phone
    };
    let new_role = if req.role.is_some() {
        req.role.clone()
    } else {
        current.role
    };
    let new_primary = req.is_primary.unwrap_or(current.is_primary);
    let new_metadata = if req.metadata.is_some() {
        req.metadata.clone()
    } else {
        current.metadata
    };

    let updated: Contact = sqlx::query_as(
        r#"
        UPDATE party_contacts
        SET first_name = $1, last_name = $2, email = $3, phone = $4,
            role = $5, is_primary = $6, metadata = $7, updated_at = $8
        WHERE id = $9 AND app_id = $10
        RETURNING id, party_id, app_id, first_name, last_name, email, phone,
                  role, is_primary, metadata, created_at, updated_at, deactivated_at
        "#,
    )
    .bind(&new_first)
    .bind(&new_last)
    .bind(&new_email)
    .bind(&new_phone)
    .bind(&new_role)
    .bind(new_primary)
    .bind(&new_metadata)
    .bind(now)
    .bind(contact_id)
    .bind(app_id)
    .fetch_one(&mut *tx)
    .await?;

    // Outbox
    let payload = ContactPayload {
        contact_id,
        party_id: updated.party_id,
        app_id: app_id.to_string(),
        first_name: updated.first_name.clone(),
        last_name: updated.last_name.clone(),
        email: updated.email.clone(),
        role: updated.role.clone(),
        is_primary: updated.is_primary,
    };

    let envelope =
        build_contact_updated_envelope(event_id, app_id.to_string(), correlation_id, None, payload);

    enqueue_event_tx(
        &mut tx,
        event_id,
        EVENT_TYPE_CONTACT_UPDATED,
        "contact",
        &contact_id.to_string(),
        app_id,
        &envelope,
    )
    .await?;

    tx.commit().await?;
    Ok(updated)
}

/// Soft-delete a contact. Emits `contact.deactivated` via outbox.
pub async fn deactivate_contact(
    pool: &PgPool,
    app_id: &str,
    contact_id: Uuid,
    correlation_id: String,
) -> Result<(), PartyError> {
    let event_id = Uuid::new_v4();
    let now = Utc::now();
    let mut tx = pool.begin().await?;

    let existing: Option<Contact> = sqlx::query_as(
        r#"
        SELECT id, party_id, app_id, first_name, last_name, email, phone,
               role, is_primary, metadata, created_at, updated_at, deactivated_at
        FROM party_contacts
        WHERE id = $1 AND app_id = $2 AND deactivated_at IS NULL
        FOR UPDATE
        "#,
    )
    .bind(contact_id)
    .bind(app_id)
    .fetch_optional(&mut *tx)
    .await?;

    let current = existing.ok_or(PartyError::NotFound(contact_id))?;

    sqlx::query(
        "UPDATE party_contacts SET deactivated_at = $1, updated_at = $1 WHERE id = $2 AND app_id = $3",
    )
    .bind(now)
    .bind(contact_id)
    .bind(app_id)
    .execute(&mut *tx)
    .await?;

    // Outbox
    let payload = ContactDeactivatedPayload {
        contact_id,
        party_id: current.party_id,
        app_id: app_id.to_string(),
        deactivated_at: now,
    };

    let envelope = build_contact_deactivated_envelope(
        event_id,
        app_id.to_string(),
        correlation_id,
        None,
        payload,
    );

    enqueue_event_tx(
        &mut tx,
        event_id,
        EVENT_TYPE_CONTACT_DEACTIVATED,
        "contact",
        &contact_id.to_string(),
        app_id,
        &envelope,
    )
    .await?;

    tx.commit().await?;
    Ok(())
}

/// Set a contact as primary for a given role. Clears any existing primary
/// for that role on the same party. Emits `contact.primary_set`.
pub async fn set_primary_for_role(
    pool: &PgPool,
    app_id: &str,
    party_id: Uuid,
    contact_id: Uuid,
    role: &str,
    correlation_id: String,
) -> Result<Contact, PartyError> {
    let event_id = Uuid::new_v4();
    let now = Utc::now();
    let mut tx = pool.begin().await?;

    // Guard: contact must exist, be active, and belong to this party
    let existing: Option<Contact> = sqlx::query_as(
        r#"
        SELECT id, party_id, app_id, first_name, last_name, email, phone,
               role, is_primary, metadata, created_at, updated_at, deactivated_at
        FROM party_contacts
        WHERE id = $1 AND app_id = $2 AND party_id = $3 AND deactivated_at IS NULL
        FOR UPDATE
        "#,
    )
    .bind(contact_id)
    .bind(app_id)
    .bind(party_id)
    .fetch_optional(&mut *tx)
    .await?;

    let _current = existing.ok_or(PartyError::NotFound(contact_id))?;

    // Clear existing primary for this role on this party
    clear_primary_for_role(&mut tx, app_id, party_id, Some(role)).await?;

    // Set this contact as primary with the given role
    let updated: Contact = sqlx::query_as(
        r#"
        UPDATE party_contacts
        SET is_primary = true, role = $1, updated_at = $2
        WHERE id = $3 AND app_id = $4
        RETURNING id, party_id, app_id, first_name, last_name, email, phone,
                  role, is_primary, metadata, created_at, updated_at, deactivated_at
        "#,
    )
    .bind(role)
    .bind(now)
    .bind(contact_id)
    .bind(app_id)
    .fetch_one(&mut *tx)
    .await?;

    // Outbox
    let payload = ContactPrimarySetPayload {
        contact_id,
        party_id,
        app_id: app_id.to_string(),
        role: role.to_string(),
    };

    let envelope = build_contact_primary_set_envelope(
        event_id,
        app_id.to_string(),
        correlation_id,
        None,
        payload,
    );

    enqueue_event_tx(
        &mut tx,
        event_id,
        EVENT_TYPE_CONTACT_PRIMARY_SET,
        "contact",
        &contact_id.to_string(),
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

async fn guard_party_exists(pool: &PgPool, app_id: &str, party_id: Uuid) -> Result<(), PartyError> {
    let exists: Option<(Uuid,)> =
        sqlx::query_as("SELECT id FROM party_parties WHERE id = $1 AND app_id = $2")
            .bind(party_id)
            .bind(app_id)
            .fetch_optional(pool)
            .await?;

    if exists.is_none() {
        return Err(PartyError::NotFound(party_id));
    }
    Ok(())
}

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

async fn clear_primary_for_role(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    app_id: &str,
    party_id: Uuid,
    role: Option<&str>,
) -> Result<(), PartyError> {
    if let Some(role) = role {
        sqlx::query(
            r#"
            UPDATE party_contacts
            SET is_primary = false
            WHERE party_id = $1 AND app_id = $2 AND role = $3
              AND is_primary = true AND deactivated_at IS NULL
            "#,
        )
        .bind(party_id)
        .bind(app_id)
        .bind(role)
        .execute(&mut **tx)
        .await?;
    } else {
        sqlx::query(
            r#"
            UPDATE party_contacts
            SET is_primary = false
            WHERE party_id = $1 AND app_id = $2
              AND is_primary = true AND deactivated_at IS NULL
            "#,
        )
        .bind(party_id)
        .bind(app_id)
        .execute(&mut **tx)
        .await?;
    }
    Ok(())
}
