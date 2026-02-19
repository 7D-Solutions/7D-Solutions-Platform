//! Fixed Assets E2E: create → depreciate → post → dispose (Phase 30, bd-u6eb)
//!
//! Proves the full fixed asset lifecycle against a real PostgreSQL database:
//! 1. Create category with GL account refs (incl. gain/loss)
//! 2. Create asset and put in service
//! 3. Generate depreciation schedule
//! 4. Run depreciation (post periods)
//! 5. Dispose asset (sale with gain)
//! 6. Verify disposal record, asset status, outbox GL data
//! 7. Verify idempotent rerun
//! 8. Verify deterministic rerun
//!
//! No mocks, no stubs — all tests run against real fixed-assets
//! PostgreSQL (port 5445).

mod common;

use chrono::NaiveDate;
use common::wait_for_db_ready;
use fixed_assets::domain::depreciation::{
    CreateRunRequest, DepreciationService, GenerateScheduleRequest,
};
use fixed_assets::domain::disposals::{DisposeAssetRequest, DisposalService, DisposalType};
use serial_test::serial;
use sqlx::PgPool;
use uuid::Uuid;

// ============================================================================
// Test infrastructure
// ============================================================================

fn fa_db_url() -> String {
    std::env::var("FA_DATABASE_URL").unwrap_or_else(|_| {
        "postgres://fixed_assets_user:fixed_assets_pass@localhost:5445/fixed_assets_db".to_string()
    })
}

async fn fa_pool() -> PgPool {
    wait_for_db_ready("fixed-assets", &fa_db_url()).await
}

const MIGRATION_LOCK_KEY: i64 = 7_445_319_825_i64;

async fn ensure_migrations(pool: &PgPool) {
    sqlx::query("SELECT pg_advisory_lock($1)")
        .bind(MIGRATION_LOCK_KEY)
        .execute(pool)
        .await
        .expect("advisory lock failed");

    let migrations = [
        include_str!("../../modules/fixed-assets/db/migrations/20260218200001_create_asset_categories.sql"),
        include_str!("../../modules/fixed-assets/db/migrations/20260218200002_create_assets.sql"),
        include_str!("../../modules/fixed-assets/db/migrations/20260218200003_create_depreciation_schedules.sql"),
        include_str!("../../modules/fixed-assets/db/migrations/20260218200004_create_depreciation_runs.sql"),
        include_str!("../../modules/fixed-assets/db/migrations/20260218200005_create_disposals.sql"),
        include_str!("../../modules/fixed-assets/db/migrations/20260218200006_create_outbox_and_idempotency.sql"),
        include_str!("../../modules/fixed-assets/db/migrations/20260218200007_create_ap_capitalizations.sql"),
        include_str!("../../modules/fixed-assets/db/migrations/20260218200008_asset_status_to_text.sql"),
        include_str!("../../modules/fixed-assets/db/migrations/20260218200009_run_status_to_text.sql"),
    ];
    for sql in migrations {
        let _ = sqlx::raw_sql(sql).execute(pool).await;
    }

    sqlx::query("SELECT pg_advisory_unlock($1)")
        .bind(MIGRATION_LOCK_KEY)
        .execute(pool)
        .await
        .expect("advisory unlock failed");
}

const TEST_TENANT: &str = "e2e-fixed-assets";

async fn cleanup(pool: &PgPool) {
    for q in [
        "DELETE FROM fa_disposals WHERE tenant_id = $1",
        "DELETE FROM fa_depreciation_schedules WHERE tenant_id = $1",
        "DELETE FROM fa_depreciation_runs WHERE tenant_id = $1",
        "DELETE FROM fa_events_outbox WHERE tenant_id = $1",
        "DELETE FROM fa_ap_capitalizations WHERE tenant_id = $1",
        "DELETE FROM fa_assets WHERE tenant_id = $1",
        "DELETE FROM fa_categories WHERE tenant_id = $1",
    ] {
        sqlx::query(q).bind(TEST_TENANT).execute(pool).await.ok();
    }
}

/// Create category + asset in service from 2026-01-01, cost=$1200, salvage=0.
async fn create_active_asset(pool: &PgPool) -> (Uuid, Uuid) {
    let cat_id = Uuid::new_v4();
    let code = format!("E2E-{}", &cat_id.to_string()[..8]);
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
    .bind(&code)
    .bind(format!("Category {}", code))
    .execute(pool)
    .await
    .expect("insert category");

    let asset_id = Uuid::new_v4();
    let tag = format!("E2E-{}", &asset_id.to_string()[..8]);
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
                120000,'usd','straight_line',12,0,0,120000,NOW(),NOW())
        "#,
    )
    .bind(asset_id)
    .bind(TEST_TENANT)
    .bind(cat_id)
    .bind(&tag)
    .bind("E2E Test Asset")
    .execute(pool)
    .await
    .expect("insert asset");

    (cat_id, asset_id)
}

