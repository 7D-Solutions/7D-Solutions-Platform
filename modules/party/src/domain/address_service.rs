//! Address CRUD service — Guard→Mutation atomicity.

use chrono::Utc;
use sqlx::PgPool;
use uuid::Uuid;

use crate::db::{address_repo, address_repo::UpdateAddressData, party_repo};

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
    party_repo::guard_party_exists(pool, app_id, party_id).await?;
    address_repo::list_addresses(pool, app_id, party_id).await
}

/// Get a single address by ID, scoped to app_id.
pub async fn get_address(
    pool: &PgPool,
    app_id: &str,
    address_id: Uuid,
) -> Result<Option<Address>, PartyError> {
    address_repo::get_address(pool, app_id, address_id).await
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

    party_repo::guard_party_exists_tx(&mut tx, app_id, party_id).await?;

    if is_primary {
        address_repo::clear_primary_addresses_tx(&mut tx, app_id, party_id).await?;
    }

    let address = address_repo::insert_address_tx(
        &mut tx,
        address_id,
        party_id,
        app_id,
        req,
        addr_type,
        country,
        is_primary,
        now,
    )
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

    let current = address_repo::fetch_address_for_update_tx(&mut tx, app_id, address_id)
        .await?
        .ok_or(PartyError::NotFound(address_id))?;

    if req.is_primary == Some(true) && !current.is_primary {
        address_repo::clear_primary_addresses_tx(&mut tx, app_id, current.party_id).await?;
    }

    let new_type = req.address_type.as_deref().unwrap_or(&current.address_type);
    let new_label = if req.label.is_some() {
        req.label.clone()
    } else {
        current.label
    };
    let new_line1 = req
        .line1
        .as_deref()
        .map(|l| l.trim().to_string())
        .unwrap_or(current.line1);
    let new_line2 = if req.line2.is_some() {
        req.line2.clone()
    } else {
        current.line2
    };
    let new_city = req
        .city
        .as_deref()
        .map(|c| c.trim().to_string())
        .unwrap_or(current.city);
    let new_state = if req.state.is_some() {
        req.state.clone()
    } else {
        current.state
    };
    let new_postal = if req.postal_code.is_some() {
        req.postal_code.clone()
    } else {
        current.postal_code
    };
    let new_country = req
        .country
        .as_deref()
        .unwrap_or(&current.country)
        .to_string();
    let new_primary = req.is_primary.unwrap_or(current.is_primary);
    let new_metadata = if req.metadata.is_some() {
        req.metadata.clone()
    } else {
        current.metadata
    };

    let updated = address_repo::update_address_row_tx(
        &mut tx,
        &UpdateAddressData {
            address_id,
            app_id,
            addr_type: new_type,
            label: new_label,
            line1: new_line1,
            line2: new_line2,
            city: new_city,
            state: new_state,
            postal_code: new_postal,
            country: new_country,
            is_primary: new_primary,
            metadata: new_metadata,
            updated_at: now,
        },
    )
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
    let affected = address_repo::delete_address(pool, app_id, address_id).await?;
    if affected == 0 {
        return Err(PartyError::NotFound(address_id));
    }
    Ok(())
}
