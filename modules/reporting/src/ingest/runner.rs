//! Backfill runner for reporting caches.
//!
//! The `BackfillRunner` provides a controlled way to trigger a full cache
//! rebuild by resetting ingestion checkpoints. After reset, the next time
//! each consumer starts it will re-process all events from the beginning,
//! rebuilding the cache from scratch.
//!
//! ## Workflow
//!
//! 1. Operator calls `BackfillRunner::reset_tenant` or `::reset_all`.
//! 2. Checkpoint rows are deleted.
//! 3. On next service restart (or consumer re-subscribe), all events are
//!    re-ingested and the cache is rebuilt idempotently.
//!
//! ## Safety
//!
//! Because reporting cache tables use `ON CONFLICT DO UPDATE` (upsert) guards,
//! re-processing events never duplicates rows — it simply overwrites cache
//! entries with the same computed values.

use sqlx::PgPool;

use crate::ingest::checkpoints;

// ── BackfillRunner ────────────────────────────────────────────────────────────

/// Triggers cache rebuilds by resetting ingestion checkpoints.
#[derive(Clone)]
pub struct BackfillRunner {
    pool: PgPool,
}

impl BackfillRunner {
    /// Create a `BackfillRunner` bound to the given reporting DB pool.
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Reset the checkpoint for a single (consumer_name, tenant_id) pair.
    ///
    /// Returns `true` if a checkpoint existed and was deleted, `false` if
    /// there was nothing to reset (consumer had never run for this tenant).
    pub async fn reset_tenant(
        &self,
        consumer_name: &str,
        tenant_id: &str,
    ) -> Result<bool, anyhow::Error> {
        let deleted = checkpoints::reset(&self.pool, consumer_name, tenant_id).await?;
        tracing::info!(
            consumer = consumer_name,
            tenant_id,
            deleted,
            "BackfillRunner: checkpoint reset"
        );
        Ok(deleted > 0)
    }

    /// Reset all tenant checkpoints for a given consumer.
    ///
    /// Use this to rebuild the cache for every tenant at once.
    /// Returns the number of checkpoint rows deleted.
    pub async fn reset_all(&self, consumer_name: &str) -> Result<u64, anyhow::Error> {
        let deleted = checkpoints::reset_all(&self.pool, consumer_name).await?;
        tracing::info!(
            consumer = consumer_name,
            deleted,
            "BackfillRunner: all checkpoints reset"
        );
        Ok(deleted)
    }

    /// List all active consumer names tracked in the checkpoint table.
    ///
    /// Useful for operators who want to know which consumers have state.
    pub async fn list_consumers(&self) -> Result<Vec<String>, anyhow::Error> {
        let names: Vec<String> = sqlx::query_scalar(
            "SELECT DISTINCT consumer_name FROM rpt_ingestion_checkpoints ORDER BY 1",
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(names)
    }

    /// Count total checkpoint rows (across all consumers and tenants).
    pub async fn checkpoint_count(&self) -> Result<i64, anyhow::Error> {
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM rpt_ingestion_checkpoints")
            .fetch_one(&self.pool)
            .await?;
        Ok(count)
    }
}

// ── Integrated tests (real DB, no mocks) ──────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ingest::checkpoints;
    use serial_test::serial;

    const CONSUMER: &str = "test-runner-consumer";

    fn test_db_url() -> String {
        std::env::var("REPORTING_DATABASE_URL").unwrap_or_else(|_| {
            "postgres://ap_user:ap_pass@localhost:5443/reporting_test".to_string()
        })
    }

    async fn test_pool() -> PgPool {
        let pool = PgPool::connect(&test_db_url())
            .await
            .expect("Failed to connect to reporting test DB");
        sqlx::migrate!("./db/migrations")
            .run(&pool)
            .await
            .expect("Failed to run reporting migrations");
        pool
    }

    async fn cleanup(pool: &PgPool) {
        sqlx::query(
            "DELETE FROM rpt_ingestion_checkpoints WHERE consumer_name LIKE 'test-runner-%'",
        )
        .execute(pool)
        .await
        .ok();
    }

