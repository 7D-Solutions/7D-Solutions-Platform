//! Contact role repository — all SQL for `party_contact_roles`.

use chrono::{DateTime, Utc};
use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

use crate::domain::contact_role::{ContactRole, CreateContactRoleRequest};
use crate::domain::party::PartyError;

// ── Reads ─────────────────────────────────────────────────────────────────────

pub async fn list_contact_roles(
    pool: &PgPool,
    app_id: &str,
    party_id: Uuid,
) -> Result<Vec<ContactRole>, PartyError> {
    Ok(sqlx::query_as(
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
    .await?)
}

pub async fn get_contact_role(
    pool: &PgPool,
    app_id: &str,
    role_id: Uuid,
) -> Result<Option<ContactRole>, PartyError> {
    Ok(sqlx::query_as(
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
    .await?)
}

pub async fn find_contact_role_by_idempotency_key(
    pool: &PgPool,
    app_id: &str,
    idem_key: &str,
) -> Result<Option<ContactRole>, PartyError> {
    Ok(sqlx::query_as(
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
    .await?)
}

// ── Transaction helpers ───────────────────────────────────────────────────────

pub async fn clear_primary_for_role_type_tx(
    tx: &mut Transaction<'_, Postgres>,
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

pub async fn insert_contact_role_tx(
    tx: &mut Transaction<'_, Postgres>,
    role_id: Uuid,
    party_id: Uuid,
    app_id: &str,
    req: &CreateContactRoleRequest,
    is_primary: bool,
    now: DateTime<Utc>,
) -> Result<ContactRole, PartyError> {
    Ok(sqlx::query_as(
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
    .fetch_one(&mut **tx)
    .await?)
}

pub async fn fetch_contact_role_for_update_tx(
    tx: &mut Transaction<'_, Postgres>,
    app_id: &str,
    role_id: Uuid,
) -> Result<Option<ContactRole>, PartyError> {
    Ok(sqlx::query_as(
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
    .fetch_optional(&mut **tx)
    .await?)
}

pub struct UpdateContactRoleData<'a> {
    pub role_id: Uuid,
    pub app_id: &'a str,
    pub role_type: String,
    pub is_primary: bool,
    pub effective_from: chrono::NaiveDate,
    pub effective_to: Option<chrono::NaiveDate>,
    pub metadata: Option<serde_json::Value>,
    pub updated_at: DateTime<Utc>,
}

pub async fn update_contact_role_row_tx(
    tx: &mut Transaction<'_, Postgres>,
    p: &UpdateContactRoleData<'_>,
) -> Result<ContactRole, PartyError> {
    Ok(sqlx::query_as(
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
    .bind(&p.role_type)
    .bind(p.is_primary)
    .bind(p.effective_from)
    .bind(p.effective_to)
    .bind(&p.metadata)
    .bind(p.updated_at)
    .bind(p.role_id)
    .bind(p.app_id)
    .fetch_one(&mut **tx)
    .await?)
}
