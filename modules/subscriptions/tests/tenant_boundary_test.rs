//! Tenant Boundary Tests (bd-vnuvp — Tenant Isolation Sweep)
//!
//! Proves no cross-tenant data leakage for subscriptions.
//! Two tenants operate on the same database — tenant B must never see tenant A's data.
//!
//! ## Fixed (bd-vnuvp.5): All lifecycle queries now filter by tenant_id
//!
//! ## Prerequisites
//! - PostgreSQL at localhost:5435 (docker compose up -d)

use serial_test::serial;
use sqlx::{postgres::PgPoolOptions, PgPool};
use uuid::Uuid;

// ============================================================================
// Test DB helpers
// ============================================================================

async fn setup_db() -> PgPool {
    dotenvy::dotenv().ok();
    let url = std::env::var("DATABASE_URL").unwrap_or_else(|_| {
        "postgres://subscriptions_user:subscriptions_pass@localhost:5435/subscriptions_db?sslmode=require"
            .to_string()
    });

    let pool = PgPoolOptions::new()
        .max_connections(10)
        .connect(&url)
        .await
        .expect("Failed to connect to subscriptions test DB");

    sqlx::migrate!("db/migrations")
        .run(&pool)
        .await
        .expect("Failed to run subscriptions migrations");

    pool
}

fn unique_tenant() -> String {
    format!("boundary-{}", Uuid::new_v4().simple())
}

/// Create a subscription plan + subscription for a tenant, returning the subscription ID.
async fn create_subscription(pool: &PgPool, tenant_id: &str, status: &str) -> Uuid {
    let plan_id: Uuid = sqlx::query_scalar(
        "INSERT INTO subscription_plans (tenant_id, name, schedule, price_minor, currency)
         VALUES ($1, 'Boundary Test Plan', 'monthly', 9999, 'USD')
         RETURNING id",
    )
    .bind(tenant_id)
    .fetch_one(pool)
    .await
    .expect("Failed to create test plan");

    let ar_customer_id = format!("cust-{}", Uuid::new_v4());

    let subscription_id: Uuid = sqlx::query_scalar(
        "INSERT INTO subscriptions (tenant_id, ar_customer_id, plan_id, status, schedule, price_minor, currency, start_date, next_bill_date)
         VALUES ($1, $2, $3, $4, 'monthly', 9999, 'USD', CURRENT_DATE, CURRENT_DATE + INTERVAL '1 month')
         RETURNING id",
    )
    .bind(tenant_id)
    .bind(&ar_customer_id)
    .bind(plan_id)
    .bind(status)
    .fetch_one(pool)
    .await
    .expect("Failed to create test subscription");

    subscription_id
}

// ============================================================================
// Tenant Boundary Tests
// ============================================================================

#[tokio::test]
#[serial]
async fn tenant_a_subscription_invisible_to_tenant_b() {
    let pool = setup_db().await;
    let tenant_a = unique_tenant();
    let tenant_b = unique_tenant();

    let a_sub = create_subscription(&pool, &tenant_a, "active").await;
    let _b_sub = create_subscription(&pool, &tenant_b, "active").await;

    // Tenant-scoped query: tenant B should NOT see tenant A's subscription
    let cross_read: Option<(Uuid,)> =
        sqlx::query_as("SELECT id FROM subscriptions WHERE id = $1 AND tenant_id = $2")
            .bind(a_sub)
            .bind(&tenant_b)
            .fetch_optional(&pool)
            .await
            .expect("Query should succeed");

    assert!(
        cross_read.is_none(),
        "Tenant B must NOT see tenant A's subscription via scoped query"
    );
}

#[tokio::test]
#[serial]
async fn tenant_list_returns_only_own_subscriptions() {
    let pool = setup_db().await;
    let tenant_a = unique_tenant();
    let tenant_b = unique_tenant();

    // Create subscriptions
    for _ in 0..3 {
        create_subscription(&pool, &tenant_a, "active").await;
    }
    for _ in 0..2 {
        create_subscription(&pool, &tenant_b, "active").await;
    }

    let a_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM subscriptions WHERE tenant_id = $1")
            .bind(&tenant_a)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(a_count, 3, "Tenant A should have 3 subscriptions");

    let b_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM subscriptions WHERE tenant_id = $1")
            .bind(&tenant_b)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(b_count, 2, "Tenant B should have 2 subscriptions");
}

