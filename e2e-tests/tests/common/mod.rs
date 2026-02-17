//! Common Test Infrastructure for Cross-Module E2E Tests (Phase 15 - bd-3rc.1)
//!
//! **Purpose:** Shared utilities for bd-3rc.2-8 test suites
//!
//! **Components:**
//! 1. Multi-DB pool connections (AR, Payments, Subscriptions, GL, Auth)
//! 2. NATS event bus setup
//! 3. Polling helpers for async event processing
//! 4. Assertion utilities for invariant validation
//! 5. Test data cleanup helpers
//!
//! **Pattern:** Follows Phase 11/12 boundary E2E test patterns

use async_nats::Client as NatsClient;
use chrono::NaiveDate;
use futures::StreamExt;
use sqlx::postgres::PgPoolOptions;
use sqlx::{PgPool, Row};
use std::time::Duration;
use tokio::time::sleep;
use uuid::Uuid;

// ============================================================================
// Database Pool Connections
// ============================================================================

/// Wait for a database to accept connections, retrying with backoff.
///
/// Invariant: returns Ok only when the DB is reachable. Fails fast (5s max)
/// with an actionable error identifying which DB was unavailable.
pub async fn wait_for_db_ready(name: &str, url: &str) -> PgPool {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    let mut delay = Duration::from_millis(100);
    loop {
        match PgPoolOptions::new()
            .max_connections(5)
            .min_connections(0)
            .acquire_timeout(Duration::from_secs(3))
            .connect(url)
            .await
        {
            Ok(pool) => {
                // Verify connectivity with a lightweight query
                if sqlx::query("SELECT 1").execute(&pool).await.is_ok() {
                    return pool;
                }
            }
            Err(e) => {
                if tokio::time::Instant::now() >= deadline {
                    panic!(
                        "DB '{}' not ready after 10s. URL: {}. Last error: {}.\n\
                         Hint: ensure the corresponding postgres service is running.",
                        name, url, e
                    );
                }
            }
        }
        sleep(delay).await;
        delay = (delay * 2).min(Duration::from_secs(1));
    }
}

// ============================================================================
// DB URL helpers (with defaults) — use these instead of env::var().expect()
// so tests don't panic when optional env vars are absent.
// ============================================================================

/// Resolve AR database URL with local-dev default.
pub fn get_ar_db_url() -> String {
    std::env::var("AR_DATABASE_URL")
        .unwrap_or_else(|_| "postgresql://ar_user:ar_pass@localhost:5434/ar_db".to_string())
}

/// Resolve Payments database URL with local-dev default.
pub fn get_payments_db_url() -> String {
    std::env::var("PAYMENTS_DATABASE_URL")
        .unwrap_or_else(|_| "postgresql://payments_user:payments_pass@localhost:5436/payments_db".to_string())
}

/// Resolve GL database URL with local-dev default.
pub fn get_gl_db_url() -> String {
    std::env::var("GL_DATABASE_URL")
        .unwrap_or_else(|_| "postgresql://gl_user:gl_pass@localhost:5438/gl_db".to_string())
}

/// Resolve Audit database URL (accepts AUDIT_DATABASE_URL or PLATFORM_AUDIT_DATABASE_URL).
pub fn get_audit_db_url() -> String {
    std::env::var("AUDIT_DATABASE_URL")
        .or_else(|_| std::env::var("PLATFORM_AUDIT_DATABASE_URL"))
        .unwrap_or_else(|_| "postgresql://audit_user:audit_pass@localhost:5440/audit_db".to_string())
}

