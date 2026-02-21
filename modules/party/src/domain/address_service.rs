//! Address CRUD service — Guard→Mutation atomicity.

use chrono::Utc;
use sqlx::PgPool;
use uuid::Uuid;

use super::address::{Address, CreateAddressRequest, UpdateAddressRequest};
use super::party::PartyError;

// ============================================================================
// Reads
// ============================================================================

/// List addresses for a party, scoped to app_id.
pub async fn list_addresses(
    pool: &PgPool,
    app_id: &str,
    party_id: Uuid,
) -> Result<Vec<Address>, PartyError> {
    guard_party_exists(pool, app_id, party_id).await?;

    let addresses: Vec<Address> = sqlx::query_as(
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
    .await?;

    Ok(addresses)
}

/// Get a single address by ID, scoped to app_id.
pub async fn get_address(
    pool: &PgPool,
    app_id: &str,
    address_id: Uuid,
) -> Result<Option<Address>, PartyError> {
    let address: Option<Address> = sqlx::query_as(
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
    .await?;

    Ok(address)
}

// ============================================================================
// Writes
// ============================================================================

/// Create an address linked to a party. Transactional.
pub async fn create_address(
    pool: &PgPool,
    app_id: &str,
    party_id: Uuid,
    req: &CreateAddressRequest,
) -> Result<Address, PartyError> {
    req.validate()?;

    let address_id = Uuid::new_v4();
    let now = Utc::now();
    let addr_type = req.address_type.as_deref().unwrap_or("other");
    let country = req.country.as_deref().unwrap_or("US");
    let is_primary = req.is_primary.unwrap_or(false);

    let mut tx = pool.begin().await?;

    guard_party_exists_tx(&mut tx, app_id, party_id).await?;

    if is_primary {
        clear_primary_addresses(&mut tx, app_id, party_id).await?;
    }

    let address: Address = sqlx::query_as(
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
    .fetch_one(&mut *tx)
    .await?;

    tx.commit().await?;
    Ok(address)
}

/// Update an address. Transactional with FOR UPDATE lock.
pub async fn update_address(
    pool: &PgPool,
    app_id: &str,
    address_id: Uuid,
    req: &UpdateAddressRequest,
) -> Result<Address, PartyError> {
    req.validate()?;

    let now = Utc::now();
    let mut tx = pool.begin().await?;

    let existing: Option<Address> = sqlx::query_as(
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
    .fetch_optional(&mut *tx)
    .await?;

    let current = existing.ok_or(PartyError::NotFound(address_id))?;

    if req.is_primary == Some(true) && !current.is_primary {
        clear_primary_addresses(&mut tx, app_id, current.party_id).await?;
    }

    let new_type = req.address_type.as_deref().unwrap_or(&current.address_type);
    let new_label = if req.label.is_some() { req.label.clone() } else { current.label };
    let new_line1 = req.line1.as_deref().map(|l| l.trim().to_string())
        .unwrap_or(current.line1);
    let new_line2 = if req.line2.is_some() { req.line2.clone() } else { current.line2 };
    let new_city = req.city.as_deref().map(|c| c.trim().to_string())
        .unwrap_or(current.city);
    let new_state = if req.state.is_some() { req.state.clone() } else { current.state };
    let new_postal = if req.postal_code.is_some() { req.postal_code.clone() } else { current.postal_code };
    let new_country = req.country.as_deref().unwrap_or(&current.country).to_string();
    let new_primary = req.is_primary.unwrap_or(current.is_primary);
    let new_metadata = if req.metadata.is_some() { req.metadata.clone() } else { current.metadata };

    let updated: Address = sqlx::query_as(
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
    .bind(new_type)
    .bind(&new_label)
    .bind(&new_line1)
    .bind(&new_line2)
    .bind(&new_city)
    .bind(&new_state)
    .bind(&new_postal)
    .bind(&new_country)
    .bind(new_primary)
    .bind(&new_metadata)
    .bind(now)
    .bind(address_id)
    .bind(app_id)
    .fetch_one(&mut *tx)
    .await?;

    tx.commit().await?;
    Ok(updated)
}

/// Delete an address by ID.
pub async fn delete_address(
    pool: &PgPool,
    app_id: &str,
    address_id: Uuid,
) -> Result<(), PartyError> {
    let result = sqlx::query(
        "DELETE FROM party_addresses WHERE id = $1 AND app_id = $2",
    )
    .bind(address_id)
    .bind(app_id)
    .execute(pool)
    .await?;

    if result.rows_affected() == 0 {
        return Err(PartyError::NotFound(address_id));
    }
    Ok(())
}

// ============================================================================
// Helpers
// ============================================================================

async fn guard_party_exists(pool: &PgPool, app_id: &str, party_id: Uuid) -> Result<(), PartyError> {
    let exists: Option<(Uuid,)> = sqlx::query_as(
        "SELECT id FROM party_parties WHERE id = $1 AND app_id = $2",
    )
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
    let exists: Option<(Uuid,)> = sqlx::query_as(
        "SELECT id FROM party_parties WHERE id = $1 AND app_id = $2",
    )
    .bind(party_id)
    .bind(app_id)
    .fetch_optional(&mut **tx)
    .await?;

    if exists.is_none() {
        return Err(PartyError::NotFound(party_id));
    }
    Ok(())
}

async fn clear_primary_addresses(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
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
