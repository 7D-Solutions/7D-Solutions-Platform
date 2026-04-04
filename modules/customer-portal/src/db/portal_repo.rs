//! Repository for customer-portal SQL operations.

use chrono::{DateTime, Utc};
use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

// ── Row types ────────────────────────────────────────────────────────

#[derive(sqlx::FromRow)]
pub struct PortalUserRow {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub party_id: Uuid,
    pub password_hash: String,
    pub is_active: bool,
    pub lock_until: Option<DateTime<Utc>>,
}

#[derive(sqlx::FromRow)]
pub struct PortalUserEmailRow {
    pub email: String,
}

#[derive(sqlx::FromRow)]
pub struct PortalDocLinkRow {
    pub document_id: Uuid,
    pub display_title: Option<String>,
}

// ── Idempotency ──────────────────────────────────────────────────────

pub async fn find_idempotency(
    pool: &PgPool,
    tenant_id: Uuid,
    operation: &str,
    idempotency_key: &str,
) -> Result<Option<serde_json::Value>, sqlx::Error> {
    sqlx::query_scalar::<_, serde_json::Value>(
        "SELECT response FROM portal_idempotency WHERE tenant_id=$1 AND operation=$2 AND idempotency_key=$3",
    )
    .bind(tenant_id)
    .bind(operation)
    .bind(idempotency_key)
    .fetch_optional(pool)
    .await
}

pub async fn insert_idempotency_tx(
    tx: &mut Transaction<'_, Postgres>,
    tenant_id: Uuid,
    operation: &str,
    idempotency_key: &str,
    response: &serde_json::Value,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO portal_idempotency (tenant_id, operation, idempotency_key, response) VALUES ($1,$2,$3,$4) \
         ON CONFLICT (tenant_id, operation, idempotency_key) DO NOTHING",
    )
    .bind(tenant_id)
    .bind(operation)
    .bind(idempotency_key)
    .bind(response)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

// ── Auth ─────────────────────────────────────────────────────────────

pub async fn find_user_by_email(
    pool: &PgPool,
    tenant_id: Uuid,
    email: &str,
) -> Result<Option<PortalUserRow>, sqlx::Error> {
    sqlx::query_as::<_, PortalUserRow>(
        "SELECT id, tenant_id, party_id, password_hash, is_active, lock_until \
         FROM portal_users WHERE tenant_id=$1 AND email=$2",
    )
    .bind(tenant_id)
    .bind(email)
    .fetch_optional(pool)
    .await
}

pub async fn insert_refresh_token_tx(
    tx: &mut Transaction<'_, Postgres>,
    id: Uuid,
    tenant_id: Uuid,
    user_id: Uuid,
    token_hash: &str,
    expires_at: DateTime<Utc>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO portal_refresh_tokens (id, tenant_id, user_id, token_hash, expires_at) \
         VALUES ($1,$2,$3,$4,$5)",
    )
    .bind(id)
    .bind(tenant_id)
    .bind(user_id)
    .bind(token_hash)
    .bind(expires_at)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

pub async fn update_last_login_tx(
    tx: &mut Transaction<'_, Postgres>,
    user_id: Uuid,
) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE portal_users SET last_login_at = NOW() WHERE id = $1")
        .bind(user_id)
        .execute(&mut **tx)
        .await?;
    Ok(())
}

pub async fn find_refresh_token(
    pool: &PgPool,
    token_hash: &str,
) -> Result<Option<(Uuid, Uuid, Uuid, DateTime<Utc>, Option<DateTime<Utc>>)>, sqlx::Error> {
    sqlx::query_as::<_, (Uuid, Uuid, Uuid, DateTime<Utc>, Option<DateTime<Utc>>)>(
        "SELECT rt.user_id, rt.tenant_id, u.party_id, rt.expires_at, rt.revoked_at \
         FROM portal_refresh_tokens rt \
         JOIN portal_users u ON u.id = rt.user_id \
         WHERE rt.token_hash = $1",
    )
    .bind(token_hash)
    .fetch_optional(pool)
    .await
}

pub async fn revoke_refresh_token_tx(
    tx: &mut Transaction<'_, Postgres>,
    token_hash: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE portal_refresh_tokens SET revoked_at = NOW() WHERE token_hash = $1")
        .bind(token_hash)
        .execute(&mut **tx)
        .await?;
    Ok(())
}

pub async fn find_active_refresh_token(
    pool: &PgPool,
    token_hash: &str,
) -> Result<Option<(Uuid, Uuid)>, sqlx::Error> {
    sqlx::query_as::<_, (Uuid, Uuid)>(
        "SELECT user_id, tenant_id FROM portal_refresh_tokens WHERE token_hash=$1 AND revoked_at IS NULL",
    )
    .bind(token_hash)
    .fetch_optional(pool)
    .await
}

// ── Admin ────────────────────────────────────────────────────────────

pub async fn insert_user_tx(
    tx: &mut Transaction<'_, Postgres>,
    user_id: Uuid,
    tenant_id: Uuid,
    party_id: Uuid,
    email: &str,
    password_hash: &str,
    display_name: &str,
    invited_by: Uuid,
    invited_at: DateTime<Utc>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO portal_users (id, tenant_id, party_id, email, password_hash, display_name, invited_by, invited_at) \
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8)",
    )
    .bind(user_id)
    .bind(tenant_id)
    .bind(party_id)
    .bind(email)
    .bind(password_hash)
    .bind(display_name)
    .bind(invited_by)
    .bind(invited_at)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

