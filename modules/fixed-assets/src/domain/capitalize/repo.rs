//! Capitalization repository — SQL layer for fa_ap_capitalizations and related queries.
//!
//! All raw SQL lives here. The service layer calls these functions
//! for persistence and delegates business logic (guards, outbox) to itself.

use sqlx::PgConnection;
use uuid::Uuid;

use crate::domain::assets::models::Category;

// ============================================================================
// Reads (within transaction)
// ============================================================================

/// Check if a capitalization linkage already exists for (tenant, bill, line).
/// Returns the existing capitalization id if found.
pub async fn check_capitalization_exists(
    conn: &mut PgConnection,
    tenant_id: &str,
    bill_id: Uuid,
    line_id: Uuid,
) -> Result<Option<i64>, sqlx::Error> {
    let row: Option<(i64,)> = sqlx::query_as(
        "SELECT id FROM fa_ap_capitalizations \
         WHERE tenant_id = $1 AND bill_id = $2 AND line_id = $3",
    )
    .bind(tenant_id)
    .bind(bill_id)
    .bind(line_id)
    .fetch_optional(conn)
    .await?;

    Ok(row.map(|(id,)| id))
}

/// Find an active category by its asset_account_ref for a tenant.
pub async fn find_category_by_asset_account(
    conn: &mut PgConnection,
    tenant_id: &str,
    gl_account_code: &str,
) -> Result<Option<Category>, sqlx::Error> {
    sqlx::query_as(
        "SELECT * FROM fa_categories \
         WHERE tenant_id = $1 AND asset_account_ref = $2 AND is_active = TRUE \
         LIMIT 1",
    )
    .bind(tenant_id)
    .bind(gl_account_code)
    .fetch_optional(conn)
    .await
}

// ============================================================================
// Writes (within transaction)
// ============================================================================

/// Insert a new asset from a capitalized AP bill line.
#[allow(clippy::too_many_arguments)]
pub async fn insert_capitalized_asset(
    conn: &mut PgConnection,
    asset_id: Uuid,
    tenant_id: &str,
    category_id: Uuid,
    asset_tag: &str,
    name: &str,
    description: &str,
    acquisition_date: chrono::NaiveDate,
    amount_minor: i64,
    currency: &str,
    method: &str,
    useful_life_months: i32,
    nbv: i64,
    vendor: &str,
    purchase_order_ref: &str,
    notes: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        INSERT INTO fa_assets
            (id, tenant_id, category_id, asset_tag, name, description,
             status, acquisition_date, in_service_date,
             acquisition_cost_minor, currency, depreciation_method,
             useful_life_months, salvage_value_minor,
             accum_depreciation_minor, net_book_value_minor,
             asset_account_ref, depreciation_expense_ref, accum_depreciation_ref,
             vendor, purchase_order_ref, notes,
             created_at, updated_at)
        VALUES ($1,$2,$3,$4,$5,$6,'draft',$7,NULL,$8,$9,$10,$11,0,0,$12,
                NULL,NULL,NULL,$13,$14,$15,NOW(),NOW())
        "#,
    )
    .bind(asset_id)
    .bind(tenant_id)
    .bind(category_id)
    .bind(asset_tag)
    .bind(name)
    .bind(description)
    .bind(acquisition_date)
    .bind(amount_minor)
    .bind(currency)
    .bind(method)
    .bind(useful_life_months)
    .bind(nbv)
    .bind(vendor)
    .bind(purchase_order_ref)
    .bind(notes)
    .execute(conn)
    .await?;

    Ok(())
}

/// Insert a capitalization linkage and return the generated id.
#[allow(clippy::too_many_arguments)]
pub async fn insert_capitalization(
    conn: &mut PgConnection,
    tenant_id: &str,
    bill_id: Uuid,
    line_id: Uuid,
    asset_id: Uuid,
    gl_account_code: &str,
    amount_minor: i64,
    currency: &str,
    source_ref: &str,
) -> Result<i64, sqlx::Error> {
    let (cap_id,): (i64,) = sqlx::query_as(
        r#"
        INSERT INTO fa_ap_capitalizations
            (tenant_id, bill_id, line_id, asset_id, gl_account_code,
             amount_minor, currency, source_ref, created_at)
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, NOW())
        RETURNING id
        "#,
    )
    .bind(tenant_id)
    .bind(bill_id)
    .bind(line_id)
    .bind(asset_id)
    .bind(gl_account_code)
    .bind(amount_minor)
    .bind(currency)
    .bind(source_ref)
    .fetch_one(conn)
    .await?;

    Ok(cap_id)
}
