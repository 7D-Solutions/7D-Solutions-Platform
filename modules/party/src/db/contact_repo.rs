//! Contact repository — all SQL for `party_contacts`.

use chrono::{DateTime, Utc};
use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

use crate::domain::contact::{Contact, CreateContactRequest};
use crate::domain::party::PartyError;

// ── Guard helpers ─────────────────────────────────────────────────────────────

pub async fn clear_primary_for_role_tx(
    tx: &mut Transaction<'_, Postgres>,
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

// ── Reads ─────────────────────────────────────────────────────────────────────

pub async fn list_contacts(
    pool: &PgPool,
    app_id: &str,
    party_id: Uuid,
) -> Result<Vec<Contact>, PartyError> {
    Ok(sqlx::query_as(
        r#"
        SELECT id, party_id, app_id, first_name, last_name, email, phone,
               role, is_primary, metadata, notification_events, notification_channels,
               created_at, updated_at, deactivated_at
        FROM party_contacts
        WHERE party_id = $1 AND app_id = $2 AND deactivated_at IS NULL
        ORDER BY is_primary DESC, last_name ASC, first_name ASC
        "#,
    )
    .bind(party_id)
    .bind(app_id)
    .fetch_all(pool)
    .await?)
}

pub async fn get_contact(
    pool: &PgPool,
    app_id: &str,
    contact_id: Uuid,
) -> Result<Option<Contact>, PartyError> {
    Ok(sqlx::query_as(
        r#"
        SELECT id, party_id, app_id, first_name, last_name, email, phone,
               role, is_primary, metadata, notification_events, notification_channels,
               created_at, updated_at, deactivated_at
        FROM party_contacts
        WHERE id = $1 AND app_id = $2 AND deactivated_at IS NULL
        "#,
    )
    .bind(contact_id)
    .bind(app_id)
    .fetch_optional(pool)
    .await?)
}

pub async fn list_primary_contacts(
    pool: &PgPool,
    app_id: &str,
    party_id: Uuid,
) -> Result<Vec<Contact>, PartyError> {
    Ok(sqlx::query_as(
        r#"
        SELECT id, party_id, app_id, first_name, last_name, email, phone,
               role, is_primary, metadata, notification_events, notification_channels,
               created_at, updated_at, deactivated_at
        FROM party_contacts
        WHERE party_id = $1 AND app_id = $2
          AND is_primary = true AND deactivated_at IS NULL
        ORDER BY role ASC
        "#,
    )
    .bind(party_id)
    .bind(app_id)
    .fetch_all(pool)
    .await?)
}

// ── Transaction helpers ───────────────────────────────────────────────────────

pub async fn insert_contact_tx(
    tx: &mut Transaction<'_, Postgres>,
    contact_id: Uuid,
    party_id: Uuid,
    app_id: &str,
    req: &CreateContactRequest,
    is_primary: bool,
    now: DateTime<Utc>,
) -> Result<Contact, PartyError> {
    Ok(sqlx::query_as(
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
    .bind(req.last_name.as_deref().map(|n| n.trim()))
    .bind(&req.email)
    .bind(&req.phone)
    .bind(&req.role)
    .bind(is_primary)
    .bind(&req.metadata)
    .bind(now)
    .fetch_one(&mut **tx)
    .await?)
}

pub async fn fetch_contact_for_update_tx(
    tx: &mut Transaction<'_, Postgres>,
    app_id: &str,
    contact_id: Uuid,
) -> Result<Option<Contact>, PartyError> {
    Ok(sqlx::query_as(
        r#"
        SELECT id, party_id, app_id, first_name, last_name, email, phone,
               role, is_primary, metadata, notification_events, notification_channels,
               created_at, updated_at, deactivated_at
        FROM party_contacts
        WHERE id = $1 AND app_id = $2 AND deactivated_at IS NULL
        FOR UPDATE
        "#,
    )
    .bind(contact_id)
    .bind(app_id)
    .fetch_optional(&mut **tx)
    .await?)
}

pub struct UpdateContactData<'a> {
    pub contact_id: Uuid,
    pub app_id: &'a str,
    pub first_name: String,
    pub last_name: Option<String>,
    pub email: Option<String>,
    pub phone: Option<String>,
    pub role: Option<String>,
    pub is_primary: bool,
    pub metadata: Option<serde_json::Value>,
    pub updated_at: DateTime<Utc>,
}

pub async fn update_contact_row_tx(
    tx: &mut Transaction<'_, Postgres>,
    p: &UpdateContactData<'_>,
) -> Result<Contact, PartyError> {
    Ok(sqlx::query_as(
        r#"
        UPDATE party_contacts
        SET first_name = $1, last_name = $2, email = $3, phone = $4,
            role = $5, is_primary = $6, metadata = $7, updated_at = $8
        WHERE id = $9 AND app_id = $10
        RETURNING id, party_id, app_id, first_name, last_name, email, phone,
                  role, is_primary, metadata, created_at, updated_at, deactivated_at
        "#,
    )
    .bind(&p.first_name)
    .bind(&p.last_name)
    .bind(&p.email)
    .bind(&p.phone)
    .bind(&p.role)
    .bind(p.is_primary)
    .bind(&p.metadata)
    .bind(p.updated_at)
    .bind(p.contact_id)
    .bind(p.app_id)
    .fetch_one(&mut **tx)
    .await?)
}

pub async fn deactivate_contact_tx(
    tx: &mut Transaction<'_, Postgres>,
    app_id: &str,
    contact_id: Uuid,
    now: DateTime<Utc>,
) -> Result<(), PartyError> {
    sqlx::query(
        "UPDATE party_contacts SET deactivated_at = $1, updated_at = $1 WHERE id = $2 AND app_id = $3",
    )
    .bind(now)
    .bind(contact_id)
    .bind(app_id)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

pub async fn fetch_contact_for_primary_set_tx(
    tx: &mut Transaction<'_, Postgres>,
    app_id: &str,
    party_id: Uuid,
    contact_id: Uuid,
) -> Result<Option<Contact>, PartyError> {
    Ok(sqlx::query_as(
        r#"
        SELECT id, party_id, app_id, first_name, last_name, email, phone,
               role, is_primary, metadata, notification_events, notification_channels,
               created_at, updated_at, deactivated_at
        FROM party_contacts
        WHERE id = $1 AND app_id = $2 AND party_id = $3 AND deactivated_at IS NULL
        FOR UPDATE
        "#,
    )
    .bind(contact_id)
    .bind(app_id)
    .bind(party_id)
    .fetch_optional(&mut **tx)
    .await?)
}

pub async fn set_contact_primary_tx(
    tx: &mut Transaction<'_, Postgres>,
    app_id: &str,
    contact_id: Uuid,
    role: &str,
    now: DateTime<Utc>,
) -> Result<Contact, PartyError> {
    Ok(sqlx::query_as(
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
    .fetch_one(&mut **tx)
    .await?)
}

/// Guard: contact must exist and belong to the given party.
pub async fn guard_contact_belongs_to_party_tx(
    tx: &mut Transaction<'_, Postgres>,
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
