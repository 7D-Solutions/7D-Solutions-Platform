//! Shared helpers for audit integration tests.

use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;
use std::time::Duration;

const AUDIT_MIGRATION_LOCK_KEY: i64 = 7_419_283_561_i64;

/// Connect to the audit database with retry.
pub async fn get_audit_pool() -> PgPool {
    let url = std::env::var("AUDIT_DATABASE_URL")
        .or_else(|_| std::env::var("PLATFORM_AUDIT_DATABASE_URL"))
        .unwrap_or_else(|_| {
            "postgresql://audit_user:audit_pass@localhost:5440/audit_db".to_string()
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
                        "Audit DB not ready after 10s. URL: {}. Last error: {}",
                        url, e
                    );
                }
            }
        }
        tokio::time::sleep(delay).await;
        delay = (delay * 2).min(Duration::from_secs(1));
    }
}

/// Run audit migrations under advisory lock to prevent catalog deadlocks.
pub async fn run_audit_migrations(pool: &PgPool) {
    sqlx::query("SELECT pg_advisory_lock($1)")
        .bind(AUDIT_MIGRATION_LOCK_KEY)
        .execute(pool)
        .await
        .expect("Failed to acquire audit migration advisory lock");

    let migration_sql = include_str!("../db/migrations/20260216000001_create_audit_log.sql");

    let result = sqlx::raw_sql(migration_sql).execute(pool).await;

    sqlx::query("SELECT pg_advisory_unlock($1)")
        .bind(AUDIT_MIGRATION_LOCK_KEY)
        .execute(pool)
        .await
        .expect("Failed to release audit migration advisory lock");

    result.expect("Audit migration failed");
}
