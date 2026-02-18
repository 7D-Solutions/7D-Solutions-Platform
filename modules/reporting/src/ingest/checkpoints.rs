//! Tenant-scoped ingestion checkpoints for idempotent NATS replay.
//!
//! Each row in `rpt_ingestion_checkpoints` tracks the last successfully
//! processed event for a (consumer_name, tenant_id) pair. Consumers upsert
//! here after each successful handler call so they can resume without
//! re-processing events on restart.
//!
//! ## Idempotency model
//! - `last_event_id`: the EventEnvelope `event_id` of the last processed event.
//!   Used as the primary idempotency key — the consumer skips any event whose
//!   `event_id` was already recorded.
//! - `last_sequence`: reserved for NATS JetStream sequence tracking; currently
//!   persisted but not used for gating (event_id is authoritative).
//!
//! ## Backfill
//! Deleting a checkpoint row (`reset`) causes the next consumer run to process
//! all events from the beginning, rebuilding the cache from scratch.

use chrono::{DateTime, Utc};
use sqlx::PgPool;

/// Loaded checkpoint record for a single (consumer_name, tenant_id) pair.
#[derive(Debug, Clone)]
pub struct Checkpoint {
    pub consumer_name: String,
    pub tenant_id: String,
    /// NATS stream sequence of the last processed message (0 = none yet).
    pub last_sequence: i64,
    /// EventEnvelope `event_id` of the last processed event (idempotency key).
    pub last_event_id: Option<String>,
    pub processed_at: DateTime<Utc>,
}

// ── sqlx FromRow shim ────────────────────────────────────────────────────────

#[derive(sqlx::FromRow)]
struct CheckpointRow {
    consumer_name: String,
    tenant_id: String,
    last_sequence: i64,
    last_event_id: Option<String>,
    processed_at: DateTime<Utc>,
}

impl From<CheckpointRow> for Checkpoint {
    fn from(row: CheckpointRow) -> Self {
        Self {
            consumer_name: row.consumer_name,
            tenant_id: row.tenant_id,
            last_sequence: row.last_sequence,
            last_event_id: row.last_event_id,
            processed_at: row.processed_at,
        }
    }
}

// ── Public API ────────────────────────────────────────────────────────────────

/// Load the current checkpoint for a (consumer_name, tenant_id) pair.
/// Returns `None` if no checkpoint exists (consumer has never run).
pub async fn load(
    pool: &PgPool,
    consumer_name: &str,
    tenant_id: &str,
) -> Result<Option<Checkpoint>, sqlx::Error> {
    let row: Option<CheckpointRow> = sqlx::query_as(
        r#"
        SELECT consumer_name, tenant_id, last_sequence, last_event_id, processed_at
        FROM rpt_ingestion_checkpoints
        WHERE consumer_name = $1 AND tenant_id = $2
        "#,
    )
    .bind(consumer_name)
    .bind(tenant_id)
    .fetch_optional(pool)
    .await?;

    Ok(row.map(Checkpoint::from))
}

/// Upsert the checkpoint after successful event processing.
///
/// If a checkpoint already exists for the pair, updates `last_sequence`,
/// `last_event_id`, and `processed_at`. Safe to call multiple times.
pub async fn save(
    pool: &PgPool,
    consumer_name: &str,
    tenant_id: &str,
    last_sequence: i64,
    last_event_id: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        INSERT INTO rpt_ingestion_checkpoints
            (consumer_name, tenant_id, last_sequence, last_event_id, processed_at)
        VALUES ($1, $2, $3, $4, NOW())
        ON CONFLICT (consumer_name, tenant_id) DO UPDATE SET
            last_sequence = EXCLUDED.last_sequence,
            last_event_id = EXCLUDED.last_event_id,
            processed_at  = NOW()
        "#,
    )
    .bind(consumer_name)
    .bind(tenant_id)
    .bind(last_sequence)
    .bind(last_event_id)
    .execute(pool)
    .await?;
    Ok(())
}

/// Reset (delete) the checkpoint for a (consumer_name, tenant_id) pair.
///
/// After reset the next consumer run treats the stream as fresh: it processes
/// all events from sequence 0, rebuilding the cache from scratch. This is the
/// primary backfill trigger used by `BackfillRunner`.
pub async fn reset(
    pool: &PgPool,
    consumer_name: &str,
    tenant_id: &str,
) -> Result<u64, sqlx::Error> {
    let result = sqlx::query(
        "DELETE FROM rpt_ingestion_checkpoints \
         WHERE consumer_name = $1 AND tenant_id = $2",
    )
    .bind(consumer_name)
    .bind(tenant_id)
    .execute(pool)
    .await?;

    Ok(result.rows_affected())
}

/// Reset all tenant checkpoints for a given consumer (full global backfill).
///
/// Returns the number of checkpoint rows deleted.
pub async fn reset_all(pool: &PgPool, consumer_name: &str) -> Result<u64, sqlx::Error> {
    let result = sqlx::query(
        "DELETE FROM rpt_ingestion_checkpoints WHERE consumer_name = $1",
    )
    .bind(consumer_name)
    .execute(pool)
    .await?;

    Ok(result.rows_affected())
}

