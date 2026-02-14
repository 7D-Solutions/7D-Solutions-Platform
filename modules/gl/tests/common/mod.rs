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

use gl_rs::db::init_pool;
use sqlx::PgPool;
use tokio::sync::OnceCell;

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
