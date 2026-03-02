//! Contact CRUD service — Guard→Mutation atomicity.

use chrono::Utc;
use sqlx::PgPool;
use uuid::Uuid;

use super::contact::{Contact, CreateContactRequest, UpdateContactRequest};
use super::party::PartyError;

// ============================================================================
// Reads
// ============================================================================

/// List contacts for a party, scoped to app_id.
pub async fn list_contacts(
    pool: &PgPool,
    app_id: &str,
    party_id: Uuid,
) -> Result<Vec<Contact>, PartyError> {
    // Guard: party must exist
    guard_party_exists(pool, app_id, party_id).await?;

    let contacts: Vec<Contact> = sqlx::query_as(
        r#"
        SELECT id, party_id, app_id, first_name, last_name, email, phone,
               role, is_primary, metadata, created_at, updated_at
        FROM party_contacts
        WHERE party_id = $1 AND app_id = $2
        ORDER BY is_primary DESC, last_name ASC, first_name ASC
        "#,
    )
    .bind(party_id)
    .bind(app_id)
    .fetch_all(pool)
    .await?;

    Ok(contacts)
}

/// Get a single contact by ID, scoped to app_id.
pub async fn get_contact(
    pool: &PgPool,
    app_id: &str,
    contact_id: Uuid,
) -> Result<Option<Contact>, PartyError> {
    let contact: Option<Contact> = sqlx::query_as(
        r#"
        SELECT id, party_id, app_id, first_name, last_name, email, phone,
               role, is_primary, metadata, created_at, updated_at
        FROM party_contacts
        WHERE id = $1 AND app_id = $2
        "#,
    )
    .bind(contact_id)
    .bind(app_id)
    .fetch_optional(pool)
    .await?;

    Ok(contact)
}

// ============================================================================
// Writes
// ============================================================================

/// Create a contact linked to a party. Transactional.
pub async fn create_contact(
    pool: &PgPool,
    app_id: &str,
    party_id: Uuid,
    req: &CreateContactRequest,
) -> Result<Contact, PartyError> {
    req.validate()?;

    let contact_id = Uuid::new_v4();
    let now = Utc::now();
    let is_primary = req.is_primary.unwrap_or(false);

    let mut tx = pool.begin().await?;

    // Guard: party must exist for this app
    guard_party_exists_tx(&mut tx, app_id, party_id).await?;

    // If marking as primary, clear existing primary contacts
    if is_primary {
        clear_primary_contacts(&mut tx, app_id, party_id).await?;
    }

    let contact: Contact = sqlx::query_as(
        r#"
        INSERT INTO party_contacts (
            id, party_id, app_id, first_name, last_name, email, phone,
            role, is_primary, metadata, created_at, updated_at
        )
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $11)
        RETURNING id, party_id, app_id, first_name, last_name, email, phone,
                  role, is_primary, metadata, created_at, updated_at
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

    tx.commit().await?;
    Ok(contact)
}

/// Update a contact. Transactional with FOR UPDATE lock.
pub async fn update_contact(
    pool: &PgPool,
    app_id: &str,
    contact_id: Uuid,
    req: &UpdateContactRequest,
) -> Result<Contact, PartyError> {
    req.validate()?;

    let now = Utc::now();
    let mut tx = pool.begin().await?;

    // Guard: contact must exist
    let existing: Option<Contact> = sqlx::query_as(
        r#"
        SELECT id, party_id, app_id, first_name, last_name, email, phone,
               role, is_primary, metadata, created_at, updated_at
        FROM party_contacts
        WHERE id = $1 AND app_id = $2
        FOR UPDATE
        "#,
    )
    .bind(contact_id)
    .bind(app_id)
    .fetch_optional(&mut *tx)
    .await?;

    let current = existing.ok_or(PartyError::NotFound(contact_id))?;

    // If marking as primary, clear existing primary contacts
    if req.is_primary == Some(true) && !current.is_primary {
        clear_primary_contacts(&mut tx, app_id, current.party_id).await?;
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
                  role, is_primary, metadata, created_at, updated_at
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

    tx.commit().await?;
    Ok(updated)
}

/// Delete a contact by ID.
pub async fn delete_contact(
    pool: &PgPool,
    app_id: &str,
    contact_id: Uuid,
) -> Result<(), PartyError> {
    let result = sqlx::query("DELETE FROM party_contacts WHERE id = $1 AND app_id = $2")
        .bind(contact_id)
        .bind(app_id)
        .execute(pool)
        .await?;

    if result.rows_affected() == 0 {
        return Err(PartyError::NotFound(contact_id));
    }
    Ok(())
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

async fn clear_primary_contacts(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    app_id: &str,
    party_id: Uuid,
) -> Result<(), PartyError> {
    sqlx::query(
        "UPDATE party_contacts SET is_primary = false WHERE party_id = $1 AND app_id = $2 AND is_primary = true",
    )
    .bind(party_id)
    .bind(app_id)
    .execute(&mut **tx)
    .await?;
    Ok(())
}
