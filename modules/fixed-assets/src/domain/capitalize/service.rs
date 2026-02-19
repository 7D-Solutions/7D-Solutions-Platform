//! Capitalization service: create fixed assets from AP bill lines.
//!
//! Triggered by `ap.vendor_bill_approved` events. For each bill line whose
//! `gl_account_code` maps to an active `fa_category.asset_account_ref`, an
//! asset is created and a linkage row written to `fa_ap_capitalizations`.
//!
//! ## Idempotency
//! `fa_ap_capitalizations` has a `UNIQUE (tenant_id, bill_id, line_id)` constraint.
//! If a row already exists for `(bill_id, line_id)`, processing is skipped and
//! `Ok(None)` is returned — safe under event replay.
//!
//! ## No AP DB writes
//! This service reads only the fixed-assets DB (fa_categories, fa_assets,
//! fa_ap_capitalizations). It never touches the AP database.

use chrono::NaiveDate;
use sqlx::PgPool;
use uuid::Uuid;

use crate::domain::assets::models::{Category, DepreciationMethod};
use crate::outbox;

/// Outcome of a single capitalization attempt.
#[derive(Debug)]
pub struct CapitalizationResult {
    pub asset_id: Uuid,
    pub capitalization_id: i64,
}

/// Input for one bill line capitalization.
#[derive(Debug)]
pub struct CapitalizeFromApLineRequest {
    pub tenant_id: String,
    pub bill_id: Uuid,
    pub line_id: Uuid,
    pub gl_account_code: String,
    pub amount_minor: i64,
    pub currency: String,
    /// Date the bill was approved (used as acquisition_date).
    pub acquisition_date: NaiveDate,
    /// Human-readable source reference for the asset (vendor invoice ref).
    pub vendor_invoice_ref: String,
}

/// Attempt to capitalize one AP bill line.
///
/// Guard:    Check idempotency table; look up active category by asset_account_ref.
/// Mutation: Insert asset + capitalization linkage atomically.
/// Outbox:   Emits asset_created event.
///
/// Returns `Ok(Some(_))` if a new asset was created, `Ok(None)` if:
///   - already processed (idempotent replay), or
///   - no category maps to this gl_account_code (non-capex line).
pub async fn capitalize_from_ap_line(
    pool: &PgPool,
    req: &CapitalizeFromApLineRequest,
) -> Result<Option<CapitalizationResult>, sqlx::Error> {
    if req.amount_minor <= 0 {
        tracing::debug!(
            bill_id = %req.bill_id,
            line_id = %req.line_id,
            "capitalize: skipping zero/negative amount line"
        );
        return Ok(None);
    }

    let mut tx = pool.begin().await?;

    // Guard: idempotency check
    let existing: Option<(i64,)> = sqlx::query_as(
        "SELECT id FROM fa_ap_capitalizations \
         WHERE tenant_id = $1 AND bill_id = $2 AND line_id = $3",
    )
    .bind(&req.tenant_id)
    .bind(req.bill_id)
    .bind(req.line_id)
    .fetch_optional(&mut *tx)
    .await?;

    if let Some((cap_id,)) = existing {
        tracing::debug!(
            bill_id = %req.bill_id,
            line_id = %req.line_id,
            cap_id = cap_id,
            "capitalize: already processed (idempotent replay)"
        );
        tx.commit().await?;
        return Ok(None);
    }

    // Guard: look up active category by asset_account_ref
    let category: Option<Category> = sqlx::query_as(
        "SELECT * FROM fa_categories \
         WHERE tenant_id = $1 AND asset_account_ref = $2 AND is_active = TRUE \
         LIMIT 1",
    )
    .bind(&req.tenant_id)
    .bind(&req.gl_account_code)
    .fetch_optional(&mut *tx)
    .await?;

    let category = match category {
        Some(c) => c,
        None => {
            tracing::debug!(
                tenant_id = %req.tenant_id,
                gl_account_code = %req.gl_account_code,
                "capitalize: no active category for gl_account_code; non-capex line"
            );
            tx.commit().await?;
            return Ok(None);
        }
    };

    // Compute asset parameters from category defaults
    let method = DepreciationMethod::try_from(category.default_method.clone())
        .unwrap_or(DepreciationMethod::StraightLine);
    let life_months = category.default_useful_life_months;
    let salvage = 0i64; // Salvage derived at disposal time; zero at acquisition
    let nbv = req.amount_minor - salvage;

    // Asset tag: AP-{line_id} — globally unique since line_id is UUID
    let asset_tag = format!("AP-{}", req.line_id);
    let asset_id = Uuid::new_v4();

    // Mutation: insert asset
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
    .bind(asset_id)                                        // $1
    .bind(&req.tenant_id)                                  // $2
    .bind(category.id)                                     // $3
    .bind(&asset_tag)                                      // $4
    .bind(&req.vendor_invoice_ref)                         // $5 name
    .bind(format!("Capitalized from AP bill {}", req.bill_id)) // $6 description
    .bind(req.acquisition_date)                            // $7
    .bind(req.amount_minor)                                // $8 acquisition_cost_minor
    .bind(req.currency.to_lowercase())                     // $9
    .bind(method.as_str())                                 // $10
    .bind(life_months)                                     // $11
    .bind(nbv)                                             // $12 net_book_value_minor
    .bind(&req.vendor_invoice_ref)                         // $13 vendor
    .bind(req.bill_id.to_string())                         // $14 purchase_order_ref
    .bind(format!("Source: AP bill {} line {}", req.bill_id, req.line_id)) // $15 notes
    .execute(&mut *tx)
    .await?;

    // Mutation: insert capitalization linkage
    let (cap_id,): (i64,) = sqlx::query_as(
        r#"
        INSERT INTO fa_ap_capitalizations
            (tenant_id, bill_id, line_id, asset_id, gl_account_code,
             amount_minor, currency, source_ref, created_at)
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, NOW())
        RETURNING id
        "#,
    )
    .bind(&req.tenant_id)
    .bind(req.bill_id)
    .bind(req.line_id)
    .bind(asset_id)
    .bind(&req.gl_account_code)
    .bind(req.amount_minor)
    .bind(req.currency.to_lowercase())
    .bind(format!("{}:{}", req.bill_id, req.line_id))
    .fetch_one(&mut *tx)
    .await?;

    // Outbox: emit asset_created
    use crate::domain::assets::models::AssetCreatedEvent;
    let event = AssetCreatedEvent {
        asset_id,
        tenant_id: req.tenant_id.clone(),
        asset_tag: asset_tag.clone(),
        category_id: category.id,
        acquisition_cost_minor: req.amount_minor,
        currency: req.currency.to_lowercase(),
    };
    outbox::enqueue_event_tx(
        &mut tx,
        &req.tenant_id,
        Uuid::new_v4(),
        "asset_created",
        "fa_asset",
        &asset_id.to_string(),
        &event,
    )
    .await?;

    tx.commit().await?;

    tracing::info!(
        tenant_id = %req.tenant_id,
        bill_id = %req.bill_id,
        line_id = %req.line_id,
        asset_id = %asset_id,
        category_code = %category.code,
        amount_minor = req.amount_minor,
        "capitalize: created asset from AP bill line"
    );

    Ok(Some(CapitalizationResult {
        asset_id,
        capitalization_id: cap_id,
    }))
}

