//! Integrated tests for DisposalService — require running fixed-assets Postgres instance.

use chrono::NaiveDate;
use serial_test::serial;
use sqlx::PgPool;
use uuid::Uuid;

use super::{Disposal, DisposalService, DisposalType, DisposeAssetRequest};

fn test_db_url() -> String {
    std::env::var("DATABASE_URL").unwrap_or_else(|_| {
        "postgres://fixed_assets_user:fixed_assets_pass@localhost:5445/fixed_assets_db?sslmode=require"
            .to_string()
    })
}

async fn test_pool() -> PgPool {
    PgPool::connect(&test_db_url())
        .await
        .expect("connect to fixed-assets test DB")
}

const TEST_TENANT: &str = "test-disposal-svc";

async fn cleanup(pool: &PgPool) {
    for q in [
        "DELETE FROM fa_disposals WHERE tenant_id = $1",
        "DELETE FROM fa_depreciation_schedules WHERE tenant_id = $1",
        "DELETE FROM fa_depreciation_runs WHERE tenant_id = $1",
        "DELETE FROM fa_events_outbox WHERE tenant_id = $1",
        "DELETE FROM fa_assets WHERE tenant_id = $1",
        "DELETE FROM fa_categories WHERE tenant_id = $1",
    ] {
        sqlx::query(q).bind(TEST_TENANT).execute(pool).await.ok();
    }
}

/// Insert category + active asset (cost=120000, accum=60000, NBV=60000).
async fn setup_active_asset(pool: &PgPool) -> (Uuid, Uuid) {
    let cat_id = Uuid::new_v4();
    let tag = format!("DSP-{}", &cat_id.to_string()[..8]);
    sqlx::query(
        r#"
        INSERT INTO fa_categories
            (id, tenant_id, code, name,
             default_method, default_useful_life_months, default_salvage_pct_bp,
             asset_account_ref, depreciation_expense_ref, accum_depreciation_ref,
             gain_loss_account_ref, is_active, created_at, updated_at)
        VALUES ($1,$2,$3,$4,'straight_line',12,0,'1500','6100','1510',
                '7000',TRUE,NOW(),NOW())
        "#,
    )
    .bind(cat_id)
    .bind(TEST_TENANT)
    .bind(tag.clone())
    .bind(format!("Category {}", tag))
    .execute(pool)
    .await
    .expect("insert test category");

    let asset_id = Uuid::new_v4();
    let atag = format!("FA-{}", &asset_id.to_string()[..8]);
    sqlx::query(
        r#"
        INSERT INTO fa_assets
            (id, tenant_id, category_id, asset_tag, name,
             status, acquisition_date, in_service_date,
             acquisition_cost_minor, currency,
             depreciation_method, useful_life_months, salvage_value_minor,
             accum_depreciation_minor, net_book_value_minor,
             created_at, updated_at)
        VALUES ($1,$2,$3,$4,$5,'active','2026-01-01','2026-01-01',
                120000,'usd','straight_line',12,0,60000,60000,NOW(),NOW())
        "#,
    )
    .bind(asset_id)
    .bind(TEST_TENANT)
    .bind(cat_id)
    .bind(atag)
    .bind("Test Disposal Asset")
    .execute(pool)
    .await
    .expect("insert test asset");

    (cat_id, asset_id)
}

#[tokio::test]
#[serial]
async fn dispose_sale_computes_gain() {
    let pool = test_pool().await;
    cleanup(&pool).await;
    let (_, asset_id) = setup_active_asset(&pool).await;

    let req = DisposeAssetRequest {
        tenant_id: TEST_TENANT.into(),
        asset_id,
        disposal_type: DisposalType::Sale,
        disposal_date: NaiveDate::from_ymd_opt(2026, 7, 1).expect("valid test date"),
        proceeds_minor: Some(80000),
        reason: Some("Upgrading".into()),
        buyer: Some("Acme Corp".into()),
        reference: None,
        created_by: None,
    };

    let d = DisposalService::dispose(&pool, &req)
        .await
        .expect("dispose failed");
    assert_eq!(d.disposal_type, "sale");
    assert_eq!(d.net_book_value_at_disposal_minor, 60000);
    assert_eq!(d.proceeds_minor, 80000);
    assert_eq!(d.gain_loss_minor, 20000);

    let (status,): (String,) =
        sqlx::query_as("SELECT status FROM fa_assets WHERE id = $1 AND tenant_id = $2")
            .bind(asset_id)
            .bind(TEST_TENANT)
            .fetch_one(&pool)
            .await
            .expect("status query failed");
    assert_eq!(status, "disposed");

    cleanup(&pool).await;
}

#[tokio::test]
#[serial]
async fn dispose_scrap_computes_loss() {
    let pool = test_pool().await;
    cleanup(&pool).await;
    let (_, asset_id) = setup_active_asset(&pool).await;

    let req = DisposeAssetRequest {
        tenant_id: TEST_TENANT.into(),
        asset_id,
        disposal_type: DisposalType::Scrap,
        disposal_date: NaiveDate::from_ymd_opt(2026, 7, 1).expect("valid test date"),
        proceeds_minor: None,
        reason: Some("Broken".into()),
        buyer: None,
        reference: None,
        created_by: None,
    };

    let d = DisposalService::dispose(&pool, &req)
        .await
        .expect("dispose failed");
    assert_eq!(d.disposal_type, "scrap");
    assert_eq!(d.gain_loss_minor, -60000);

    cleanup(&pool).await;
}

#[tokio::test]
#[serial]
async fn dispose_is_idempotent() {
    let pool = test_pool().await;
    cleanup(&pool).await;
    let (_, asset_id) = setup_active_asset(&pool).await;

    let req = DisposeAssetRequest {
        tenant_id: TEST_TENANT.into(),
        asset_id,
        disposal_type: DisposalType::Sale,
        disposal_date: NaiveDate::from_ymd_opt(2026, 7, 1).expect("valid test date"),
        proceeds_minor: Some(50000),
        reason: None,
        buyer: None,
        reference: None,
        created_by: None,
    };

    let d1 = DisposalService::dispose(&pool, &req)
        .await
        .expect("dispose d1 failed");
    let d2 = DisposalService::dispose(&pool, &req)
        .await
        .expect("dispose d2 failed");
    assert_eq!(d1.id, d2.id, "idempotent — same disposal returned");

    cleanup(&pool).await;
}

#[tokio::test]
#[serial]
async fn impairment_sets_impaired_status() {
    let pool = test_pool().await;
    cleanup(&pool).await;
    let (_, asset_id) = setup_active_asset(&pool).await;

    let req = DisposeAssetRequest {
        tenant_id: TEST_TENANT.into(),
        asset_id,
        disposal_type: DisposalType::Impairment,
        disposal_date: NaiveDate::from_ymd_opt(2026, 7, 1).expect("valid test date"),
        proceeds_minor: None,
        reason: Some("Market value decline".into()),
        buyer: None,
        reference: None,
        created_by: None,
    };

    let d = DisposalService::dispose(&pool, &req)
        .await
        .expect("dispose failed");
    assert_eq!(d.disposal_type, "impairment");
    assert_eq!(d.gain_loss_minor, -60000);

    let (status,): (String,) =
        sqlx::query_as("SELECT status FROM fa_assets WHERE id = $1 AND tenant_id = $2")
            .bind(asset_id)
            .bind(TEST_TENANT)
            .fetch_one(&pool)
            .await
            .expect("status query failed");
    assert_eq!(status, "impaired");

    cleanup(&pool).await;
}