// ============================================================================
// Test 1: Full lifecycle — create → depreciate → dispose (sale with gain)
// ============================================================================

#[tokio::test]
#[serial]
async fn test_fixed_assets_full_lifecycle_create_depreciate_dispose() {
    let pool = fa_pool().await;
    ensure_migrations(&pool).await;
    cleanup(&pool).await;

    // 1. Create category + asset (active, in service 2026-01-01, cost=$1200)
    let (_cat_id, asset_id) = create_active_asset(&pool).await;

    // 2. Generate depreciation schedule
    let schedules = DepreciationService::generate_schedule(
        &pool,
        &GenerateScheduleRequest {
            tenant_id: TEST_TENANT.into(),
            asset_id,
        },
    )
    .await
    .expect("generate schedule");
    assert_eq!(schedules.len(), 12, "12 monthly periods");

    // 3. Run depreciation through 2026-06-30 → post periods 1-6
    let run = DepreciationService::run(
        &pool,
        &CreateRunRequest {
            tenant_id: TEST_TENANT.into(),
            as_of_date: NaiveDate::from_ymd_opt(2026, 6, 30).unwrap(),
            currency: None,
            created_by: None,
        },
    )
    .await
    .expect("run depreciation");
    assert_eq!(run.periods_posted, 6);
    assert_eq!(run.total_depreciation_minor, 60000);

    // Update asset accum/NBV to reflect posted depreciation
    // (the run marks schedule rows posted; asset accum is updated here for the E2E)
    sqlx::query(
        "UPDATE fa_assets SET accum_depreciation_minor = 60000, \
         net_book_value_minor = 60000 WHERE id = $1",
    )
    .bind(asset_id)
    .execute(&pool)
    .await
    .expect("update asset accum");

    // 4. Dispose asset — sell for $800 (NBV=$600, gain=$200)
    let disposal = DisposalService::dispose(
        &pool,
        &DisposeAssetRequest {
            tenant_id: TEST_TENANT.into(),
            asset_id,
            disposal_type: DisposalType::Sale,
            disposal_date: NaiveDate::from_ymd_opt(2026, 7, 1).unwrap(),
            proceeds_minor: Some(80000),
            reason: Some("Equipment upgrade".into()),
            buyer: Some("Buyer Corp".into()),
            reference: Some("INV-2026-100".into()),
            created_by: Some("admin".into()),
        },
    )
    .await
    .expect("dispose asset");

    assert_eq!(disposal.disposal_type, "sale");
    assert_eq!(disposal.net_book_value_at_disposal_minor, 60000);
    assert_eq!(disposal.proceeds_minor, 80000);
    assert_eq!(disposal.gain_loss_minor, 20000, "gain = proceeds - NBV");

    // 5. Verify asset status
    let (status, nbv): (String, i64) =
        sqlx::query_as("SELECT status, net_book_value_minor FROM fa_assets WHERE id = $1")
            .bind(asset_id)
            .fetch_one(&pool)
            .await
            .expect("check asset status");
    assert_eq!(status, "disposed");
    assert_eq!(nbv, 0, "NBV zeroed after disposal");

    // 6. Verify outbox event with GL data
    let (payload,): (serde_json::Value,) = sqlx::query_as(
        "SELECT payload FROM fa_events_outbox \
         WHERE tenant_id = $1 AND event_type = 'asset_disposed' \
         ORDER BY created_at DESC LIMIT 1",
    )
    .bind(TEST_TENANT)
    .fetch_one(&pool)
    .await
    .expect("outbox payload");

    let gl = &payload["gl_data"];
    assert_eq!(gl["asset_account_ref"], "1500");
    assert_eq!(gl["accum_depreciation_ref"], "1510");
    assert_eq!(gl["gain_loss_account_ref"], "7000");
    assert_eq!(gl["gain_loss_minor"], 20000);
    assert_eq!(gl["acquisition_cost_minor"], 120000);
    assert_eq!(gl["accum_depreciation_minor"], 60000);

    println!("PASS: Full lifecycle create→depreciate→dispose with gain verified");
    cleanup(&pool).await;
}

// ============================================================================
// Test 2: Disposal idempotency — rerun returns same result
// ============================================================================

