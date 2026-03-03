//! Integrated tests for asset disposal and impairment (bd-1g18).
//!
//! Covers:
//! 1. Sale disposal — proceeds > NBV yields positive gain
//! 2. Scrap disposal — no proceeds yields negative gain (loss)
//! 3. Double-dispose is idempotent — returns same disposal record
//! 4. Asset status is set correctly after disposal

use chrono::NaiveDate;
use fixed_assets::domain::assets::{CategoryRepo, CreateCategoryRequest};
use fixed_assets::domain::disposals::{DisposalService, DisposalType, DisposeAssetRequest};
use serial_test::serial;
use sqlx::postgres::PgPoolOptions;
use uuid::Uuid;

async fn setup_db() -> sqlx::PgPool {
    dotenvy::dotenv().ok();
    let url = std::env::var("DATABASE_URL").unwrap_or_else(|_| {
        "postgresql://fixed_assets_user:fixed_assets_pass@localhost:5445/fixed_assets_db?sslmode=disable"
            .to_string()
    });

    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&url)
        .await
        .expect("Failed to connect to fixed-assets test DB");

    sqlx::migrate!("db/migrations")
        .run(&pool)
        .await
        .expect("Failed to run fixed-assets migrations");

    pool
}

fn unique_tenant() -> String {
    format!("disp-test-{}", Uuid::new_v4().simple())
}

/// Insert an active asset: cost=120_000, accum=60_000, NBV=60_000.
/// Uses raw SQL to set status='active' and accum_depreciation (AssetRepo sets status='draft').
async fn create_active_asset(pool: &sqlx::PgPool, tenant_id: &str) -> Uuid {
    let code = format!("CAT-{}", &Uuid::new_v4().to_string()[..8]);
    let cat_id = CategoryRepo::create(
        pool,
        &CreateCategoryRequest {
            tenant_id: tenant_id.to_string(),
            code,
            name: "Test Category".to_string(),
            description: None,
            default_method: None,
            default_useful_life_months: Some(12),
            default_salvage_pct_bp: Some(0),
            asset_account_ref: "1500".to_string(),
            depreciation_expense_ref: "6100".to_string(),
            accum_depreciation_ref: "1510".to_string(),
            gain_loss_account_ref: Some("7000".to_string()),
        },
    )
    .await
    .unwrap()
    .id;

    let asset_id = Uuid::new_v4();
    let tag = format!("FA-{}", &Uuid::new_v4().to_string()[..8]);
    sqlx::query(
        r#"
        INSERT INTO fa_assets
            (id, tenant_id, category_id, asset_tag, name,
             status, acquisition_date, in_service_date,
             acquisition_cost_minor, currency,
             depreciation_method, useful_life_months, salvage_value_minor,
             accum_depreciation_minor, net_book_value_minor,
             created_at, updated_at)
        VALUES ($1,$2,$3,$4,'Active Asset',
                'active','2026-01-01','2026-01-01',
                120000,'usd','straight_line',12,0,60000,60000,NOW(),NOW())
        "#,
    )
    .bind(asset_id)
    .bind(tenant_id)
    .bind(cat_id)
    .bind(&tag)
    .execute(pool)
    .await
    .expect("insert active test asset");

    asset_id
}

fn dispose_req(
    tenant_id: &str,
    asset_id: Uuid,
    disposal_type: DisposalType,
    proceeds: Option<i64>,
) -> DisposeAssetRequest {
    DisposeAssetRequest {
        tenant_id: tenant_id.to_string(),
        asset_id,
        disposal_type,
        disposal_date: NaiveDate::from_ymd_opt(2026, 7, 1).unwrap(),
        proceeds_minor: proceeds,
        reason: Some("Test disposal".to_string()),
        buyer: None,
        reference: None,
        created_by: None,
    }
}

// ============================================================================
// 1. Sale disposal — gain = proceeds - NBV
// ============================================================================

#[tokio::test]
#[serial]
async fn test_disposal_sale_gain() {
    let pool = setup_db().await;
    let tid = unique_tenant();
    let asset_id = create_active_asset(&pool, &tid).await;

    let d = DisposalService::dispose(
        &pool,
        &dispose_req(&tid, asset_id, DisposalType::Sale, Some(80_000)),
    )
    .await
    .unwrap();

    assert_eq!(d.disposal_type, "sale");
    assert_eq!(d.net_book_value_at_disposal_minor, 60_000);
    assert_eq!(d.proceeds_minor, 80_000);
    assert_eq!(d.gain_loss_minor, 20_000, "gain = proceeds - NBV");
}

// ============================================================================
// 2. Scrap disposal — loss (no proceeds)
// ============================================================================

#[tokio::test]
#[serial]
async fn test_disposal_scrap_loss() {
    let pool = setup_db().await;
    let tid = unique_tenant();
    let asset_id = create_active_asset(&pool, &tid).await;

    let d = DisposalService::dispose(
        &pool,
        &dispose_req(&tid, asset_id, DisposalType::Scrap, None),
    )
    .await
    .unwrap();

    assert_eq!(d.disposal_type, "scrap");
    assert_eq!(d.proceeds_minor, 0);
    assert_eq!(d.gain_loss_minor, -60_000, "loss = 0 - NBV");
}

// ============================================================================
// 3. Double-dispose is idempotent
// ============================================================================

#[tokio::test]
#[serial]
async fn test_double_dispose_idempotent() {
    let pool = setup_db().await;
    let tid = unique_tenant();
    let asset_id = create_active_asset(&pool, &tid).await;

    let req = dispose_req(&tid, asset_id, DisposalType::Sale, Some(50_000));
    let d1 = DisposalService::dispose(&pool, &req).await.unwrap();
    let d2 = DisposalService::dispose(&pool, &req).await.unwrap();

    assert_eq!(d1.id, d2.id, "second dispose returns the same record");
}

// ============================================================================
// 4. Asset status set to 'disposed' after sale, 'impaired' after impairment
// ============================================================================

#[tokio::test]
#[serial]
async fn test_disposal_sets_asset_status() {
    let pool = setup_db().await;
    let tid = unique_tenant();
    let asset_id = create_active_asset(&pool, &tid).await;

    DisposalService::dispose(
        &pool,
        &dispose_req(&tid, asset_id, DisposalType::Sale, Some(50_000)),
    )
    .await
    .unwrap();

    let (status,): (String,) = sqlx::query_as("SELECT status FROM fa_assets WHERE id = $1")
        .bind(asset_id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(status, "disposed");
}

#[tokio::test]
#[serial]
async fn test_impairment_sets_impaired_status() {
    let pool = setup_db().await;
    let tid = unique_tenant();
    let asset_id = create_active_asset(&pool, &tid).await;

    DisposalService::dispose(
        &pool,
        &dispose_req(&tid, asset_id, DisposalType::Impairment, None),
    )
    .await
    .unwrap();

    let (status,): (String,) = sqlx::query_as("SELECT status FROM fa_assets WHERE id = $1")
        .bind(asset_id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(status, "impaired");
}
