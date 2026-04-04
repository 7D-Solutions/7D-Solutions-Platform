//! Address repository — all SQL for `party_addresses`.

use chrono::{DateTime, Utc};
use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

use crate::domain::address::{Address, CreateAddressRequest, UpdateAddressRequest};
use crate::domain::party::PartyError;

// ── Reads ─────────────────────────────────────────────────────────────────────

pub async fn list_addresses(
    pool: &PgPool,
    app_id: &str,
    party_id: Uuid,
) -> Result<Vec<Address>, PartyError> {
    Ok(sqlx::query_as(
        r#"
        SELECT id, party_id, app_id, address_type::TEXT AS address_type,
               label, line1, line2, city, state, postal_code, country,
               is_primary, metadata, created_at, updated_at
        FROM party_addresses
        WHERE party_id = $1 AND app_id = $2
        ORDER BY is_primary DESC, address_type ASC
        "#,
    )
    .bind(party_id)
    .bind(app_id)
    .fetch_all(pool)
    .await?)
}

pub async fn get_address(
    pool: &PgPool,
    app_id: &str,
    address_id: Uuid,
) -> Result<Option<Address>, PartyError> {
    Ok(sqlx::query_as(
        r#"
        SELECT id, party_id, app_id, address_type::TEXT AS address_type,
               label, line1, line2, city, state, postal_code, country,
               is_primary, metadata, created_at, updated_at
        FROM party_addresses
        WHERE id = $1 AND app_id = $2
        "#,
    )
    .bind(address_id)
    .bind(app_id)
    .fetch_optional(pool)
    .await?)
}

// ── Transaction helpers ───────────────────────────────────────────────────────

pub async fn clear_primary_addresses_tx(
    tx: &mut Transaction<'_, Postgres>,
    app_id: &str,
    party_id: Uuid,
) -> Result<(), PartyError> {
    sqlx::query(
        "UPDATE party_addresses SET is_primary = false WHERE party_id = $1 AND app_id = $2 AND is_primary = true",
    )
    .bind(party_id)
    .bind(app_id)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

pub async fn insert_address_tx(
    tx: &mut Transaction<'_, Postgres>,
    address_id: Uuid,
    party_id: Uuid,
    app_id: &str,
    req: &CreateAddressRequest,
    addr_type: &str,
    country: &str,
    is_primary: bool,
    now: DateTime<Utc>,
) -> Result<Address, PartyError> {
    Ok(sqlx::query_as(
        r#"
        INSERT INTO party_addresses (
            id, party_id, app_id, address_type, label, line1, line2,
            city, state, postal_code, country, is_primary, metadata,
            created_at, updated_at
        )
        VALUES ($1, $2, $3, $4::party_address_type, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $14)
        RETURNING id, party_id, app_id, address_type::TEXT AS address_type,
                  label, line1, line2, city, state, postal_code, country,
                  is_primary, metadata, created_at, updated_at
        "#,
    )
    .bind(address_id)
    .bind(party_id)
    .bind(app_id)
    .bind(addr_type)
    .bind(&req.label)
    .bind(req.line1.trim())
    .bind(&req.line2)
    .bind(req.city.trim())
    .bind(&req.state)
    .bind(&req.postal_code)
    .bind(country)
    .bind(is_primary)
    .bind(&req.metadata)
    .bind(now)
    .fetch_one(&mut **tx)
    .await?)
}

pub async fn fetch_address_for_update_tx(
    tx: &mut Transaction<'_, Postgres>,
    app_id: &str,
    address_id: Uuid,
) -> Result<Option<Address>, PartyError> {
    Ok(sqlx::query_as(
        r#"
        SELECT id, party_id, app_id, address_type::TEXT AS address_type,
               label, line1, line2, city, state, postal_code, country,
               is_primary, metadata, created_at, updated_at
        FROM party_addresses
        WHERE id = $1 AND app_id = $2
        FOR UPDATE
        "#,
    )
    .bind(address_id)
    .bind(app_id)
    .fetch_optional(&mut **tx)
    .await?)
}

pub struct UpdateAddressData<'a> {
    pub address_id: Uuid,
    pub app_id: &'a str,
    pub addr_type: &'a str,
    pub label: Option<String>,
    pub line1: String,
    pub line2: Option<String>,
    pub city: String,
    pub state: Option<String>,
    pub postal_code: Option<String>,
    pub country: String,
    pub is_primary: bool,
    pub metadata: Option<serde_json::Value>,
    pub updated_at: DateTime<Utc>,
}

pub async fn update_address_row_tx(
    tx: &mut Transaction<'_, Postgres>,
    p: &UpdateAddressData<'_>,
) -> Result<Address, PartyError> {
    Ok(sqlx::query_as(
        r#"
        UPDATE party_addresses
        SET address_type = $1::party_address_type, label = $2, line1 = $3, line2 = $4,
            city = $5, state = $6, postal_code = $7, country = $8,
            is_primary = $9, metadata = $10, updated_at = $11
        WHERE id = $12 AND app_id = $13
        RETURNING id, party_id, app_id, address_type::TEXT AS address_type,
                  label, line1, line2, city, state, postal_code, country,
                  is_primary, metadata, created_at, updated_at
        "#,
    )
    .bind(p.addr_type)
    .bind(&p.label)
    .bind(&p.line1)
    .bind(&p.line2)
    .bind(&p.city)
    .bind(&p.state)
    .bind(&p.postal_code)
    .bind(&p.country)
    .bind(p.is_primary)
    .bind(&p.metadata)
    .bind(p.updated_at)
    .bind(p.address_id)
    .bind(p.app_id)
    .fetch_one(&mut **tx)
    .await?)
}

pub async fn delete_address(
    pool: &PgPool,
    app_id: &str,
    address_id: Uuid,
) -> Result<u64, PartyError> {
    let result = sqlx::query("DELETE FROM party_addresses WHERE id = $1 AND app_id = $2")
        .bind(address_id)
        .bind(app_id)
        .execute(pool)
        .await?;
    Ok(result.rows_affected())
}
