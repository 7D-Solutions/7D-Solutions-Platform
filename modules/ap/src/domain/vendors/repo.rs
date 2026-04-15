//! Vendor repository — SQL layer for the vendors table.
//!
//! All raw SQL for vendors lives here.
//! The service layer calls these functions and owns business logic
//! (validation, duplicate-name detection, outbox orchestration).

use chrono::DateTime;
use chrono::Utc;
use sqlx::PgConnection;
use sqlx::PgPool;
use uuid::Uuid;

use super::{Vendor, VendorError};

// ============================================================================
// Reads (pool-based)
// ============================================================================

/// Fetch a single vendor by ID and tenant. Returns None if not found.
pub async fn fetch_vendor(
    pool: &PgPool,
    tenant_id: &str,
    vendor_id: Uuid,
) -> Result<Option<Vendor>, VendorError> {
    let vendor = sqlx::query_as::<_, Vendor>(
        r#"
        SELECT vendor_id, tenant_id, name, tax_id, currency,
               payment_terms_days, payment_method, remittance_email,
               is_active, party_id, created_at, updated_at
        FROM vendors
        WHERE vendor_id = $1 AND tenant_id = $2
        "#,
    )
    .bind(vendor_id)
    .bind(tenant_id)
    .fetch_optional(pool)
    .await?;
    Ok(vendor)
}

/// List vendors for a tenant, ordered by name.
/// Pass `include_inactive = true` to include deactivated vendors.
pub async fn list_vendors(
    pool: &PgPool,
    tenant_id: &str,
    include_inactive: bool,
) -> Result<Vec<Vendor>, VendorError> {
    let vendors = if include_inactive {
        sqlx::query_as::<_, Vendor>(
            r#"
            SELECT vendor_id, tenant_id, name, tax_id, currency,
                   payment_terms_days, payment_method, remittance_email,
                   is_active, party_id, created_at, updated_at
            FROM vendors
            WHERE tenant_id = $1
            ORDER BY name ASC
            "#,
        )
        .bind(tenant_id)
        .fetch_all(pool)
        .await?
    } else {
        sqlx::query_as::<_, Vendor>(
            r#"
            SELECT vendor_id, tenant_id, name, tax_id, currency,
                   payment_terms_days, payment_method, remittance_email,
                   is_active, party_id, created_at, updated_at
            FROM vendors
            WHERE tenant_id = $1 AND is_active = TRUE
            ORDER BY name ASC
            "#,
        )
        .bind(tenant_id)
        .fetch_all(pool)
        .await?
    };
    Ok(vendors)
}

// ============================================================================
// Writes (conn-based — called within a transaction)
// ============================================================================

/// Check whether an active vendor with this name already exists for the tenant.
pub async fn check_duplicate_name(
    conn: &mut PgConnection,
    tenant_id: &str,
    name: &str,
) -> Result<bool, VendorError> {
    let row: Option<(Uuid,)> = sqlx::query_as(
        "SELECT vendor_id FROM vendors \
         WHERE tenant_id = $1 AND name = $2 AND is_active = TRUE LIMIT 1",
    )
    .bind(tenant_id)
    .bind(name)
    .fetch_optional(&mut *conn)
    .await?;
    Ok(row.is_some())
}

/// Insert a new vendor row. Returns the inserted record.
///
/// Maps the 23505 unique-violation to `VendorError::DuplicateName`.
pub async fn insert_vendor(
    conn: &mut PgConnection,
    vendor_id: Uuid,
    tenant_id: &str,
    name: &str,
    tax_id: &Option<String>,
    currency: &str,
    payment_terms_days: i32,
    payment_method: &Option<String>,
    remittance_email: &Option<String>,
    party_id: Option<Uuid>,
    now: DateTime<Utc>,
) -> Result<Vendor, VendorError> {
    let vendor: Vendor = sqlx::query_as(
        r#"
        INSERT INTO vendors (
            vendor_id, tenant_id, name, tax_id, currency,
            payment_terms_days, payment_method, remittance_email,
            is_active, party_id, created_at, updated_at
        )
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, TRUE, $9, $10, $10)
        RETURNING
            vendor_id, tenant_id, name, tax_id, currency,
            payment_terms_days, payment_method, remittance_email,
            is_active, party_id, created_at, updated_at
        "#,
    )
    .bind(vendor_id)
    .bind(tenant_id)
    .bind(name)
    .bind(tax_id)
    .bind(currency)
    .bind(payment_terms_days)
    .bind(payment_method)
    .bind(remittance_email)
    .bind(party_id)
    .bind(now)
    .fetch_one(&mut *conn)
    .await
    .map_err(|e| {
        if let sqlx::Error::Database(ref db_err) = e {
            if db_err.code().as_deref() == Some("23505") {
                return VendorError::DuplicateName(name.to_string());
            }
        }
        VendorError::Database(e)
    })?;
    Ok(vendor)
}

