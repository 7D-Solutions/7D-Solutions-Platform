//! Common test utilities for GL E2E tests
//!
//! ## Per-Test Pool Pattern (Grok Fix for Runtime Binding)
//! Each test gets a fresh database connection pool to avoid Tokio runtime binding issues.
//!
//! **Root Cause:** Static OnceCell pools persist across #[tokio::test] boundaries.
//! Each test runs in its own Tokio runtime, but the pool's internal async machinery
//! (connection manager tasks, I/O drivers) is bound to the first test's runtime.
//! When test 1 finishes, its runtime drops → connections become client-side zombies.
//!
//! **Solution:** Create fresh pool per test. Since tests are serial (#[serial] + --test-threads=1),
//! this is cheap and eliminates runtime contamination.
//!
//! ## Usage
//! ```rust
//! use common::{get_test_pool, log_pool_state};
//!
//! #[tokio::test]
//! #[serial]
//! async fn my_test() {
//!     let pool = get_test_pool().await;
//!     log_pool_state(&pool, "START").await;
//!
//!     // ... test logic ...
//!
//!     log_pool_state(&pool, "BEFORE_CLOSE").await;
//!     pool.close().await;  // ← REQUIRED for graceful cleanup
//! }
//! ```

use chrono::NaiveDate;
use gl_rs::db::init_pool;
use sqlx::PgPool;
use uuid::Uuid;

/// Create a fresh test database pool (no caching, no OnceCell)
///
/// ## Connection Limits
/// Set via environment variables:
/// - `DB_MAX_CONNECTIONS=5` (default for period close tests)
/// - `DB_ACQUIRE_TIMEOUT_SECS=10` (longer for nested service calls)
///
/// ## Why Fresh Per Test?
/// Avoids Tokio runtime binding bugs. Each #[tokio::test] creates its own runtime.
/// Sharing a static pool causes the pool's async tasks to be bound to the first test's runtime.
/// When that runtime drops, connections become unusable (PoolTimedOut errors).
///
/// ## Cleanup
/// **IMPORTANT:** Call `pool.close().await` at end of test for graceful shutdown.
pub async fn get_test_pool() -> PgPool {
    // Set test-specific defaults BEFORE pool initialization
    // Phase 13 period close tests require 5+ connections for nested service calls
    if std::env::var("DB_MAX_CONNECTIONS").is_err() {
        std::env::set_var("DB_MAX_CONNECTIONS", "5");
    }

    // Set longer acquire timeout for tests (10s vs 3s production default)
    if std::env::var("DB_ACQUIRE_TIMEOUT_SECS").is_err() {
        std::env::set_var("DB_ACQUIRE_TIMEOUT_SECS", "10");
    }

    // Get database URL
    let database_url = std::env::var("DATABASE_URL").unwrap_or_else(|_| {
        "postgres://gl_user:gl_pass@localhost:5438/gl_db".to_string()
    });

    // Create fresh pool (no OnceCell, no advisory lock)
    // Serial execution (#[serial] + --test-threads=1) already prevents races
    init_pool(&database_url)
        .await
        .expect("Failed to create test pool")
}

/// Log pool state for diagnostics (Grok verification strategy)
pub async fn log_pool_state(pool: &PgPool, label: &str) {
    eprintln!(
        "[POOL_STATE:{}] size={}, idle={}",
        label,
        pool.size(),
        pool.num_idle()
    );
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
