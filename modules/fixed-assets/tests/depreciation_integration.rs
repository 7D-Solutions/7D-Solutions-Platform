//! Integrated tests for depreciation schedule generation and runs (bd-1g18).
//!
//! Covers:
//! 1. Schedule generates correct number of periods with correct totals
//! 2. Schedule generation is idempotent (no duplicate rows)
//! 3. Run posts correct periods with correct totals
//! 4. Run is idempotent (re-run posts 0 periods)
//! 5. Depreciation blocked after disposal (disposed asset skipped by run)
//! 6. Tenant isolation — run for tenant B does not post tenant A's periods

use chrono::NaiveDate;
use fixed_assets::domain::assets::{
    AssetRepo, CategoryRepo, CreateAssetRequest, CreateCategoryRequest, DepreciationMethod,
};
use fixed_assets::domain::depreciation::{
    CreateRunRequest, DepreciationService, GenerateScheduleRequest,
};
use fixed_assets::domain::disposals::{DisposalService, DisposalType, DisposeAssetRequest};
use serial_test::serial;
use sqlx::postgres::PgPoolOptions;
use uuid::Uuid;

async fn setup_db() -> sqlx::PgPool {
    dotenvy::dotenv().ok();
    let url = std::env::var("DATABASE_URL").unwrap_or_else(|_| {
        "postgresql://fixed_assets_user:fixed_assets_pass@localhost:5445/fixed_assets_db?sslmode=require"
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
    format!("depr-test-{}", Uuid::new_v4().simple())
}

/// Create a straight-line asset: 12-month life, cost 120_000, no salvage = 10_000/month.
async fn create_sl_asset(pool: &sqlx::PgPool, tenant_id: &str, in_service: NaiveDate) -> Uuid {
    let code = format!("CAT-{}", &Uuid::new_v4().to_string()[..8]);
    let cat_id = CategoryRepo::create(
        pool,
        &CreateCategoryRequest {
            tenant_id: tenant_id.to_string(),
            code,
            name: "Test Category".to_string(),
            description: None,
            default_method: Some(DepreciationMethod::StraightLine),
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

    let tag = format!("FA-{}", &Uuid::new_v4().to_string()[..8]);
    AssetRepo::create(
        pool,
        &CreateAssetRequest {
            tenant_id: tenant_id.to_string(),
            category_id: cat_id,
            asset_tag: tag,
            name: "Test Asset".to_string(),
            description: None,
            acquisition_date: in_service,
            in_service_date: Some(in_service),
            acquisition_cost_minor: 120_000,
            currency: None,
            depreciation_method: Some(DepreciationMethod::StraightLine),
            useful_life_months: Some(12),
            salvage_value_minor: Some(0),
            location: None,
            department: None,
            responsible_person: None,
            serial_number: None,
            vendor: None,
            purchase_order_ref: None,
            notes: None,
        },
    )
    .await
    .unwrap()
    .id
}

// ============================================================================
// 1. Schedule generates 12 periods with correct totals
// ============================================================================

#[tokio::test]
#[serial]
async fn test_schedule_generates_12_periods() {
    let pool = setup_db().await;
    let tid = unique_tenant();
    let in_service = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
    let asset_id = create_sl_asset(&pool, &tid, in_service).await;

    let schedules = DepreciationService::generate_schedule(
        &pool,
        &GenerateScheduleRequest {
            tenant_id: tid.clone(),
            asset_id,
        },
    )
    .await
    .unwrap();

    assert_eq!(schedules.len(), 12);
    let total: i64 = schedules.iter().map(|s| s.depreciation_amount_minor).sum();
    assert_eq!(total, 120_000, "total depreciation equals acquisition cost");
    assert_eq!(schedules[0].period_number, 1);
    assert_eq!(schedules[11].period_number, 12);
    assert_eq!(schedules[11].remaining_book_value_minor, 0);
}

// ============================================================================
// 2. Schedule generation is idempotent
// ============================================================================

#[tokio::test]
#[serial]
async fn test_schedule_idempotent() {
    let pool = setup_db().await;
    let tid = unique_tenant();
    let asset_id = create_sl_asset(&pool, &tid, NaiveDate::from_ymd_opt(2026, 1, 1).unwrap()).await;

    let req = GenerateScheduleRequest {
        tenant_id: tid.clone(),
        asset_id,
    };
    let first = DepreciationService::generate_schedule(&pool, &req)
        .await
        .unwrap();
    let second = DepreciationService::generate_schedule(&pool, &req)
        .await
        .unwrap();

    assert_eq!(
        first.len(),
        second.len(),
        "no duplicate rows on second call"
    );
    for (a, b) in first.iter().zip(second.iter()) {
        assert_eq!(a.id, b.id, "same row ids — no new inserts");
    }
}

// ============================================================================
// 3. Run posts correct periods with correct totals
// ============================================================================

#[tokio::test]
#[serial]
async fn test_depreciation_run_posts_correct_periods() {
    let pool = setup_db().await;
    let tid = unique_tenant();
    let asset_id = create_sl_asset(&pool, &tid, NaiveDate::from_ymd_opt(2026, 1, 1).unwrap()).await;

    DepreciationService::generate_schedule(
        &pool,
        &GenerateScheduleRequest {
            tenant_id: tid.clone(),
            asset_id,
        },
    )
    .await
    .unwrap();

    // Run through 2026-06-30 → 6 periods @ 10_000 each = 60_000
    let run = DepreciationService::run(
        &pool,
        &CreateRunRequest {
            tenant_id: tid.clone(),
            as_of_date: NaiveDate::from_ymd_opt(2026, 6, 30).unwrap(),
            currency: None,
            created_by: None,
        },
    )
    .await
    .unwrap();

    assert_eq!(run.status, "completed");
    assert_eq!(run.periods_posted, 6);
    assert_eq!(run.assets_processed, 1);
    assert_eq!(run.total_depreciation_minor, 60_000);
}

// ============================================================================
// 4. Run is idempotent — second run for same period posts 0
// ============================================================================

#[tokio::test]
#[serial]
async fn test_depreciation_run_idempotent() {
    let pool = setup_db().await;
    let tid = unique_tenant();
    let asset_id = create_sl_asset(&pool, &tid, NaiveDate::from_ymd_opt(2026, 1, 1).unwrap()).await;

    DepreciationService::generate_schedule(
        &pool,
        &GenerateScheduleRequest {
            tenant_id: tid.clone(),
            asset_id,
        },
    )
    .await
    .unwrap();

    let run_req = CreateRunRequest {
        tenant_id: tid.clone(),
        as_of_date: NaiveDate::from_ymd_opt(2026, 6, 30).unwrap(),
        currency: None,
        created_by: None,
    };

    DepreciationService::run(&pool, &run_req).await.unwrap();
    let run2 = DepreciationService::run(&pool, &run_req).await.unwrap();

    assert_eq!(
        run2.periods_posted, 0,
        "second run is idempotent — 0 new periods"
    );
    assert_eq!(run2.total_depreciation_minor, 0);
}

// ============================================================================
// 5. Depreciation blocked after disposal
// ============================================================================

#[tokio::test]
#[serial]
async fn test_depreciation_blocked_after_disposal() {
    let pool = setup_db().await;
    let tid = unique_tenant();
    let asset_id = create_sl_asset(&pool, &tid, NaiveDate::from_ymd_opt(2026, 1, 1).unwrap()).await;

    DepreciationService::generate_schedule(
        &pool,
        &GenerateScheduleRequest {
            tenant_id: tid.clone(),
            asset_id,
        },
    )
    .await
    .unwrap();

    // Post first 3 periods
    DepreciationService::run(
        &pool,
        &CreateRunRequest {
            tenant_id: tid.clone(),
            as_of_date: NaiveDate::from_ymd_opt(2026, 3, 31).unwrap(),
            currency: None,
            created_by: None,
        },
    )
    .await
    .unwrap();

    // Dispose the asset — sets status = 'disposed'
    DisposalService::dispose(
        &pool,
        &DisposeAssetRequest {
            tenant_id: tid.clone(),
            asset_id,
            disposal_type: DisposalType::Scrap,
            disposal_date: NaiveDate::from_ymd_opt(2026, 4, 1).unwrap(),
            proceeds_minor: None,
            reason: Some("Written off post-disposal".to_string()),
            buyer: None,
            reference: None,
            created_by: None,
        },
    )
    .await
    .unwrap();

    // Attempt to post remaining 9 periods — should post 0 (asset is disposed)
    let run_after = DepreciationService::run(
        &pool,
        &CreateRunRequest {
            tenant_id: tid.clone(),
            as_of_date: NaiveDate::from_ymd_opt(2026, 12, 31).unwrap(),
            currency: None,
            created_by: None,
        },
    )
    .await
    .unwrap();

    assert_eq!(
        run_after.periods_posted, 0,
        "depreciation must not be posted for disposed assets"
    );
    assert_eq!(run_after.total_depreciation_minor, 0);
}

// ============================================================================
// 6. Tenant isolation — run for tenant B does not post tenant A's periods
// ============================================================================

#[tokio::test]
#[serial]
async fn test_depreciation_tenant_isolation() {
    let pool = setup_db().await;
    let tid_a = unique_tenant();
    let tid_b = unique_tenant();
    let in_service = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();

    let asset_a = create_sl_asset(&pool, &tid_a, in_service).await;
    DepreciationService::generate_schedule(
        &pool,
        &GenerateScheduleRequest {
            tenant_id: tid_a.clone(),
            asset_id: asset_a,
        },
    )
    .await
    .unwrap();

    // Run for tenant B — should post 0 (no schedule for tenant B)
    let run_b = DepreciationService::run(
        &pool,
        &CreateRunRequest {
            tenant_id: tid_b.clone(),
            as_of_date: NaiveDate::from_ymd_opt(2026, 12, 31).unwrap(),
            currency: None,
            created_by: None,
        },
    )
    .await
    .unwrap();
    assert_eq!(
        run_b.periods_posted, 0,
        "tenant B run must not post tenant A's periods"
    );

    // Run for tenant A — should post all 12 periods
    let run_a = DepreciationService::run(
        &pool,
        &CreateRunRequest {
            tenant_id: tid_a.clone(),
            as_of_date: NaiveDate::from_ymd_opt(2026, 12, 31).unwrap(),
            currency: None,
            created_by: None,
        },
    )
    .await
    .unwrap();
    assert_eq!(run_a.periods_posted, 12);
    assert_eq!(run_a.total_depreciation_minor, 120_000);
}