/// SELECT … FOR UPDATE on the vendors row. Used to lock before an update.
pub async fn lock_vendor_for_update(
    conn: &mut PgConnection,
    vendor_id: Uuid,
    tenant_id: &str,
) -> Result<Option<Vendor>, VendorError> {
    let vendor: Option<Vendor> = sqlx::query_as(
        r#"
        SELECT vendor_id, tenant_id, name, tax_id, currency,
               payment_terms_days, payment_method, remittance_email,
               is_active, party_id, created_at, updated_at
        FROM vendors
        WHERE vendor_id = $1 AND tenant_id = $2
        FOR UPDATE
        "#,
    )
    .bind(vendor_id)
    .bind(tenant_id)
    .fetch_optional(&mut *conn)
    .await?;
    Ok(vendor)
}

/// Update mutable vendor fields. Returns the updated record.
pub async fn update_vendor_row(
    conn: &mut PgConnection,
    vendor_id: Uuid,
    tenant_id: &str,
    name: &str,
    tax_id: &Option<String>,
    currency: &str,
    payment_terms_days: i32,
    payment_method: &Option<String>,
    remittance_email: &Option<String>,
    party_id: Option<Uuid>,
    now: DateTime<Utc>,
) -> Result<Vendor, VendorError> {
    let vendor: Vendor = sqlx::query_as(
        r#"
        UPDATE vendors
        SET name = $1, tax_id = $2, currency = $3,
            payment_terms_days = $4, payment_method = $5,
            remittance_email = $6, party_id = $7, updated_at = $8
        WHERE vendor_id = $9 AND tenant_id = $10
        RETURNING
            vendor_id, tenant_id, name, tax_id, currency,
            payment_terms_days, payment_method, remittance_email,
            is_active, party_id, created_at, updated_at
        "#,
    )
    .bind(name)
    .bind(tax_id)
    .bind(currency)
    .bind(payment_terms_days)
    .bind(payment_method)
    .bind(remittance_email)
    .bind(party_id)
    .bind(now)
    .bind(vendor_id)
    .bind(tenant_id)
    .fetch_one(&mut *conn)
    .await?;
    Ok(vendor)
}

/// Check whether a vendor row exists for the given tenant (used as guard before deactivation).
pub async fn vendor_exists(
    conn: &mut PgConnection,
    vendor_id: Uuid,
    tenant_id: &str,
) -> Result<bool, VendorError> {
    let row: Option<(bool,)> =
        sqlx::query_as("SELECT is_active FROM vendors WHERE vendor_id = $1 AND tenant_id = $2")
            .bind(vendor_id)
            .bind(tenant_id)
            .fetch_optional(&mut *conn)
            .await?;
    Ok(row.is_some())
}

/// Set is_active = FALSE on a vendor row.
pub async fn set_vendor_inactive(
    conn: &mut PgConnection,
    vendor_id: Uuid,
    tenant_id: &str,
    now: DateTime<Utc>,
) -> Result<(), VendorError> {
    sqlx::query(
        "UPDATE vendors SET is_active = FALSE, updated_at = $1 \
         WHERE vendor_id = $2 AND tenant_id = $3",
    )
    .bind(now)
    .bind(vendor_id)
    .bind(tenant_id)
    .execute(&mut *conn)
    .await?;
    Ok(())
}
