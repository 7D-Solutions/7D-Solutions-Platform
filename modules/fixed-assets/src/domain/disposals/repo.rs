//! Disposal repository — SQL layer for fa_disposals and related queries.
//!
//! All raw SQL lives here. The service layer calls these functions
//! for persistence and delegates business logic (guards, financial
//! computation, outbox orchestration) to its own methods.

use sqlx::{PgConnection, PgPool};
use uuid::Uuid;

use super::Disposal;

/// Minimal projection for disposal — avoids decoding custom PG types.
#[derive(sqlx::FromRow)]
pub struct AssetForDisposal {
    #[allow(dead_code)]
    pub id: Uuid,
    pub category_id: Uuid,
    pub status: String,
    pub acquisition_cost_minor: i64,
    pub accum_depreciation_minor: i64,
    pub net_book_value_minor: i64,
    pub currency: String,
    pub asset_account_ref: Option<String>,
    pub accum_depreciation_ref: Option<String>,
}

/// Category account refs needed for GL posting.
#[derive(sqlx::FromRow)]
pub struct CategoryAccounts {
    pub asset_account_ref: String,
    pub accum_depreciation_ref: String,
    pub gain_loss_account_ref: Option<String>,
}

/// Select columns for Disposal, casting disposal_type to TEXT.
const DISPOSAL_COLUMNS: &str = r#"
    id, tenant_id, asset_id, disposal_type::TEXT as disposal_type,
    disposal_date, net_book_value_at_disposal_minor,
    proceeds_minor, gain_loss_minor, currency,
    reason, buyer, reference, journal_entry_ref,
    is_posted, posted_at, created_by, approved_by,
    created_at, updated_at
"#;

// ============================================================================
// Reads
// ============================================================================

/// Fetch asset data needed for disposal with row lock (FOR UPDATE).
pub async fn fetch_asset_for_disposal(
    conn: &mut PgConnection,
    asset_id: Uuid,
    tenant_id: &str,
) -> Result<Option<AssetForDisposal>, sqlx::Error> {
    sqlx::query_as::<_, AssetForDisposal>(
        r#"
        SELECT id, category_id, status,
               acquisition_cost_minor, accum_depreciation_minor,
               net_book_value_minor, currency,
               asset_account_ref, accum_depreciation_ref
        FROM fa_assets
        WHERE id = $1 AND tenant_id = $2
        FOR UPDATE
        "#,
    )
    .bind(asset_id)
    .bind(tenant_id)
    .fetch_optional(conn)
    .await
}

/// Fetch the most recent existing disposal for an asset (for idempotency).
pub async fn fetch_existing_disposal(
    conn: &mut PgConnection,
    asset_id: Uuid,
    tenant_id: &str,
) -> Result<Option<Disposal>, sqlx::Error> {
    sqlx::query_as::<_, Disposal>(&format!(
        "SELECT {} FROM fa_disposals WHERE asset_id = $1 AND tenant_id = $2 \
         ORDER BY created_at DESC LIMIT 1",
        DISPOSAL_COLUMNS
    ))
    .bind(asset_id)
    .bind(tenant_id)
    .fetch_optional(conn)
    .await
}

/// Fetch category GL account refs for disposal posting.
pub async fn fetch_category_accounts(
    conn: &mut PgConnection,
    category_id: Uuid,
    tenant_id: &str,
) -> Result<Option<CategoryAccounts>, sqlx::Error> {
    sqlx::query_as::<_, CategoryAccounts>(
        "SELECT asset_account_ref, accum_depreciation_ref, gain_loss_account_ref \
         FROM fa_categories WHERE id = $1 AND tenant_id = $2",
    )
    .bind(category_id)
    .bind(tenant_id)
    .fetch_optional(conn)
    .await
}

/// List all disposals for a tenant.
pub async fn list_disposals(
    pool: &PgPool,
    tenant_id: &str,
) -> Result<Vec<Disposal>, sqlx::Error> {
    sqlx::query_as::<_, Disposal>(&format!(
        "SELECT {} FROM fa_disposals WHERE tenant_id = $1 ORDER BY disposal_date DESC",
        DISPOSAL_COLUMNS
    ))
    .bind(tenant_id)
    .fetch_all(pool)
    .await
}

/// Fetch a single disposal by id.
pub async fn get_disposal(
    pool: &PgPool,
    id: Uuid,
    tenant_id: &str,
) -> Result<Option<Disposal>, sqlx::Error> {
    sqlx::query_as::<_, Disposal>(&format!(
        "SELECT {} FROM fa_disposals WHERE id = $1 AND tenant_id = $2",
        DISPOSAL_COLUMNS
    ))
    .bind(id)
    .bind(tenant_id)
    .fetch_optional(pool)
    .await
}

// ============================================================================
// Writes (within transaction)
// ============================================================================

/// Insert a disposal record.
#[allow(clippy::too_many_arguments)]
pub async fn insert_disposal(
    conn: &mut PgConnection,
    disposal_id: Uuid,
    tenant_id: &str,
    asset_id: Uuid,
    disposal_type: &str,
    disposal_date: chrono::NaiveDate,
    nbv: i64,
    proceeds: i64,
    gain_loss: i64,
    currency: &str,
    reason: Option<&str>,
    buyer: Option<&str>,
    reference: Option<&str>,
    created_by: Option<&str>,
) -> Result<Disposal, sqlx::Error> {
    sqlx::query_as::<_, Disposal>(&format!(
        r#"
        INSERT INTO fa_disposals
            (id, tenant_id, asset_id, disposal_type, disposal_date,
             net_book_value_at_disposal_minor, proceeds_minor, gain_loss_minor,
             currency, reason, buyer, reference, created_by,
             created_at, updated_at)
        VALUES ($1, $2, $3, $4::fa_disposal_type, $5, $6, $7, $8, $9,
                $10, $11, $12, $13, NOW(), NOW())
        RETURNING {}
        "#,
        DISPOSAL_COLUMNS
    ))
    .bind(disposal_id)
    .bind(tenant_id)
    .bind(asset_id)
    .bind(disposal_type)
    .bind(disposal_date)
    .bind(nbv)
    .bind(proceeds)
    .bind(gain_loss)
    .bind(currency)
    .bind(reason)
    .bind(buyer)
    .bind(reference)
    .bind(created_by)
    .fetch_one(conn)
    .await
}

/// Update asset status and zero NBV after disposal.
pub async fn update_asset_status_disposed(
    conn: &mut PgConnection,
    asset_id: Uuid,
    tenant_id: &str,
    target_status: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE fa_assets SET status = $3, net_book_value_minor = 0, updated_at = NOW() \
         WHERE id = $1 AND tenant_id = $2",
    )
    .bind(asset_id)
    .bind(tenant_id)
    .bind(target_status)
    .execute(conn)
    .await?;

    Ok(())
}
