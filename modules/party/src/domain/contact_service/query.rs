use sqlx::PgPool;
use uuid::Uuid;

use crate::domain::contact::{Contact, PrimaryContactEntry};
use crate::domain::party::PartyError;

use super::guards::guard_party_exists;

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
