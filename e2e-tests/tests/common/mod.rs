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

/// Get AR database pool
pub async fn get_ar_pool() -> PgPool {
    let url = std::env::var("AR_DATABASE_URL")
        .unwrap_or_else(|_| "postgresql://ar_user:ar_pass@localhost:5434/ar_db".to_string());

    PgPoolOptions::new()
        .max_connections(5)
        .min_connections(1)
        .connect(&url)
        .await
        .expect("Failed to connect to AR database")
}

/// Get Payments database pool
pub async fn get_payments_pool() -> PgPool {
    let url = std::env::var("PAYMENTS_DATABASE_URL")
        .unwrap_or_else(|_| "postgresql://payments_user:payments_pass@localhost:5436/payments_db".to_string());

    PgPoolOptions::new()
        .max_connections(5)
        .min_connections(1)
        .connect(&url)
        .await
        .expect("Failed to connect to Payments database")
}

/// Get Subscriptions database pool
pub async fn get_subscriptions_pool() -> PgPool {
    let url = std::env::var("SUBSCRIPTIONS_DATABASE_URL")
        .unwrap_or_else(|_| "postgresql://subscriptions_user:subscriptions_pass@localhost:5435/subscriptions_db".to_string());

    PgPoolOptions::new()
        .max_connections(5)
        .min_connections(1)
        .connect(&url)
        .await
        .expect("Failed to connect to Subscriptions database")
}

/// Get GL database pool
pub async fn get_gl_pool() -> PgPool {
    let url = std::env::var("GL_DATABASE_URL")
        .unwrap_or_else(|_| "postgresql://gl_user:gl_pass@localhost:5438/gl_db".to_string());

    PgPoolOptions::new()
        .max_connections(5)
        .min_connections(1)
        .connect(&url)
        .await
        .expect("Failed to connect to GL database")
}

/// Get Auth database pool
pub async fn get_auth_pool() -> PgPool {
    let url = std::env::var("AUTH_DATABASE_URL")
        .unwrap_or_else(|_| "postgresql://postgres:postgres@localhost:5434/auth".to_string());

    PgPoolOptions::new()
        .max_connections(5)
        .min_connections(1)
        .connect(&url)
        .await
        .expect("Failed to connect to Auth database")
}

/// Get Projections database pool
pub async fn get_projections_pool() -> PgPool {
    let url = std::env::var("PROJECTIONS_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .unwrap_or_else(|_| "postgresql://projections_user:projections_pass@localhost:5439/projections_db".to_string());

    PgPoolOptions::new()
        .max_connections(5)
        .min_connections(1)
        .connect(&url)
        .await
        .expect("Failed to connect to Projections database")
}

/// Get Audit database pool
pub async fn get_audit_pool() -> PgPool {
    let url = std::env::var("AUDIT_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .unwrap_or_else(|_| "postgresql://audit_user:audit_pass@localhost:5440/audit_db".to_string());

    PgPoolOptions::new()
        .max_connections(5)
        .min_connections(1)
        .connect(&url)
        .await
        .expect("Failed to connect to Audit database")
}

/// Get Tenant Registry database pool
pub async fn get_tenant_registry_pool() -> PgPool {
    let url = std::env::var("TENANT_REGISTRY_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .unwrap_or_else(|_| "postgresql://tenant_registry_user:tenant_registry_pass@localhost:5441/tenant_registry_db".to_string());

    PgPoolOptions::new()
        .max_connections(5)
        .min_connections(1)
        .connect(&url)
        .await
        .expect("Failed to connect to Tenant Registry database")
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
            COALESCE(SUM(debit_minor), 0) as total_debits,
            COALESCE(SUM(credit_minor), 0) as total_credits
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

    sqlx::query("DELETE FROM ar_invoices WHERE app_id = $1")
        .bind(tenant_id)
        .execute(ar_pool)
        .await
        .map_err(|e| format!("Failed to cleanup AR invoices: {}", e))?;

    // Subscriptions
    sqlx::query("DELETE FROM subscription_invoice_attempts WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(subscriptions_pool)
        .await
        .map_err(|e| format!("Failed to cleanup subscription attempts: {}", e))?;

    Ok(())
}

/// Generate unique test tenant ID
pub fn generate_test_tenant() -> String {
    format!("test-tenant-{}", Uuid::new_v4())
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
