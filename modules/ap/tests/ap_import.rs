//! Integration tests for POST /api/ap/import/vendors.
//!
//! Connects to a real PostgreSQL database.  No mocks, no stubs.
//! Requires DATABASE_URL env var (or falls back to the default AP dev URL).
//!
//! Run with:
//!   ./scripts/cargo-slot.sh test -p ap --test ap_import -- --nocapture

use ap::http::imports::{run_vendors_import, VendorImportRow};
use serial_test::serial;
use sqlx::{postgres::PgPoolOptions, PgPool};
use uuid::Uuid;

async fn setup_pool() -> PgPool {
    dotenvy::dotenv().ok();
    let url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://ap_user:ap_pass@localhost:5443/ap_db".to_string());
    PgPoolOptions::new()
        .max_connections(5)
        .connect(&url)
        .await
        .expect("Failed to connect to AP DB")
}

fn unique_tenant() -> String {
    format!("import-ap-{}", Uuid::new_v4().simple())
}

async fn cleanup(pool: &PgPool, tenant_id: &str) {
    sqlx::query("DELETE FROM vendors WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(pool)
        .await
        .ok();
}

fn vendor(name: &str, terms: &str, currency: &str) -> VendorImportRow {
    VendorImportRow {
        vendor_code: None,
        name: name.into(),
        payment_terms: Some(terms.into()),
        currency: Some(currency.into()),
    }
}

// ============================================================================
// Tests
// ============================================================================

#[tokio::test]
#[serial]
async fn ap_import_creates_new_vendors() {
    let pool = setup_pool().await;
    let tenant = unique_tenant();
    cleanup(&pool, &tenant).await;

    let rows = vec![
        vendor("Acme Corp", "30", "USD"),
        vendor("Beta Supplies", "45", "EUR"),
    ];

    let summary = run_vendors_import(&pool, &tenant, &rows)
        .await
        .expect("import should succeed");

    assert_eq!(summary.created, 2);
    assert_eq!(summary.updated, 0);
    assert_eq!(summary.skipped, 0);
    assert!(summary.errors.is_empty());

    let count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM vendors WHERE tenant_id = $1")
            .bind(&tenant)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(count, 2);

    cleanup(&pool, &tenant).await;
}

#[tokio::test]
#[serial]
async fn ap_import_skips_identical_on_reimport() {
    let pool = setup_pool().await;
    let tenant = unique_tenant();
    cleanup(&pool, &tenant).await;

    let rows = vec![vendor("Acme Corp", "30", "USD")];

    let s1 = run_vendors_import(&pool, &tenant, &rows).await.unwrap();
    assert_eq!(s1.created, 1);

    let s2 = run_vendors_import(&pool, &tenant, &rows).await.unwrap();
    assert_eq!(s2.created, 0);
    assert_eq!(s2.updated, 0);
    assert_eq!(s2.skipped, 1);
    assert!(s2.errors.is_empty());

    cleanup(&pool, &tenant).await;
}

#[tokio::test]
#[serial]
async fn ap_import_updates_changed_terms() {
    let pool = setup_pool().await;
    let tenant = unique_tenant();
    cleanup(&pool, &tenant).await;

    run_vendors_import(&pool, &tenant, &[vendor("Acme Corp", "30", "USD")])
        .await
        .unwrap();

    let s2 = run_vendors_import(&pool, &tenant, &[vendor("Acme Corp", "45", "USD")])
        .await
        .unwrap();

    assert_eq!(s2.updated, 1);
    assert_eq!(s2.skipped, 0);
    assert!(s2.errors.is_empty());

    let terms: i32 = sqlx::query_scalar(
        "SELECT payment_terms_days FROM vendors WHERE tenant_id = $1 AND name = 'Acme Corp'",
    )
    .bind(&tenant)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(terms, 45);

    cleanup(&pool, &tenant).await;
}

#[tokio::test]
#[serial]
async fn ap_import_validates_all_before_insert() {
    let pool = setup_pool().await;
    let tenant = unique_tenant();
    cleanup(&pool, &tenant).await;

    let rows = vec![
        vendor("Good Vendor", "30", "USD"),
        VendorImportRow {
            vendor_code: None,
            name: "Bad Currency".into(),
            payment_terms: Some("30".into()),
            // 4-char currency is invalid
            currency: Some("USDX".into()),
        },
    ];

    let summary = run_vendors_import(&pool, &tenant, &rows).await.unwrap();

    assert_eq!(summary.errors.len(), 1);
    assert_eq!(summary.errors[0].row, 2);
    assert_eq!(summary.created, 0, "No rows should be inserted when any row fails validation");

    let count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM vendors WHERE tenant_id = $1")
            .bind(&tenant)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(count, 0);

    cleanup(&pool, &tenant).await;
}

#[tokio::test]
#[serial]
async fn ap_import_defaults_currency_to_usd() {
    let pool = setup_pool().await;
    let tenant = unique_tenant();
    cleanup(&pool, &tenant).await;

    let rows = vec![VendorImportRow {
        vendor_code: None,
        name: "No Currency Vendor".into(),
        payment_terms: None,
        currency: None,
    }];

    let summary = run_vendors_import(&pool, &tenant, &rows).await.unwrap();
    assert_eq!(summary.created, 1);
    assert!(summary.errors.is_empty());

    let currency: String = sqlx::query_scalar(
        "SELECT currency FROM vendors WHERE tenant_id = $1 AND name = 'No Currency Vendor'",
    )
    .bind(&tenant)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(currency, "USD");

    cleanup(&pool, &tenant).await;
}
