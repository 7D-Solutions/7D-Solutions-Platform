//! Shared helpers for event-consumer integration tests.

use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;
use std::time::Duration;

const MIGRATION_LOCK_KEY: i64 = 8_392_174_650_i64;

/// Connect to the test database with retry.
pub async fn get_pool() -> PgPool {
    let url = std::env::var("EVENT_CONSUMER_DATABASE_URL")
        .or_else(|_| std::env::var("AUDIT_DATABASE_URL"))
        .or_else(|_| std::env::var("PLATFORM_AUDIT_DATABASE_URL"))
        .unwrap_or_else(|_| {
            "postgresql://audit_user:audit_pass@localhost:5440/audit_db?sslmode=require".to_string()
        });

    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    let mut delay = Duration::from_millis(100);

    loop {
        match PgPoolOptions::new()
            .max_connections(5)
            .min_connections(0)
            .acquire_timeout(Duration::from_secs(3))
            .connect(&url)
            .await
        {
            Ok(pool) => {
                if sqlx::query("SELECT 1").execute(&pool).await.is_ok() {
                    return pool;
                }
            }
            Err(e) => {
                if tokio::time::Instant::now() >= deadline {
                    panic!(
                        "Test DB not ready after 10s. URL: {}. Last error: {}",
                        url, e
                    );
                }
            }
        }
        tokio::time::sleep(delay).await;
        delay = (delay * 2).min(Duration::from_secs(1));
    }
}

/// Run event-consumer migrations under advisory lock.
pub async fn run_migrations(pool: &PgPool) {
    sqlx::query("SELECT pg_advisory_lock($1)")
        .bind(MIGRATION_LOCK_KEY)
        .execute(pool)
        .await
        .expect("Failed to acquire migration advisory lock");

    let dedupe_sql = include_str!("../sql/event_dedupe.sql");
    let dlq_sql = include_str!("../sql/event_dlq.sql");

    let r1 = sqlx::raw_sql(dedupe_sql).execute(pool).await;
    let r2 = sqlx::raw_sql(dlq_sql).execute(pool).await;

    sqlx::query("SELECT pg_advisory_unlock($1)")
        .bind(MIGRATION_LOCK_KEY)
        .execute(pool)
        .await
        .expect("Failed to release migration advisory lock");

    r1.expect("event_dedupe migration failed");
    r2.expect("event_dlq migration failed");
}