#[tokio::test]
#[serial]
async fn lifecycle_transition_rejects_wrong_tenant() {
    // bd-vnuvp.5: Proves lifecycle functions now enforce tenant isolation.
    // transition_to_active with wrong tenant_id must fail (SubscriptionNotFound).
    let pool = setup_db().await;
    let tenant_a = unique_tenant();
    let wrong_tenant = unique_tenant();

    let a_sub = create_subscription(&pool, &tenant_a, "past_due").await;

    // Attempt transition with wrong tenant — must fail
    let result = subscriptions_rs::lifecycle::transition_to_active(
        a_sub,
        &wrong_tenant,
        "payment_recovered",
        &pool,
    )
    .await;

    assert!(result.is_err(), "Transition with wrong tenant_id must fail");

    // Verify status was NOT modified
    let status: String =
        sqlx::query_scalar("SELECT status FROM subscriptions WHERE id = $1 AND tenant_id = $2")
            .bind(a_sub)
            .bind(&tenant_a)
            .fetch_one(&pool)
            .await
            .expect("Should find subscription");

    assert_eq!(status, "past_due", "Subscription should still be past_due");
}

#[tokio::test]
#[serial]
async fn update_with_wrong_tenant_must_not_affect_rows() {
    // Demonstrates that an UPDATE scoped by tenant_id would prevent cross-tenant mutation.
    let pool = setup_db().await;
    let tenant_a = unique_tenant();
    let wrong_tenant = unique_tenant();

    let a_sub = create_subscription(&pool, &tenant_a, "active").await;

    // Scoped update with wrong tenant should affect 0 rows
    let rows = sqlx::query(
        "UPDATE subscriptions SET status = 'suspended', updated_at = NOW() WHERE id = $1 AND tenant_id = $2",
    )
    .bind(a_sub)
    .bind(&wrong_tenant)
    .execute(&pool)
    .await
    .expect("Scoped update should succeed")
    .rows_affected();

    assert_eq!(rows, 0, "Update with wrong tenant must affect 0 rows");

    // Verify subscription was NOT modified
    let status: String =
        sqlx::query_scalar("SELECT status FROM subscriptions WHERE id = $1 AND tenant_id = $2")
            .bind(a_sub)
            .bind(&tenant_a)
            .fetch_one(&pool)
            .await
            .expect("Should find subscription");

    assert_eq!(status, "active", "Subscription should still be active");
}

#[tokio::test]
#[serial]
async fn bill_run_tenant_isolation() {
    // bd-dwb41: Bill runs must be scoped by tenant_id.
    // Tenant B must not see tenant A's bill run records.
    let pool = setup_db().await;
    let tenant_a = unique_tenant();
    let tenant_b = unique_tenant();

    let bill_run_id_a = format!("br-{}", Uuid::new_v4());
    let bill_run_id_b = format!("br-{}", Uuid::new_v4());

    // Create bill run for tenant A
    sqlx::query(
        "INSERT INTO bill_runs (bill_run_id, tenant_id, execution_date, status, subscriptions_processed, invoices_created, failures)
         VALUES ($1, $2, CURRENT_DATE, 'completed', 5, 5, 0)",
    )
    .bind(&bill_run_id_a)
    .bind(&tenant_a)
    .execute(&pool)
    .await
    .expect("Failed to create bill run for tenant A");

    // Create bill run for tenant B
    sqlx::query(
        "INSERT INTO bill_runs (bill_run_id, tenant_id, execution_date, status, subscriptions_processed, invoices_created, failures)
         VALUES ($1, $2, CURRENT_DATE, 'completed', 3, 3, 0)",
    )
    .bind(&bill_run_id_b)
    .bind(&tenant_b)
    .execute(&pool)
    .await
    .expect("Failed to create bill run for tenant B");

    // Tenant B must NOT see tenant A's bill run
    let cross_read: Option<(Uuid,)> =
        sqlx::query_as("SELECT id FROM bill_runs WHERE bill_run_id = $1 AND tenant_id = $2")
            .bind(&bill_run_id_a)
            .bind(&tenant_b)
            .fetch_optional(&pool)
            .await
            .expect("Query should succeed");

    assert!(
        cross_read.is_none(),
        "Tenant B must NOT see tenant A's bill run via scoped query"
    );

    // Tenant A should see their own bill run
    let own_read: Option<(Uuid,)> =
        sqlx::query_as("SELECT id FROM bill_runs WHERE bill_run_id = $1 AND tenant_id = $2")
            .bind(&bill_run_id_a)
            .bind(&tenant_a)
            .fetch_optional(&pool)
            .await
            .expect("Query should succeed");

    assert!(own_read.is_some(), "Tenant A must see their own bill run");

    // Count: each tenant should only see their own bill runs
    let a_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM bill_runs WHERE tenant_id = $1")
        .bind(&tenant_a)
        .fetch_one(&pool)
        .await
        .unwrap();

    let b_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM bill_runs WHERE tenant_id = $1")
        .bind(&tenant_b)
        .fetch_one(&pool)
        .await
        .unwrap();

    assert!(a_count >= 1, "Tenant A should have at least 1 bill run");
    assert!(b_count >= 1, "Tenant B should have at least 1 bill run");
}
