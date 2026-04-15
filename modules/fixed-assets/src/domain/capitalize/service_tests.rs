//! Integrated Tests for capitalize_from_ap_line (real DB, no mocks).

use chrono::NaiveDate;
use serial_test::serial;
use sqlx::PgPool;
use uuid::Uuid;

use super::service::{capitalize_from_ap_line, CapitalizeFromApLineRequest};

const TEST_TENANT: &str = "test-tenant-capitalize";

fn test_db_url() -> String {
    std::env::var("DATABASE_URL").unwrap_or_else(|_| {
        "postgres://fixed_assets_user:fixed_assets_pass@localhost:5445/fixed_assets_db?sslmode=require"
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
    sqlx::query("DELETE FROM fa_ap_capitalizations WHERE tenant_id = $1")
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

fn sample_req(bill_id: Uuid, line_id: Uuid, gl_account_code: &str) -> CapitalizeFromApLineRequest {
    CapitalizeFromApLineRequest {
        tenant_id: TEST_TENANT.to_string(),
        bill_id,
        line_id,
        gl_account_code: gl_account_code.to_string(),
        amount_minor: 100_000,
        currency: "USD".to_string(),
        acquisition_date: NaiveDate::from_ymd_opt(2026, 2, 18).expect("valid test date"),
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
    let (count,): (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM fa_assets WHERE id = $1 AND tenant_id = $2")
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
    assert!(
        second.is_none(),
        "second call must be idempotent (return None)"
    );

    // Only one asset should exist
    let (count,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM fa_assets WHERE tenant_id = $1")
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
         WHERE bill_id = $1 AND line_id = $2 AND tenant_id = $3",
    )
    .bind(bill_id)
    .bind(line_id)
    .bind(TEST_TENANT)
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
