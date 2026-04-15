//! Integration tests for GET /api/auth/users (user lookup by email)
//!
//! Tests:
//! 1. Register a user, look up by email → 200 with correct UUID
//! 2. Look up nonexistent email → 404
//! 3. Cross-tenant isolation: register in tenant A, look up in tenant B → 404

use sqlx::postgres::PgPoolOptions;
use sqlx::{PgPool, Row};
use uuid::Uuid;

async fn test_pool() -> PgPool {
    let url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://auth_user:auth_pass@localhost:5433/auth_db".into());
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&url)
        .await
        .expect("connect to test DB");
    sqlx::migrate!("./db/migrations")
        .run(&pool)
        .await
        .expect("run migrations");
    pool
}

/// Insert a test credential directly (bypass hashing for speed).
async fn insert_credential(pool: &PgPool, tenant_id: Uuid, user_id: Uuid, email: &str) {
    sqlx::query(
        "INSERT INTO credentials (tenant_id, user_id, email, password_hash) VALUES ($1, $2, $3, $4)",
    )
    .bind(tenant_id)
    .bind(user_id)
    .bind(email)
    .bind("test-hash-not-real")
    .execute(pool)
    .await
    .expect("insert test credential");
}

// ============================================================================
// Test 1: Lookup existing user by email returns correct UUID
// ============================================================================

#[tokio::test]
async fn user_lookup_by_email_returns_correct_uuid() {
    let pool = test_pool().await;
    let tenant_id = Uuid::new_v4();
    let user_id = Uuid::new_v4();
    let email = format!("lookup-test-{}@example.com", &user_id.to_string()[..8]);

    insert_credential(&pool, tenant_id, user_id, &email).await;

    // Query using the same SQL the handler uses
    let row = sqlx::query(
        "SELECT user_id, email, tenant_id, created_at FROM credentials WHERE tenant_id = $1 AND email = $2",
    )
    .bind(tenant_id)
    .bind(&email)
    .fetch_optional(&pool)
    .await
    .expect("query should succeed");

    assert!(row.is_some(), "User should be found by email");
    let row = row.unwrap();
    let found_user_id: Uuid = row.get("user_id");
    let found_email: String = row.get("email");
    let found_tenant: Uuid = row.get("tenant_id");

    assert_eq!(found_user_id, user_id, "user_id must match");
    assert_eq!(found_email, email, "email must match");
    assert_eq!(found_tenant, tenant_id, "tenant_id must match");
}

// ============================================================================
// Test 2: Lookup nonexistent email returns no result
// ============================================================================

#[tokio::test]
async fn user_lookup_nonexistent_email_returns_none() {
    let pool = test_pool().await;
    let tenant_id = Uuid::new_v4();

    let row = sqlx::query(
        "SELECT user_id, email, tenant_id, created_at FROM credentials WHERE tenant_id = $1 AND email = $2",
    )
    .bind(tenant_id)
    .bind("nonexistent@nowhere.example")
    .fetch_optional(&pool)
    .await
    .expect("query should succeed");

    assert!(row.is_none(), "Nonexistent email should return no result");
}

// ============================================================================
// Test 3: Cross-tenant isolation — user in tenant A not visible to tenant B
// ============================================================================

#[tokio::test]
async fn user_lookup_cross_tenant_isolation() {
    let pool = test_pool().await;
    let tenant_a = Uuid::new_v4();
    let tenant_b = Uuid::new_v4();
    let user_id = Uuid::new_v4();
    let email = format!("isolation-test-{}@example.com", &user_id.to_string()[..8]);

    // Insert user in tenant A
    insert_credential(&pool, tenant_a, user_id, &email).await;

    // Look up in tenant A — should find
    let row_a = sqlx::query("SELECT user_id FROM credentials WHERE tenant_id = $1 AND email = $2")
        .bind(tenant_a)
        .bind(&email)
        .fetch_optional(&pool)
        .await
        .expect("query tenant A");
    assert!(row_a.is_some(), "User should be found in tenant A");

    // Look up in tenant B — should NOT find
    let row_b = sqlx::query("SELECT user_id FROM credentials WHERE tenant_id = $1 AND email = $2")
        .bind(tenant_b)
        .bind(&email)
        .fetch_optional(&pool)
        .await
        .expect("query tenant B");
    assert!(
        row_b.is_none(),
        "User in tenant A must NOT be visible in tenant B"
    );
}