// ============================================================================
// Integrated Tests (real DB, no mocks)
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;
    use serial_test::serial;

    const TEST_TENANT: &str = "test-tenant-capitalize";

    fn test_db_url() -> String {
        std::env::var("DATABASE_URL").unwrap_or_else(|_| {
            "postgres://fixed_assets_user:fixed_assets_pass@localhost:5445/fixed_assets_db"
                .to_string()
        })
    }

    async fn test_pool() -> PgPool {
        PgPool::connect(&test_db_url())
            .await
            .expect("Failed to connect to FA test DB")
    }

    async fn setup_category(pool: &PgPool) -> Uuid {
        let id = Uuid::new_v4();
        sqlx::query(
            r#"
            INSERT INTO fa_categories
                (id, tenant_id, code, name,
                 default_method, default_useful_life_months, default_salvage_pct_bp,
                 asset_account_ref, depreciation_expense_ref, accum_depreciation_ref,
                 is_active, created_at, updated_at)
            VALUES ($1, $2, $3, $4, 'straight_line', 60, 0, '1500', '6100', '1510',
                    TRUE, NOW(), NOW())
            "#,
        )
        .bind(id)
        .bind(TEST_TENANT)
        .bind(format!("EQUIP-{}", &id.to_string()[..8]))
        .bind(format!("Equipment-{}", &id.to_string()[..8]))
        .execute(pool)
        .await
        .expect("insert category");
        id
    }

    async fn cleanup(pool: &PgPool) {
        sqlx::query(
            "DELETE FROM fa_ap_capitalizations WHERE tenant_id = $1",
        )
        .bind(TEST_TENANT)
        .execute(pool)
        .await
        .ok();

        sqlx::query(
            "DELETE FROM fa_events_outbox WHERE tenant_id = $1",
        )
        .bind(TEST_TENANT)
        .execute(pool)
        .await
        .ok();

        sqlx::query(
            "DELETE FROM fa_assets WHERE tenant_id = $1",
        )
        .bind(TEST_TENANT)
        .execute(pool)
        .await
        .ok();

        sqlx::query(
            "DELETE FROM fa_categories WHERE tenant_id = $1",
        )
        .bind(TEST_TENANT)
        .execute(pool)
        .await
        .ok();
    }

    fn sample_req(bill_id: Uuid, line_id: Uuid, gl_account_code: &str) -> CapitalizeFromApLineRequest {
        CapitalizeFromApLineRequest {
            tenant_id: TEST_TENANT.to_string(),
            bill_id,
            line_id,
            gl_account_code: gl_account_code.to_string(),
            amount_minor: 100_000,
            currency: "USD".to_string(),
            acquisition_date: NaiveDate::from_ymd_opt(2026, 2, 18).unwrap(),
            vendor_invoice_ref: "INV-TEST-001".to_string(),
        }
    }

    #[tokio::test]
    #[serial]
    async fn test_capex_line_creates_asset_and_linkage() {
        let pool = test_pool().await;
        cleanup(&pool).await;
        let _cat_id = setup_category(&pool).await;

        let bill_id = Uuid::new_v4();
        let line_id = Uuid::new_v4();
        let req = sample_req(bill_id, line_id, "1500");

        let result = capitalize_from_ap_line(&pool, &req)
            .await
            .expect("capitalize failed");
        let result = result.expect("expected Some(result) for capex line");

        // Verify asset was created
        let (count,): (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM fa_assets WHERE id = $1 AND tenant_id = $2",
        )
        .bind(result.asset_id)
        .bind(TEST_TENANT)
        .fetch_one(&pool)
        .await
        .expect("asset count");
        assert_eq!(count, 1, "asset must be created");

        // Verify linkage was created
        let (link_count,): (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM fa_ap_capitalizations \
             WHERE bill_id = $1 AND line_id = $2 AND tenant_id = $3",
        )
        .bind(bill_id)
        .bind(line_id)
        .bind(TEST_TENANT)
        .fetch_one(&pool)
        .await
        .expect("linkage count");
        assert_eq!(link_count, 1, "linkage must be created");

        cleanup(&pool).await;
    }

    #[tokio::test]
    #[serial]
    async fn test_non_capex_line_returns_none() {
        let pool = test_pool().await;
        cleanup(&pool).await;
        let _cat_id = setup_category(&pool).await;

        // Use expense account "6200" — no category maps to it
        let req = sample_req(Uuid::new_v4(), Uuid::new_v4(), "6200");

        let result = capitalize_from_ap_line(&pool, &req)
            .await
            .expect("capitalize failed");
        assert!(result.is_none(), "expense line must return None");

        cleanup(&pool).await;
    }

    #[tokio::test]
    #[serial]
    async fn test_idempotent_on_replay() {
        let pool = test_pool().await;
        cleanup(&pool).await;
        let _cat_id = setup_category(&pool).await;

        let bill_id = Uuid::new_v4();
        let line_id = Uuid::new_v4();
        let req = sample_req(bill_id, line_id, "1500");

        // First call: creates asset
        let first = capitalize_from_ap_line(&pool, &req)
            .await
            .expect("first capitalize failed");
        assert!(first.is_some(), "first call must create asset");

        // Second call: idempotent — must return None without creating duplicate
        let second = capitalize_from_ap_line(&pool, &req)
            .await
            .expect("second capitalize failed");
        assert!(second.is_none(), "second call must be idempotent (return None)");

        // Only one asset should exist
        let (count,): (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM fa_assets WHERE tenant_id = $1",
        )
        .bind(TEST_TENANT)
        .fetch_one(&pool)
        .await
        .expect("asset count");
        assert_eq!(count, 1, "idempotent replay must not duplicate assets");

        cleanup(&pool).await;
    }

    #[tokio::test]
    #[serial]
    async fn test_source_ref_stored_correctly() {
        let pool = test_pool().await;
        cleanup(&pool).await;
        let _cat_id = setup_category(&pool).await;

        let bill_id = Uuid::new_v4();
        let line_id = Uuid::new_v4();
        let req = sample_req(bill_id, line_id, "1500");

        capitalize_from_ap_line(&pool, &req)
            .await
            .expect("capitalize failed");

        let (source_ref,): (String,) = sqlx::query_as(
            "SELECT source_ref FROM fa_ap_capitalizations \
             WHERE bill_id = $1 AND line_id = $2",
        )
        .bind(bill_id)
        .bind(line_id)
        .fetch_one(&pool)
        .await
        .expect("source_ref query");

        let expected = format!("{}:{}", bill_id, line_id);
        assert_eq!(source_ref, expected, "source_ref must be bill_id:line_id");

        cleanup(&pool).await;
    }

    #[tokio::test]
    #[serial]
    async fn test_zero_amount_returns_none() {
        let pool = test_pool().await;
        cleanup(&pool).await;
        let _cat_id = setup_category(&pool).await;

        let mut req = sample_req(Uuid::new_v4(), Uuid::new_v4(), "1500");
        req.amount_minor = 0;

        let result = capitalize_from_ap_line(&pool, &req)
            .await
            .expect("capitalize failed");
        assert!(result.is_none(), "zero amount must return None");

        cleanup(&pool).await;
    }
}