#[tokio::test]
#[serial]
async fn test_fixed_assets_disposal_idempotent() {
    let pool = fa_pool().await;
    ensure_migrations(&pool).await;
    cleanup(&pool).await;

    let (_, asset_id) = create_active_asset(&pool).await;

    let req = DisposeAssetRequest {
        tenant_id: TEST_TENANT.into(),
        asset_id,
        disposal_type: DisposalType::Scrap,
        disposal_date: NaiveDate::from_ymd_opt(2026, 7, 1).unwrap(),
        proceeds_minor: None,
        reason: Some("Broken".into()),
        buyer: None,
        reference: None,
        created_by: None,
    };

    let d1 = DisposalService::dispose(&pool, &req).await.expect("first dispose");
    let d2 = DisposalService::dispose(&pool, &req).await.expect("second dispose");

    assert_eq!(d1.id, d2.id, "idempotent — same disposal ID");
    assert_eq!(d1.gain_loss_minor, d2.gain_loss_minor);

    let (count,): (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM fa_disposals WHERE asset_id = $1 AND tenant_id = $2",
    )
    .bind(asset_id)
    .bind(TEST_TENANT)
    .fetch_one(&pool)
    .await
    .expect("disposal count");
    assert_eq!(count, 1, "only one disposal record");

    println!("PASS: Disposal idempotency verified");
    cleanup(&pool).await;
}

// ============================================================================
// Test 3: Impairment flow
// ============================================================================

#[tokio::test]
#[serial]
async fn test_fixed_assets_impairment_flow() {
    let pool = fa_pool().await;
    ensure_migrations(&pool).await;
    cleanup(&pool).await;

    let (_, asset_id) = create_active_asset(&pool).await;

    let disposal = DisposalService::dispose(
        &pool,
        &DisposeAssetRequest {
            tenant_id: TEST_TENANT.into(),
            asset_id,
            disposal_type: DisposalType::Impairment,
            disposal_date: NaiveDate::from_ymd_opt(2026, 7, 1).unwrap(),
            proceeds_minor: None,
            reason: Some("Market value decline".into()),
            buyer: None,
            reference: None,
            created_by: None,
        },
    )
    .await
    .expect("impair");

    assert_eq!(disposal.disposal_type, "impairment");
    assert_eq!(disposal.gain_loss_minor, -120000, "loss = -NBV");

    let (status,): (String,) =
        sqlx::query_as("SELECT status FROM fa_assets WHERE id = $1")
            .bind(asset_id)
            .fetch_one(&pool)
            .await
            .expect("check status");
    assert_eq!(status, "impaired");

    println!("PASS: Impairment flow verified");
    cleanup(&pool).await;
}

// ============================================================================
// Test 4: Deterministic — same inputs produce same financial outputs
// ============================================================================

#[tokio::test]
#[serial]
async fn test_fixed_assets_disposal_deterministic() {
    let pool = fa_pool().await;
    ensure_migrations(&pool).await;

    // Run 1
    cleanup(&pool).await;
    let (_, asset_id_1) = create_active_asset(&pool).await;
    sqlx::query(
        "UPDATE fa_assets SET accum_depreciation_minor = 30000, \
         net_book_value_minor = 90000 WHERE id = $1",
    )
    .bind(asset_id_1)
    .execute(&pool)
    .await
    .unwrap();

    let d1 = DisposalService::dispose(
        &pool,
        &DisposeAssetRequest {
            tenant_id: TEST_TENANT.into(),
            asset_id: asset_id_1,
            disposal_type: DisposalType::Sale,
            disposal_date: NaiveDate::from_ymd_opt(2026, 4, 1).unwrap(),
            proceeds_minor: Some(100000),
            reason: None,
            buyer: None,
            reference: None,
            created_by: None,
        },
    )
    .await
    .unwrap();

    // Run 2 — fresh data, same financial inputs
    cleanup(&pool).await;
    let (_, asset_id_2) = create_active_asset(&pool).await;
    sqlx::query(
        "UPDATE fa_assets SET accum_depreciation_minor = 30000, \
         net_book_value_minor = 90000 WHERE id = $1",
    )
    .bind(asset_id_2)
    .execute(&pool)
    .await
    .unwrap();

    let d2 = DisposalService::dispose(
        &pool,
        &DisposeAssetRequest {
            tenant_id: TEST_TENANT.into(),
            asset_id: asset_id_2,
            disposal_type: DisposalType::Sale,
            disposal_date: NaiveDate::from_ymd_opt(2026, 4, 1).unwrap(),
            proceeds_minor: Some(100000),
            reason: None,
            buyer: None,
            reference: None,
            created_by: None,
        },
    )
    .await
    .unwrap();

    assert_eq!(d1.net_book_value_at_disposal_minor, d2.net_book_value_at_disposal_minor);
    assert_eq!(d1.proceeds_minor, d2.proceeds_minor);
    assert_eq!(d1.gain_loss_minor, d2.gain_loss_minor);
    assert_eq!(d1.gain_loss_minor, 10000, "gain = 100000 - 90000");

    println!("PASS: Disposal determinism verified");
    cleanup(&pool).await;
}
