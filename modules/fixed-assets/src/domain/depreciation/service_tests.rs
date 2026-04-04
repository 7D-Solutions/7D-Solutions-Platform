//! Integrated tests for DepreciationService — require a running fixed-assets Postgres instance.

use chrono::NaiveDate;
use serial_test::serial;
use sqlx::PgPool;
use uuid::Uuid;

use super::models::*;
use super::service::DepreciationService;

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

const TEST_TENANT: &str = "test-depr-svc";

async fn cleanup(pool: &PgPool) {
    sqlx::query("DELETE FROM fa_depreciation_schedules WHERE tenant_id = $1")
        .bind(TEST_TENANT)
        .execute(pool)
        .await
        .ok();
    sqlx::query("DELETE FROM fa_depreciation_runs WHERE tenant_id = $1")
        .bind(TEST_TENANT)
        .execute(pool)
        .await
        .ok();
    sqlx::query("DELETE FROM fa_events_outbox WHERE tenant_id = $1")
        .bind(TEST_TENANT)
        .execute(pool)
        .await
        .ok();
    sqlx::query("DELETE FROM fa_assets WHERE tenant_id = $1")
        .bind(TEST_TENANT)
        .execute(pool)
        .await
        .ok();
    sqlx::query("DELETE FROM fa_categories WHERE tenant_id = $1")
        .bind(TEST_TENANT)
        .execute(pool)
        .await
        .ok();
}

/// Insert category + asset directly (avoiding RETURNING * with the fa_asset_status ENUM).
async fn create_test_asset(pool: &PgPool, in_service_date: NaiveDate) -> Uuid {
    let cat_id = Uuid::new_v4();
    let tag = format!("CAT-{}", &cat_id.to_string()[..8]);
    sqlx::query(
        r#"
        INSERT INTO fa_categories
            (id, tenant_id, code, name,
             default_method, default_useful_life_months, default_salvage_pct_bp,
             asset_account_ref, depreciation_expense_ref, accum_depreciation_ref,
             is_active, created_at, updated_at)
        VALUES ($1,$2,$3,$4,'straight_line',12,0,'1500','6100','1510',TRUE,NOW(),NOW())
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
    let asset_tag = format!("FA-{}", &asset_id.to_string()[..8]);
    sqlx::query(
        r#"
        INSERT INTO fa_assets
            (id, tenant_id, category_id, asset_tag, name,
             acquisition_date, in_service_date,
             acquisition_cost_minor, currency,
             depreciation_method, useful_life_months, salvage_value_minor,
             accum_depreciation_minor, net_book_value_minor,
             created_at, updated_at)
        VALUES ($1,$2,$3,$4,$5,$6,$6,120000,'usd','straight_line',12,0,0,120000,NOW(),NOW())
        "#,
    )
    .bind(asset_id)
    .bind(TEST_TENANT)
    .bind(cat_id)
    .bind(asset_tag)
    .bind("Test Asset")
    .bind(in_service_date)
    .execute(pool)
    .await
    .expect("insert test asset");

    asset_id
}

#[tokio::test]
#[serial]
async fn generate_schedule_creates_12_periods() {
    let pool = test_pool().await;
    cleanup(&pool).await;

    let in_service = NaiveDate::from_ymd_opt(2026, 1, 1).expect("valid test date");
    let asset_id = create_test_asset(&pool, in_service).await;

    let req = GenerateScheduleRequest {
        tenant_id: TEST_TENANT.into(),
        asset_id,
    };
    let schedules = DepreciationService::generate_schedule(&pool, &req)
        .await
        .expect("generate_schedule failed");

    assert_eq!(schedules.len(), 12);
    let total: i64 = schedules.iter().map(|s| s.depreciation_amount_minor).sum();
    assert_eq!(total, 120_000);
    assert_eq!(schedules[0].period_number, 1);
    assert_eq!(schedules[11].period_number, 12);
    assert_eq!(schedules[11].remaining_book_value_minor, 0);
}

#[tokio::test]
#[serial]
async fn generate_schedule_is_idempotent() {
    let pool = test_pool().await;
    cleanup(&pool).await;

    let in_service = NaiveDate::from_ymd_opt(2026, 1, 1).expect("valid test date");
    let asset_id = create_test_asset(&pool, in_service).await;

    let req = GenerateScheduleRequest {
        tenant_id: TEST_TENANT.into(),
        asset_id,
    };
    let first = DepreciationService::generate_schedule(&pool, &req)
        .await
        .expect("first generate_schedule failed");
    let second = DepreciationService::generate_schedule(&pool, &req)
        .await
        .expect("second generate_schedule failed");

    assert_eq!(first.len(), second.len(), "no duplicate rows on rerun");
    for (a, b) in first.iter().zip(second.iter()) {
        assert_eq!(a.id, b.id, "same row ids — no new inserts");
    }
}

#[tokio::test]
#[serial]
async fn run_posts_periods_and_is_idempotent() {
    let pool = test_pool().await;
    cleanup(&pool).await;

    let in_service = NaiveDate::from_ymd_opt(2026, 1, 1).expect("valid test date");
    let asset_id = create_test_asset(&pool, in_service).await;

    // Generate schedule first
    let sched_req = GenerateScheduleRequest {
        tenant_id: TEST_TENANT.into(),
        asset_id,
    };
    DepreciationService::generate_schedule(&pool, &sched_req)
        .await
        .expect("generate_schedule failed");

    // Run through 2026-06-30 → should post periods 1-6
    let run_req = CreateRunRequest {
        tenant_id: TEST_TENANT.into(),
        as_of_date: NaiveDate::from_ymd_opt(2026, 6, 30).expect("valid test date"),
        currency: None,
        created_by: None,
    };
    let run1 = DepreciationService::run(&pool, &run_req)
        .await
        .expect("run1 failed");
    assert_eq!(run1.status, "completed");
    assert_eq!(run1.periods_posted, 6);
    assert_eq!(run1.assets_processed, 1);
    assert_eq!(run1.total_depreciation_minor, 60_000);

    // Rerun same as_of_date → no new periods posted
    let run2 = DepreciationService::run(&pool, &run_req)
        .await
        .expect("run2 failed");
    assert_eq!(run2.status, "completed");
    assert_eq!(run2.periods_posted, 0, "already posted — idempotent");
    assert_eq!(run2.total_depreciation_minor, 0);
}

#[tokio::test]
#[serial]
async fn run_respects_as_of_date_boundary() {
    let pool = test_pool().await;
    cleanup(&pool).await;

    let in_service = NaiveDate::from_ymd_opt(2026, 1, 1).expect("valid test date");
    let asset_id = create_test_asset(&pool, in_service).await;

    DepreciationService::generate_schedule(
        &pool,
        &GenerateScheduleRequest {
            tenant_id: TEST_TENANT.into(),
            asset_id,
        },
    )
    .await
    .expect("generate_schedule failed");

    // Only one period ending 2026-01-31 should be posted
    let run = DepreciationService::run(
        &pool,
        &CreateRunRequest {
            tenant_id: TEST_TENANT.into(),
            as_of_date: NaiveDate::from_ymd_opt(2026, 1, 31).expect("valid test date"),
            currency: None,
            created_by: None,
        },
    )
    .await
    .expect("run failed");

    assert_eq!(run.periods_posted, 1);
    assert_eq!(run.total_depreciation_minor, 10_000); // 120_000 / 12
}
