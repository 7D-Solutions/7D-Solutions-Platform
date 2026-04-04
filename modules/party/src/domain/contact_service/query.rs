use sqlx::PgPool;
use uuid::Uuid;

use crate::db::contact_repo;
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
    contact_repo::list_contacts(pool, app_id, party_id).await
}

/// Get a single active contact by ID, scoped to app_id.
pub async fn get_contact(
    pool: &PgPool,
    app_id: &str,
    contact_id: Uuid,
) -> Result<Option<Contact>, PartyError> {
    contact_repo::get_contact(pool, app_id, contact_id).await
}

/// Get primary contacts per role for a party.
pub async fn get_primary_contacts(
    pool: &PgPool,
    app_id: &str,
    party_id: Uuid,
) -> Result<Vec<PrimaryContactEntry>, PartyError> {
    guard_party_exists(pool, app_id, party_id).await?;

    let contacts = contact_repo::list_primary_contacts(pool, app_id, party_id).await?;

    let entries = contacts
        .into_iter()
        .map(|c| PrimaryContactEntry {
            role: c.role.clone().unwrap_or_default(),
            contact: c,
        })
        .collect();

    Ok(entries)
}
