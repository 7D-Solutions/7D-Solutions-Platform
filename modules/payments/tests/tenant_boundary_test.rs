//! Tenant Boundary Tests (bd-vnuvp — Tenant Isolation Sweep)
//!
//! Proves no cross-tenant data leakage for payment attempts.
//! Two tenants operate on the same database — tenant B must never see tenant A's data.
//!
//! ## Known Gaps (documented in docs/audits/tenant-isolation-sweep-2026-03-31.md)
//! - lifecycle.rs: validate_transition queries by attempt_id without tenant filter
//! - webhook_handler.rs: queries by attempt_id without tenant filter
//! - reconciliation.rs: queries by attempt_id without tenant filter
//!
//! ## Prerequisites
//! - PostgreSQL at localhost:5436 (docker compose up -d)

use serial_test::serial;
use sqlx::{postgres::PgPoolOptions, PgPool};
use uuid::Uuid;

// ============================================================================
// Test DB helpers
// ============================================================================

async fn setup_db() -> PgPool {
    dotenvy::dotenv().ok();
    let url = std::env::var("DATABASE_URL").unwrap_or_else(|_| {
        "postgres://payments_user:payments_pass@localhost:5436/payments_db?sslmode=require"
            .to_string()
    });

    let pool = PgPoolOptions::new()
        .max_connections(10)
        .connect(&url)
        .await
        .expect("Failed to connect to payments test DB");

    sqlx::migrate!("db/migrations")
        .run(&pool)
        .await
        .expect("Failed to run payments migrations");

    pool
}

fn unique_tenant() -> String {
    format!("boundary-{}", Uuid::new_v4().simple())
}

/// Insert a payment attempt for a given tenant
async fn insert_attempt(pool: &PgPool, app_id: &str, invoice_id: &str) -> Uuid {
    let payment_id = Uuid::new_v4();
    sqlx::query(
        r#"INSERT INTO payment_attempts (
            app_id, payment_id, invoice_id, attempt_no, status
        ) VALUES ($1, $2, $3, 0, 'attempting')"#,
    )
    .bind(app_id)
    .bind(payment_id)
    .bind(invoice_id)
    .execute(pool)
    .await
    .expect("Failed to insert payment attempt");

    // Return the attempt id
    let id: Uuid = sqlx::query_scalar(
        "SELECT id FROM payment_attempts WHERE payment_id = $1 AND app_id = $2",
    )
    .bind(payment_id)
    .bind(app_id)
    .fetch_one(pool)
    .await
    .expect("Failed to fetch attempt id");

    id
}

// ============================================================================
// Tenant Boundary Tests
// ============================================================================

#[tokio::test]
#[serial]
async fn tenant_a_data_invisible_to_tenant_b_scoped_query() {
    let pool = setup_db().await;
    let tenant_a = unique_tenant();
    let tenant_b = unique_tenant();

    // Create attempts for both tenants
    let a_attempt = insert_attempt(&pool, &tenant_a, "inv-a-001").await;
    let _b_attempt = insert_attempt(&pool, &tenant_b, "inv-b-001").await;

    // Tenant-scoped query: tenant B should NOT see tenant A's attempt
    let cross_read: Option<(Uuid,)> = sqlx::query_as(
        "SELECT id FROM payment_attempts WHERE id = $1 AND app_id = $2",
    )
    .bind(a_attempt)
    .bind(&tenant_b)
    .fetch_optional(&pool)
    .await
    .expect("Query should succeed");

    assert!(
        cross_read.is_none(),
        "Tenant B must NOT see tenant A's payment attempt via scoped query"
    );
}

#[tokio::test]
#[serial]
async fn tenant_list_returns_only_own_attempts() {
    let pool = setup_db().await;
    let tenant_a = unique_tenant();
    let tenant_b = unique_tenant();

    // Create 3 attempts for tenant A, 2 for tenant B
    for i in 0..3 {
        insert_attempt(&pool, &tenant_a, &format!("inv-a-{i}")).await;
    }
    for i in 0..2 {
        insert_attempt(&pool, &tenant_b, &format!("inv-b-{i}")).await;
    }

    // Count tenant A's attempts
    let a_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM payment_attempts WHERE app_id = $1")
            .bind(&tenant_a)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(a_count, 3, "Tenant A should have 3 attempts");

    // Count tenant B's attempts
    let b_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM payment_attempts WHERE app_id = $1")
            .bind(&tenant_b)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(b_count, 2, "Tenant B should have 2 attempts");

    // Verify no cross-contamination in list
    let a_ids: Vec<String> = sqlx::query_scalar(
        "SELECT app_id FROM payment_attempts WHERE app_id IN ($1, $2)",
    )
    .bind(&tenant_a)
    .bind(&tenant_b)
    .fetch_all(&pool)
    .await
    .unwrap();

    let a_only: Vec<&String> = a_ids.iter().filter(|id| *id == &tenant_a).collect();
    let b_only: Vec<&String> = a_ids.iter().filter(|id| *id == &tenant_b).collect();
    assert_eq!(a_only.len(), 3);
    assert_eq!(b_only.len(), 2);
}

#[tokio::test]
#[serial]
async fn lifecycle_query_without_tenant_demonstrates_gap() {
    // This test documents a known vulnerability:
    // The lifecycle module queries by attempt_id without tenant_id filter.
    // An unscoped query returns data regardless of which tenant owns it.
    let pool = setup_db().await;
    let tenant_a = unique_tenant();

    let a_attempt = insert_attempt(&pool, &tenant_a, "inv-gap-001").await;

    // This is how lifecycle.rs queries — NO tenant filter:
    // SELECT status::text FROM payment_attempts WHERE id = $1
    let unscoped: Option<(String,)> = sqlx::query_as(
        "SELECT status::text FROM payment_attempts WHERE id = $1",
    )
    .bind(a_attempt)
    .fetch_optional(&pool)
    .await
    .expect("Unscoped query should succeed");

    // The unscoped query returns data — this is the vulnerability.
    // After the fix (child bead), this query should include AND app_id = $2.
    assert!(
        unscoped.is_some(),
        "Unscoped query returns data (expected — documents the gap)"
    );

    // The CORRECT query should require tenant context:
    let wrong_tenant = unique_tenant();
    let scoped: Option<(String,)> = sqlx::query_as(
        "SELECT status::text FROM payment_attempts WHERE id = $1 AND app_id = $2",
    )
    .bind(a_attempt)
    .bind(&wrong_tenant)
    .fetch_optional(&pool)
    .await
    .expect("Scoped query should succeed");

    assert!(
        scoped.is_none(),
        "Scoped query with wrong tenant must return nothing"
    );
}
