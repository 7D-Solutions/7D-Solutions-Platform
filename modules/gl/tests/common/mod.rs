//! Common test utilities for GL E2E tests
//!
//! ## Singleton Pool Pattern
//! All E2E tests share a single database connection pool per test binary.
//! This prevents resource exhaustion when running 316+ tests.
//!
//! ## Usage
//! ```rust
//! use common::get_test_pool;
//!
//! #[tokio::test]
//! async fn my_test() {
//!     let pool = get_test_pool().await;
//!     // use pool...
//! }
//! ```

use chrono::NaiveDate;
use gl_rs::db::init_pool;
use sqlx::PgPool;
use tokio::sync::OnceCell;
use uuid::Uuid;

/// Singleton pool instance shared across all tests in this binary
static TEST_POOL: OnceCell<PgPool> = OnceCell::const_new();

/// Get or initialize the shared test database pool
///
/// ## Connection Limits
/// Set via environment variables:
/// - `DB_MAX_CONNECTIONS=2` (recommended for E2E)
/// - `DB_MIN_CONNECTIONS=0`
///
/// ## Why Singleton?
/// Without this, each test creates a new pool with 10 connections.
/// With 316+ tests running in parallel (test-threads=4), this creates:
/// - 4 tests × 10 connections = 40 concurrent connections
/// - Multiple test binaries × 40 = 160-320 total connections
/// - Result: Postgres OOM kills (exit code 137)
///
/// With singleton + DB_MAX_CONNECTIONS=2:
/// - 1 test binary × 2 connections = 2 total connections
/// - Even with 8 test binaries = 16 total connections (safe!)
pub async fn get_test_pool() -> PgPool {
    // Set test-specific defaults BEFORE pool initialization
    // Phase 13 period close tests require 5+ connections for nested service calls
    // with serial execution (#[serial] attribute)
    if std::env::var("DB_MAX_CONNECTIONS").is_err() {
        std::env::set_var("DB_MAX_CONNECTIONS", "5");
    }

    // Set longer acquire timeout for tests (10s vs 3s production default)
    // Nested service calls + serial execution may need more time
    if std::env::var("DB_ACQUIRE_TIMEOUT_SECS").is_err() {
        std::env::set_var("DB_ACQUIRE_TIMEOUT_SECS", "10");
    }

    TEST_POOL
        .get_or_init(|| async {
            let database_url = std::env::var("DATABASE_URL").unwrap_or_else(|_| {
                "postgres://gl_user:gl_pass@localhost:5438/gl_db".to_string()
            });

            init_pool(&database_url)
                .await
                .expect("Failed to initialize test pool")
        })
        .await
        .clone()
}

/// Create a test accounting period
///
/// # Returns
/// UUID of the created period
pub async fn setup_test_period(
    pool: &PgPool,
    tenant_id: &str,
    period_start: NaiveDate,
    period_end: NaiveDate,
) -> Uuid {
    let period_id = Uuid::new_v4();
    sqlx::query(
        r#"
        INSERT INTO accounting_periods (id, tenant_id, period_start, period_end, is_closed, created_at)
        VALUES ($1, $2, $3, $4, false, NOW())
        "#,
    )
    .bind(period_id)
    .bind(tenant_id)
    .bind(period_start)
    .bind(period_end)
    .execute(pool)
    .await
    .expect("Failed to create test period");

    period_id
}

/// Create a test account
///
/// # Returns
/// UUID of the created account
pub async fn setup_test_account(
    pool: &PgPool,
    tenant_id: &str,
    code: &str,
    name: &str,
    account_type: &str,
    normal_balance: &str,
) -> Uuid {
    let account_id = Uuid::new_v4();
    sqlx::query(
        r#"
        INSERT INTO accounts (id, tenant_id, code, name, type, normal_balance, is_active, created_at)
        VALUES ($1, $2, $3, $4, $5::account_type, $6::normal_balance, true, NOW())
        "#,
    )
    .bind(account_id)
    .bind(tenant_id)
    .bind(code)
    .bind(name)
    .bind(account_type)
    .bind(normal_balance)
    .execute(pool)
    .await
    .expect("Failed to create test account");

    account_id
}

/// Cleanup test data for a tenant (delete all periods and related data)
///
/// Deletes in reverse FK order to avoid constraint violations.
pub async fn cleanup_test_tenant(pool: &PgPool, tenant_id: &str) {
    // Delete in reverse FK order
    sqlx::query("DELETE FROM period_summary_snapshots WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(pool)
        .await
        .ok();

    sqlx::query("DELETE FROM account_balances WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(pool)
        .await
        .ok();

    sqlx::query(
        "DELETE FROM journal_lines WHERE journal_entry_id IN (SELECT id FROM journal_entries WHERE tenant_id = $1)"
    )
    .bind(tenant_id)
    .execute(pool)
    .await
    .ok();

    sqlx::query("DELETE FROM journal_entries WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(pool)
        .await
        .ok();

    sqlx::query("DELETE FROM accounts WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(pool)
        .await
        .ok();

    sqlx::query("DELETE FROM accounting_periods WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(pool)
        .await
        .ok();
}