// ── Status feed ──────────────────────────────────────────────────────

pub async fn insert_status_card(
    pool: &PgPool,
    id: Uuid,
    tenant_id: Uuid,
    party_id: Uuid,
    entity_type: &str,
    entity_id: Option<Uuid>,
    title: &str,
    status: &str,
    details: &serde_json::Value,
    source: &str,
    occurred_at: DateTime<Utc>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO portal_status_feed (id, tenant_id, party_id, entity_type, entity_id, title, status, details, source, occurred_at) \
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10)",
    )
    .bind(id)
    .bind(tenant_id)
    .bind(party_id)
    .bind(entity_type)
    .bind(entity_id)
    .bind(title)
    .bind(status)
    .bind(details)
    .bind(source)
    .bind(occurred_at)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn count_status_cards(
    pool: &PgPool,
    tenant_id: Uuid,
    party_id: Uuid,
) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar(
        "SELECT COUNT(*) FROM portal_status_feed WHERE tenant_id = $1 AND party_id = $2",
    )
    .bind(tenant_id)
    .bind(party_id)
    .fetch_one(pool)
    .await
}

pub async fn list_status_cards(
    pool: &PgPool,
    tenant_id: Uuid,
    party_id: Uuid,
    limit: i64,
    offset: i64,
) -> Result<Vec<crate::http::status::StatusCard>, sqlx::Error> {
    sqlx::query_as::<_, crate::http::status::StatusCard>(
        "SELECT id, entity_type, entity_id, title, status, details, source, occurred_at \
         FROM portal_status_feed WHERE tenant_id = $1 AND party_id = $2 ORDER BY occurred_at DESC LIMIT $3 OFFSET $4",
    )
    .bind(tenant_id)
    .bind(party_id)
    .bind(limit)
    .bind(offset)
    .fetch_all(pool)
    .await
}

pub async fn find_document_link(
    pool: &PgPool,
    tenant_id: Uuid,
    party_id: Uuid,
    document_id: Uuid,
) -> Result<Option<(Uuid,)>, sqlx::Error> {
    sqlx::query_as(
        "SELECT document_id FROM portal_document_links WHERE tenant_id = $1 AND party_id = $2 AND document_id = $3",
    )
    .bind(tenant_id)
    .bind(party_id)
    .bind(document_id)
    .fetch_optional(pool)
    .await
}

pub async fn insert_acknowledgment_tx(
    tx: &mut Transaction<'_, Postgres>,
    id: Uuid,
    tenant_id: Uuid,
    party_id: Uuid,
    portal_user_id: Uuid,
    document_id: Option<Uuid>,
    status_card_id: Option<Uuid>,
    ack_type: &str,
    notes: Option<&str>,
    idempotency_key: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO portal_acknowledgments \
         (id, tenant_id, party_id, portal_user_id, document_id, status_card_id, ack_type, notes, idempotency_key) \
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9)",
    )
    .bind(id)
    .bind(tenant_id)
    .bind(party_id)
    .bind(portal_user_id)
    .bind(document_id)
    .bind(status_card_id)
    .bind(ack_type)
    .bind(notes)
    .bind(idempotency_key)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

pub async fn insert_idempotency_no_conflict_tx(
    tx: &mut Transaction<'_, Postgres>,
    tenant_id: Uuid,
    operation: &str,
    idempotency_key: &str,
    response: &serde_json::Value,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO portal_idempotency (tenant_id, operation, idempotency_key, response) VALUES ($1,$2,$3,$4)",
    )
    .bind(tenant_id)
    .bind(operation)
    .bind(idempotency_key)
    .bind(response)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

pub async fn upsert_document_link(
    pool: &PgPool,
    id: Uuid,
    tenant_id: Uuid,
    party_id: Uuid,
    document_id: Uuid,
    display_title: Option<&str>,
    created_by: Uuid,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO portal_document_links (id, tenant_id, party_id, document_id, display_title, created_by) \
         VALUES ($1,$2,$3,$4,$5,$6) \
         ON CONFLICT (tenant_id, party_id, document_id) DO NOTHING",
    )
    .bind(id)
    .bind(tenant_id)
    .bind(party_id)
    .bind(document_id)
    .bind(display_title)
    .bind(created_by)
    .execute(pool)
    .await?;
    Ok(())
}

// ── Docs ─────────────────────────────────────────────────────────────

pub async fn get_user_email(
    pool: &PgPool,
    user_id: Uuid,
    tenant_id: Uuid,
    party_id: Uuid,
) -> Result<Option<PortalUserEmailRow>, sqlx::Error> {
    sqlx::query_as::<_, PortalUserEmailRow>(
        "SELECT email FROM portal_users WHERE id = $1 AND tenant_id = $2 AND party_id = $3",
    )
    .bind(user_id)
    .bind(tenant_id)
    .bind(party_id)
    .fetch_optional(pool)
    .await
}

pub async fn list_document_links(
    pool: &PgPool,
    tenant_id: Uuid,
    party_id: Uuid,
) -> Result<Vec<PortalDocLinkRow>, sqlx::Error> {
    sqlx::query_as::<_, PortalDocLinkRow>(
        "SELECT document_id, display_title FROM portal_document_links WHERE tenant_id = $1 AND party_id = $2 ORDER BY created_at DESC",
    )
    .bind(tenant_id)
    .bind(party_id)
    .fetch_all(pool)
    .await
}
