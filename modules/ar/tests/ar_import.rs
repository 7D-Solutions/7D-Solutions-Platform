//! Integration tests for POST /api/ar/import/customers.
//!
//! Connects to a real PostgreSQL database.  No mocks, no stubs.
//! Requires DATABASE_URL env var (or falls back to the default AR dev URL).
//!
//! Run with:
//!   ./scripts/cargo-slot.sh test -p ar-rs --test ar_import -- --nocapture

use ar_rs::http::imports::{run_customers_import, CustomerImportRow};
use serial_test::serial;
use sqlx::{postgres::PgPoolOptions, PgPool};
use uuid::Uuid;

async fn setup_pool() -> PgPool {
    dotenvy::dotenv().ok();
    let url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://ar_user:ar_pass@localhost:5441/ar_db".to_string());
    PgPoolOptions::new()
        .max_connections(5)
        .connect(&url)
        .await
        .expect("Failed to connect to AR DB")
}

fn unique_tenant() -> String {
    format!("import-ar-{}", Uuid::new_v4().simple())
}

async fn cleanup(pool: &PgPool, app_id: &str) {
    sqlx::query("DELETE FROM ar_customers WHERE app_id = $1")
        .bind(app_id)
        .execute(pool)
        .await
        .ok();
}

fn customer(code: &str, name: &str) -> CustomerImportRow {
    CustomerImportRow {
        customer_code: code.into(),
        name: Some(name.into()),
        email: None,
        payment_terms: Some("30".into()),
        currency: Some("USD".into()),
        credit_limit: None,
    }
}

// ============================================================================
// Tests
// ============================================================================

#[tokio::test]
#[serial]
async fn ar_import_creates_new_customers() {
    let pool = setup_pool().await;
    let app_id = unique_tenant();
    cleanup(&pool, &app_id).await;

    let rows = vec![customer("CUST-001", "Alpha Corp"), customer("CUST-002", "Beta Inc")];

    let summary = run_customers_import(&pool, &app_id, &rows)
        .await
        .expect("import should succeed");

    assert_eq!(summary.created, 2);
    assert_eq!(summary.updated, 0);
    assert_eq!(summary.skipped, 0);
    assert!(summary.errors.is_empty());

    let count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM ar_customers WHERE app_id = $1")
            .bind(&app_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(count, 2);

    cleanup(&pool, &app_id).await;
}

#[tokio::test]
#[serial]
async fn ar_import_skips_identical_on_reimport() {
    let pool = setup_pool().await;
    let app_id = unique_tenant();
    cleanup(&pool, &app_id).await;

    let rows = vec![customer("CUST-001", "Alpha Corp")];

    let s1 = run_customers_import(&pool, &app_id, &rows).await.unwrap();
    assert_eq!(s1.created, 1);

    let s2 = run_customers_import(&pool, &app_id, &rows).await.unwrap();
    assert_eq!(s2.created, 0);
    assert_eq!(s2.updated, 0);
    assert_eq!(s2.skipped, 1);
    assert!(s2.errors.is_empty());

    cleanup(&pool, &app_id).await;
}

#[tokio::test]
#[serial]
async fn ar_import_updates_changed_name() {
    let pool = setup_pool().await;
    let app_id = unique_tenant();
    cleanup(&pool, &app_id).await;

    run_customers_import(&pool, &app_id, &[customer("CUST-001", "Old Name")])
        .await
        .unwrap();

    let s2 = run_customers_import(&pool, &app_id, &[customer("CUST-001", "New Name")])
        .await
        .unwrap();

    assert_eq!(s2.updated, 1);
    assert_eq!(s2.skipped, 0);
    assert!(s2.errors.is_empty());

    let name: Option<String> = sqlx::query_scalar(
        "SELECT name FROM ar_customers WHERE app_id = $1 AND external_customer_id = 'CUST-001'",
    )
    .bind(&app_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(name.as_deref(), Some("New Name"));

    cleanup(&pool, &app_id).await;
}

#[tokio::test]
#[serial]
async fn ar_import_validates_all_before_insert() {
    let pool = setup_pool().await;
    let app_id = unique_tenant();
    cleanup(&pool, &app_id).await;

    let rows = vec![
        customer("CUST-001", "Good Customer"),
        // Empty customer_code is invalid
        CustomerImportRow {
            customer_code: "".into(),
            name: Some("Bad Customer".into()),
            email: None,
            payment_terms: None,
            currency: None,
            credit_limit: None,
        },
    ];

    let summary = run_customers_import(&pool, &app_id, &rows).await.unwrap();

    assert_eq!(summary.errors.len(), 1);
    assert_eq!(summary.errors[0].row, 2);
    assert_eq!(summary.created, 0, "No rows should be inserted when any row fails validation");

    let count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM ar_customers WHERE app_id = $1")
            .bind(&app_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(count, 0);

    cleanup(&pool, &app_id).await;
}

#[tokio::test]
#[serial]
async fn ar_import_synthesises_email_when_missing() {
    let pool = setup_pool().await;
    let app_id = unique_tenant();
    cleanup(&pool, &app_id).await;

    let rows = vec![CustomerImportRow {
        customer_code: "CUST-X".into(),
        name: None,
        email: None,
        payment_terms: None,
        currency: None,
        credit_limit: None,
    }];

    let summary = run_customers_import(&pool, &app_id, &rows).await.unwrap();
    assert_eq!(summary.created, 1);
    assert!(summary.errors.is_empty());

    let email: String = sqlx::query_scalar(
        "SELECT email FROM ar_customers WHERE app_id = $1 AND external_customer_id = 'CUST-X'",
    )
    .bind(&app_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(
        email.ends_with("@import.placeholder"),
        "Expected synthesised email, got: {}",
        email
    );

    cleanup(&pool, &app_id).await;
}