/// Resolve Tenant Registry database URL with local-dev default.
pub fn get_tenant_registry_db_url() -> String {
    std::env::var("TENANT_REGISTRY_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .unwrap_or_else(|_| "postgresql://tenant_registry_user:tenant_registry_pass@localhost:5441/tenant_registry_db".to_string())
}

/// Get AR database pool
pub async fn get_ar_pool() -> PgPool {
    let url = std::env::var("AR_DATABASE_URL")
        .unwrap_or_else(|_| "postgresql://ar_user:ar_pass@localhost:5434/ar_db".to_string());
    wait_for_db_ready("ar", &url).await
}

/// Get Payments database pool
pub async fn get_payments_pool() -> PgPool {
    let url = std::env::var("PAYMENTS_DATABASE_URL")
        .unwrap_or_else(|_| "postgresql://payments_user:payments_pass@localhost:5436/payments_db".to_string());
    wait_for_db_ready("payments", &url).await
}

/// Get Subscriptions database pool
pub async fn get_subscriptions_pool() -> PgPool {
    let url = std::env::var("SUBSCRIPTIONS_DATABASE_URL")
        .unwrap_or_else(|_| "postgresql://subscriptions_user:subscriptions_pass@localhost:5435/subscriptions_db".to_string());
    wait_for_db_ready("subscriptions", &url).await
}

/// Get GL database pool
pub async fn get_gl_pool() -> PgPool {
    let url = std::env::var("GL_DATABASE_URL")
        .unwrap_or_else(|_| "postgresql://gl_user:gl_pass@localhost:5438/gl_db".to_string());
    wait_for_db_ready("gl", &url).await
}

/// Get Notifications database pool
pub async fn get_notifications_pool() -> PgPool {
    let url = std::env::var("NOTIFICATIONS_DATABASE_URL")
        .unwrap_or_else(|_| "postgresql://notifications_user:notifications_pass@localhost:5437/notifications_db".to_string());
    wait_for_db_ready("notifications", &url).await
}

/// Get Auth database pool
pub async fn get_auth_pool() -> PgPool {
    let url = std::env::var("AUTH_DATABASE_URL")
        .or_else(|_| std::env::var("IDENTITY_DATABASE_URL"))
        .unwrap_or_else(|_| "postgresql://auth_user:auth_pass@localhost:5433/auth_db".to_string());
    wait_for_db_ready("auth", &url).await
}

/// Get Projections database pool
pub async fn get_projections_pool() -> PgPool {
    let url = std::env::var("PROJECTIONS_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .unwrap_or_else(|_| "postgresql://projections_user:projections_pass@localhost:5439/projections_db".to_string());
    wait_for_db_ready("projections", &url).await
}

/// Get Audit database pool
pub async fn get_audit_pool() -> PgPool {
    let url = std::env::var("AUDIT_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .unwrap_or_else(|_| "postgresql://audit_user:audit_pass@localhost:5440/audit_db".to_string());
    wait_for_db_ready("audit", &url).await
}

/// Get Tenant Registry database pool
pub async fn get_tenant_registry_pool() -> PgPool {
    let url = std::env::var("TENANT_REGISTRY_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .unwrap_or_else(|_| "postgresql://tenant_registry_user:tenant_registry_pass@localhost:5441/tenant_registry_db".to_string());
    wait_for_db_ready("tenant-registry", &url).await
}

// ============================================================================
// NATS Event Bus
// ============================================================================

/// Setup NATS client connection
pub async fn setup_nats_client() -> NatsClient {
    let nats_url = std::env::var("NATS_URL")
        .unwrap_or_else(|_| "nats://localhost:4222".to_string());

    async_nats::connect(&nats_url)
        .await
        .expect("Failed to connect to NATS")
}

/// Publish event to NATS subject
pub async fn publish_event<T: serde::Serialize>(
    client: &NatsClient,
    subject: &str,
    payload: &T,
) -> Result<(), String> {
    let json = serde_json::to_vec(payload)
        .map_err(|e| format!("Failed to serialize event: {}", e))?;

    client.publish(subject.to_string(), json.into())
        .await
        .map_err(|e| format!("Failed to publish event: {}", e))?;

    Ok(())
}

/// Subscribe to NATS subject and collect messages
pub async fn subscribe_to_events(
    client: &NatsClient,
    subject: &str,
) -> async_nats::Subscriber {
    client.subscribe(subject.to_string())
        .await
        .expect("Failed to subscribe to NATS subject")
}

// ============================================================================
// Polling Helpers
// ============================================================================

/// Poll for a database record with retry
///
/// **Parameters:**
/// - `pool`: Database connection pool
/// - `query`: SQL query to execute
/// - `params`: Query parameters (use sqlx::query! for type safety)
/// - `max_attempts`: Maximum polling attempts (default: 10)
/// - `delay_ms`: Delay between attempts in milliseconds (default: 200)
///
/// **Returns:** Some(T) if record found, None if timeout
pub async fn poll_for_record<F, Fut, T>(
    mut check_fn: F,
    max_attempts: usize,
    delay_ms: u64,
) -> Option<T>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Option<T>>,
{
    for attempt in 0..max_attempts {
        if let Some(result) = check_fn().await {
            tracing::debug!(
                attempt = attempt + 1,
                max_attempts = max_attempts,
                "Record found"
            );
            return Some(result);
        }

        if attempt < max_attempts - 1 {
            sleep(Duration::from_millis(delay_ms)).await;
        }
    }

    tracing::warn!(
        max_attempts = max_attempts,
        "Record not found after polling"
    );
    None
}

/// Poll for invoice creation
pub async fn poll_for_invoice(
    pool: &PgPool,
    app_id: &str,
    ar_customer_id: &str,
    max_attempts: usize,
    delay_ms: u64,
) -> Option<i32> {
    poll_for_record(
        || async {
            sqlx::query_scalar::<_, i32>(
                "SELECT id FROM ar_invoices
                 WHERE app_id = $1 AND ar_customer_id = $2
                 ORDER BY created_at DESC LIMIT 1"
            )
            .bind(app_id)
            .bind(ar_customer_id)
            .fetch_optional(pool)
            .await
            .ok()
            .flatten()
        },
        max_attempts,
        delay_ms,
    )
    .await
}

/// Poll for payment attempt creation
pub async fn poll_for_payment_attempt(
    pool: &PgPool,
    app_id: &str,
    invoice_id: &str,
    max_attempts: usize,
    delay_ms: u64,
) -> Option<Uuid> {
    poll_for_record(
        || async {
            sqlx::query_scalar::<_, Uuid>(
                "SELECT id FROM payment_attempts
                 WHERE app_id = $1 AND invoice_id = $2
                 ORDER BY created_at DESC LIMIT 1"
            )
            .bind(app_id)
            .bind(invoice_id)
            .fetch_optional(pool)
            .await
            .ok()
            .flatten()
        },
        max_attempts,
        delay_ms,
    )
    .await
}

/// Poll for GL journal entry creation
pub async fn poll_for_journal_entry(
    pool: &PgPool,
    tenant_id: &str,
    source_event_id: Uuid,
    max_attempts: usize,
    delay_ms: u64,
) -> Option<Uuid> {
    poll_for_record(
        || async {
            sqlx::query_scalar::<_, Uuid>(
                "SELECT id FROM journal_entries
                 WHERE tenant_id = $1 AND source_event_id = $2
                 LIMIT 1"
            )
            .bind(tenant_id)
            .bind(source_event_id)
            .fetch_optional(pool)
            .await
            .ok()
            .flatten()
        },
        max_attempts,
        delay_ms,
    )
    .await
}

// ============================================================================
// Assertion Utilities
// ============================================================================

/// Assert exactly one record exists matching query
pub async fn assert_exactly_one(
    pool: &PgPool,
    query: &str,
    error_msg: &str,
) -> Result<(), String> {
    let count: i64 = sqlx::query_scalar(query)
        .fetch_one(pool)
        .await
        .map_err(|e| format!("Query failed: {}", e))?;

    if count != 1 {
        return Err(format!("{}: expected 1, got {}", error_msg, count));
    }

    Ok(())
}

/// Assert GL journal entry is balanced (debits == credits)
pub async fn assert_journal_balanced(
    pool: &PgPool,
    entry_id: Uuid,
) -> Result<(), String> {
    let row = sqlx::query(
        "SELECT
            COALESCE(SUM(debit_minor), 0)::BIGINT as total_debits,
            COALESCE(SUM(credit_minor), 0)::BIGINT as total_credits
         FROM journal_lines
         WHERE journal_entry_id = $1"
    )
    .bind(entry_id)
    .fetch_one(pool)
    .await
    .map_err(|e| format!("Failed to fetch journal lines: {}", e))?;

    let total_debits: i64 = row.try_get("total_debits")
        .map_err(|e| format!("Failed to get total_debits: {}", e))?;
    let total_credits: i64 = row.try_get("total_credits")
        .map_err(|e| format!("Failed to get total_credits: {}", e))?;

    if total_debits != total_credits {
        return Err(format!(
            "Journal entry {} unbalanced: debits={}, credits={}",
            entry_id, total_debits, total_credits
        ));
    }

    Ok(())
}

/// Count records matching query
pub async fn count_records(pool: &PgPool, query: &str) -> Result<i64, String> {
    sqlx::query_scalar(query)
        .fetch_one(pool)
        .await
        .map_err(|e| format!("Count query failed: {}", e))
}

// ============================================================================
// Test Data Cleanup
// ============================================================================

/// Cleanup test data for a tenant
pub async fn cleanup_tenant_data(
    ar_pool: &PgPool,
    payments_pool: &PgPool,
    subscriptions_pool: &PgPool,
    gl_pool: &PgPool,
    tenant_id: &str,
) -> Result<(), String> {
    // Cleanup in reverse dependency order

    // GL
    sqlx::query("DELETE FROM journal_lines WHERE journal_entry_id IN (SELECT id FROM journal_entries WHERE tenant_id = $1)")
        .bind(tenant_id)
        .execute(gl_pool)
        .await
        .map_err(|e| format!("Failed to cleanup GL lines: {}", e))?;

    sqlx::query("DELETE FROM journal_entries WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(gl_pool)
        .await
        .map_err(|e| format!("Failed to cleanup GL entries: {}", e))?;

    // Payments
    sqlx::query("DELETE FROM payment_attempts WHERE app_id = $1")
        .bind(tenant_id)
        .execute(payments_pool)
        .await
        .map_err(|e| format!("Failed to cleanup payment attempts: {}", e))?;

    // AR
    sqlx::query("DELETE FROM ar_invoice_attempts WHERE app_id = $1")
        .bind(tenant_id)
        .execute(ar_pool)
        .await
        .map_err(|e| format!("Failed to cleanup AR attempts: {}", e))?;

    sqlx::query("DELETE FROM ar_metered_usage WHERE app_id = $1")
        .bind(tenant_id)
        .execute(ar_pool)
        .await
        .map_err(|e| format!("Failed to cleanup AR metered usage: {}", e))?;

    sqlx::query("DELETE FROM ar_charges WHERE app_id = $1")
        .bind(tenant_id)
        .execute(ar_pool)
        .await
        .map_err(|e| format!("Failed to cleanup AR charges: {}", e))?;

    sqlx::query("DELETE FROM ar_invoice_line_items WHERE app_id = $1")
        .bind(tenant_id)
        .execute(ar_pool)
        .await
        .map_err(|e| format!("Failed to cleanup AR invoice line items: {}", e))?;

    sqlx::query("DELETE FROM ar_invoices WHERE app_id = $1")
        .bind(tenant_id)
        .execute(ar_pool)
        .await
        .map_err(|e| format!("Failed to cleanup AR invoices: {}", e))?;

    sqlx::query("DELETE FROM ar_aging_buckets WHERE app_id = $1")
        .bind(tenant_id)
        .execute(ar_pool)
        .await
        .map_err(|e| format!("Failed to cleanup AR aging buckets: {}", e))?;

    // Subscriptions
    sqlx::query("DELETE FROM subscription_invoice_attempts WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(subscriptions_pool)
        .await
        .map_err(|e| format!("Failed to cleanup subscription attempts: {}", e))?;

    // Subscriptions (after attempts due to FK RESTRICT)
    sqlx::query("DELETE FROM subscriptions WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(subscriptions_pool)
        .await
        .map_err(|e| format!("Failed to cleanup subscriptions: {}", e))?;

    // Subscription plans (after subscriptions due to FK)
    sqlx::query("DELETE FROM subscription_plans WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(subscriptions_pool)
        .await
        .map_err(|e| format!("Failed to cleanup subscription plans: {}", e))?;

    // AR customers (after invoices due to FK RESTRICT)
    sqlx::query("DELETE FROM ar_customers WHERE app_id = $1")
        .bind(tenant_id)
        .execute(ar_pool)
        .await
        .map_err(|e| format!("Failed to cleanup AR customers: {}", e))?;

    Ok(())
}

/// Create a test AR customer and return its integer SERIAL id
pub async fn create_ar_customer(pool: &PgPool, app_id: &str) -> i32 {
    sqlx::query_scalar::<_, i32>(
        "INSERT INTO ar_customers (app_id, email, name, status, retry_attempt_count, created_at, updated_at)
         VALUES ($1, $2, $3, 'active', 0, NOW(), NOW())
         RETURNING id"
    )
    .bind(app_id)
    .bind(format!("customer-{}@test.com", Uuid::new_v4()))
    .bind(format!("Test Customer {}", app_id))
    .fetch_one(pool)
    .await
    .expect("Failed to create test AR customer")
}

/// Generate unique test tenant ID
pub fn generate_test_tenant() -> String {
    format!("test-tenant-{}", Uuid::new_v4())
}

// ============================================================================
// Audit Migrations
// ============================================================================

/// Advisory lock key for serializing audit migration execution.
///
/// Value chosen to avoid collision with any other advisory locks in the system.
/// Same key across all test processes ensures mutual exclusion even under
/// RUST_TEST_THREADS > 1 or multi-process test runs.
const AUDIT_MIGRATION_LOCK_KEY: i64 = 7_419_283_561_i64;

/// Run audit migrations with a pg_advisory_lock to prevent 40P01 catalog deadlocks.
///
/// ## Problem
/// Parallel tests all call `CREATE OR REPLACE FUNCTION` and `CREATE OR REPLACE TRIGGER`
/// on the same `pg_proc` rows simultaneously. PostgreSQL takes an exclusive row lock
/// on `pg_proc` for each `CREATE OR REPLACE FUNCTION`, and when two sessions each hold
/// part of what the other needs, a deadlock (40P01) is thrown.
///
/// ## Fix
/// Acquire a session-level advisory lock before any DDL. Only one session at a time
/// can run the migration; others wait. The lock is released after migration completes
/// (or on connection loss). This is multi-process safe — works with any number of
/// RUST_TEST_THREADS or separate test binary invocations.
pub async fn run_audit_migrations(pool: &PgPool) {
    // Acquire advisory lock — blocks until the previous holder releases.
    sqlx::query("SELECT pg_advisory_lock($1)")
        .bind(AUDIT_MIGRATION_LOCK_KEY)
        .execute(pool)
        .await
        .expect("Failed to acquire audit migration advisory lock");

    // Run DROP + CREATE inside the lock window.
    let result = run_audit_migrations_inner(pool).await;

    // Always release, even on failure, to prevent starvation.
    sqlx::query("SELECT pg_advisory_unlock($1)")
        .bind(AUDIT_MIGRATION_LOCK_KEY)
        .execute(pool)
        .await
        .expect("Failed to release audit migration advisory lock");

    result.expect("Audit migration failed");
}

async fn run_audit_migrations_inner(pool: &PgPool) -> Result<(), sqlx::Error> {
    let migration_sql = include_str!("../../../platform/audit/db/migrations/20260216000001_create_audit_log.sql");

    // Execute the migration idempotently — migration SQL already uses
    // CREATE TABLE IF NOT EXISTS and DO $$ ... EXCEPTION duplicate_object $$
    // for the enum. No DROP needed: tests use unique tenant_ids for isolation
    // and dropping here causes race conditions across parallel test binaries.
    sqlx::raw_sql(migration_sql)
        .execute(pool)
        .await?;

    Ok(())
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_test_tenant() {
        let tenant1 = generate_test_tenant();
        let tenant2 = generate_test_tenant();

        assert!(tenant1.starts_with("test-tenant-"));
        assert!(tenant2.starts_with("test-tenant-"));
        assert_ne!(tenant1, tenant2);
    }
}