    #[tokio::test]
    #[serial]
    async fn test_reset_tenant_removes_checkpoint() {
        let pool = test_pool().await;
        cleanup(&pool).await;

        // Seed a checkpoint
        checkpoints::save(&pool, CONSUMER, "tenant-a", 10, "evt-100")
            .await
            .expect("seed save");

        let runner = BackfillRunner::new(pool.clone());
        let existed = runner
            .reset_tenant(CONSUMER, "tenant-a")
            .await
            .expect("reset");
        assert!(existed, "should report that checkpoint existed");

        let cp = checkpoints::load(&pool, CONSUMER, "tenant-a")
            .await
            .expect("load");
        assert!(cp.is_none(), "checkpoint must be gone after reset");

        cleanup(&pool).await;
    }

    #[tokio::test]
    #[serial]
    async fn test_reset_tenant_returns_false_when_nothing_to_reset() {
        let pool = test_pool().await;
        cleanup(&pool).await;

        let runner = BackfillRunner::new(pool.clone());
        let existed = runner
            .reset_tenant(CONSUMER, "tenant-never-seen")
            .await
            .expect("reset on missing");
        assert!(!existed, "nothing to reset should return false");

        cleanup(&pool).await;
    }

    #[tokio::test]
    #[serial]
    async fn test_reset_all_removes_all_consumer_checkpoints() {
        let pool = test_pool().await;
        cleanup(&pool).await;

        checkpoints::save(&pool, CONSUMER, "tenant-x", 1, "e1")
            .await
            .ok();
        checkpoints::save(&pool, CONSUMER, "tenant-y", 2, "e2")
            .await
            .ok();
        checkpoints::save(&pool, CONSUMER, "tenant-z", 3, "e3")
            .await
            .ok();

        let runner = BackfillRunner::new(pool.clone());
        let deleted = runner.reset_all(CONSUMER).await.expect("reset_all");
        assert_eq!(deleted, 3);

        for t in &["tenant-x", "tenant-y", "tenant-z"] {
            let cp = checkpoints::load(&pool, CONSUMER, t).await.expect("load");
            assert!(cp.is_none(), "checkpoint for {t} must be gone");
        }

        cleanup(&pool).await;
    }

    #[tokio::test]
    #[serial]
    async fn test_reset_all_does_not_affect_other_consumers() {
        let pool = test_pool().await;
        cleanup(&pool).await;

        let other_consumer = "test-runner-other";
        checkpoints::save(&pool, CONSUMER, "tenant-a", 1, "e1")
            .await
            .ok();
        checkpoints::save(&pool, other_consumer, "tenant-a", 2, "e2")
            .await
            .ok();

        let runner = BackfillRunner::new(pool.clone());
        runner.reset_all(CONSUMER).await.expect("reset_all");

        // The other consumer's checkpoint must survive
        let cp = checkpoints::load(&pool, other_consumer, "tenant-a")
            .await
            .expect("load other");
        assert!(
            cp.is_some(),
            "other consumer's checkpoint must not be deleted"
        );

        // Cleanup other consumer too
        sqlx::query("DELETE FROM rpt_ingestion_checkpoints WHERE consumer_name = $1")
            .bind(other_consumer)
            .execute(&pool)
            .await
            .ok();

        cleanup(&pool).await;
    }

    #[tokio::test]
    #[serial]
    async fn test_checkpoint_count_and_list_consumers() {
        let pool = test_pool().await;
        cleanup(&pool).await;

        let runner = BackfillRunner::new(pool.clone());

        let initial_count = runner.checkpoint_count().await.expect("count");
        let initial_consumers = runner.list_consumers().await.expect("list");

        checkpoints::save(&pool, CONSUMER, "tenant-m", 1, "e1")
            .await
            .ok();
        checkpoints::save(&pool, CONSUMER, "tenant-n", 2, "e2")
            .await
            .ok();

        let new_count = runner.checkpoint_count().await.expect("count2");
        assert_eq!(new_count, initial_count + 2);

        let consumers = runner.list_consumers().await.expect("list2");
        assert!(
            consumers.contains(&CONSUMER.to_string()),
            "consumer must appear in list"
        );
        assert!(consumers.len() >= initial_consumers.len() + 1);

        cleanup(&pool).await;
    }
}