/// Return true if `event_id` matches the `last_event_id` in the checkpoint.
///
/// This is the fast-path idempotency gate: skip events we definitely already
/// processed (the most-recent one for this consumer+tenant). For events earlier
/// in the replay stream, handler-level `ON CONFLICT` guards handle deduplication.
pub async fn is_processed(
    pool: &PgPool,
    consumer_name: &str,
    tenant_id: &str,
    event_id: &str,
) -> Result<bool, sqlx::Error> {
    let exists: Option<i64> = sqlx::query_scalar(
        r#"
        SELECT 1::BIGINT
        FROM rpt_ingestion_checkpoints
        WHERE consumer_name = $1
          AND tenant_id     = $2
          AND last_event_id = $3
        "#,
    )
    .bind(consumer_name)
    .bind(tenant_id)
    .bind(event_id)
    .fetch_optional(pool)
    .await?;

    Ok(exists.is_some())
}

// ── Integrated tests (real DB, no mocks) ──────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;

    const CONSUMER: &str = "test-cp-consumer";
    const TENANT: &str = "test-cp-tenant-a";

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
            "DELETE FROM rpt_ingestion_checkpoints WHERE consumer_name LIKE 'test-cp-%'",
        )
        .execute(pool)
        .await
        .ok();
    }

    #[tokio::test]
    #[serial]
    async fn test_load_returns_none_for_fresh_consumer() {
        let pool = test_pool().await;
        cleanup(&pool).await;

        let cp = load(&pool, CONSUMER, TENANT).await.expect("load failed");
        assert!(cp.is_none(), "fresh consumer should have no checkpoint");

        cleanup(&pool).await;
    }

    #[tokio::test]
    #[serial]
    async fn test_save_and_load_roundtrip() {
        let pool = test_pool().await;
        cleanup(&pool).await;

        save(&pool, CONSUMER, TENANT, 42, "evt-abc-123")
            .await
            .expect("save failed");

        let cp = load(&pool, CONSUMER, TENANT)
            .await
            .expect("load failed")
            .expect("checkpoint must exist after save");

        assert_eq!(cp.consumer_name, CONSUMER);
        assert_eq!(cp.tenant_id, TENANT);
        assert_eq!(cp.last_sequence, 42);
        assert_eq!(cp.last_event_id.as_deref(), Some("evt-abc-123"));

        cleanup(&pool).await;
    }

    #[tokio::test]
    #[serial]
    async fn test_save_is_idempotent_last_write_wins() {
        let pool = test_pool().await;
        cleanup(&pool).await;

        save(&pool, CONSUMER, TENANT, 1, "evt-first").await.expect("save1");
        save(&pool, CONSUMER, TENANT, 2, "evt-second").await.expect("save2");

        let cp = load(&pool, CONSUMER, TENANT)
            .await
            .expect("load failed")
            .expect("checkpoint must exist");

        assert_eq!(cp.last_sequence, 2);
        assert_eq!(cp.last_event_id.as_deref(), Some("evt-second"));

        cleanup(&pool).await;
    }

    #[tokio::test]
    #[serial]
    async fn test_is_processed_true_for_last_event_id() {
        let pool = test_pool().await;
        cleanup(&pool).await;

        save(&pool, CONSUMER, TENANT, 5, "evt-xyz")
            .await
            .expect("save");

        let result = is_processed(&pool, CONSUMER, TENANT, "evt-xyz")
            .await
            .expect("is_processed");
        assert!(result, "last_event_id must be detected as processed");

        cleanup(&pool).await;
    }

    #[tokio::test]
    #[serial]
    async fn test_is_processed_false_for_new_event() {
        let pool = test_pool().await;
        cleanup(&pool).await;

        save(&pool, CONSUMER, TENANT, 5, "evt-prev")
            .await
            .expect("save");

        let result = is_processed(&pool, CONSUMER, TENANT, "evt-new")
            .await
            .expect("is_processed");
        assert!(!result, "new event_id should not be marked processed");

        cleanup(&pool).await;
    }

    #[tokio::test]
    #[serial]
    async fn test_reset_removes_checkpoint() {
        let pool = test_pool().await;
        cleanup(&pool).await;

        save(&pool, CONSUMER, TENANT, 7, "evt-to-reset")
            .await
            .expect("save");

        let deleted = reset(&pool, CONSUMER, TENANT).await.expect("reset");
        assert_eq!(deleted, 1);

        let cp = load(&pool, CONSUMER, TENANT).await.expect("load");
        assert!(cp.is_none(), "checkpoint must be gone after reset");

        cleanup(&pool).await;
    }

    #[tokio::test]
    #[serial]
    async fn test_reset_all_removes_all_tenant_checkpoints() {
        let pool = test_pool().await;
        cleanup(&pool).await;

        save(&pool, CONSUMER, "tenant-1", 1, "e1").await.expect("s1");
        save(&pool, CONSUMER, "tenant-2", 2, "e2").await.expect("s2");
        save(&pool, CONSUMER, "tenant-3", 3, "e3").await.expect("s3");

        let deleted = reset_all(&pool, CONSUMER).await.expect("reset_all");
        assert_eq!(deleted, 3, "three checkpoint rows must be removed");

        for t in &["tenant-1", "tenant-2", "tenant-3"] {
            let cp = load(&pool, CONSUMER, t).await.expect("load");
            assert!(cp.is_none(), "checkpoint for {t} must be gone");
        }

        cleanup(&pool).await;
    }
}
