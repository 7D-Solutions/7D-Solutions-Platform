//! Inventory valuation ingestion for the reporting KPI cache.
//!
//! Subscribes to inventory valuation snapshot events and stores the
//! computed inventory value in `rpt_kpi_cache` under the key `inventory_value`.
//!
//! ## Event consumed
//!
//! - `inventory.events.inventory.valuation_snapshot` — published when the
//!   inventory module computes a point-in-time valuation of all stock.
//!
//! ## Payload fields (mirrored — no dependency on inventory crate)
//!
//! ```json
//! {
//!   "as_of": "2026-03-01",
//!   "currency": "USD",
//!   "total_value_minor": 500000
//! }
//! ```

use std::sync::Arc;

use async_trait::async_trait;
use chrono::NaiveDate;
use serde::Deserialize;
use sqlx::PgPool;

use event_bus::EventBus;

use crate::ingest::{start_consumer, IngestConsumer, StreamHandler};

// ── Constants ────────────────────────────────────────────────────────────────

pub const SUBJECT_VALUATION_SNAPSHOT: &str = "inventory.events.inventory.valuation_snapshot";
pub const CONSUMER_INVENTORY_VALUE: &str = "reporting.inventory_value";

// ── Local payload mirror ─────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct ValuationSnapshotPayload {
    as_of: NaiveDate,
    currency: String,
    total_value_minor: i64,
}

// ── Handler ──────────────────────────────────────────────────────────────────

/// Stores the inventory valuation snapshot in `rpt_kpi_cache`.
pub struct InventoryValueHandler;

#[async_trait]
impl StreamHandler for InventoryValueHandler {
    async fn handle(
        &self,
        pool: &PgPool,
        tenant_id: &str,
        _event_id: &str,
        payload: &serde_json::Value,
    ) -> Result<(), anyhow::Error> {
        let p: ValuationSnapshotPayload = serde_json::from_value(payload.clone())
            .map_err(|e| anyhow::anyhow!("Failed to parse valuation snapshot payload: {}", e))?;

        sqlx::query(
            r#"
            INSERT INTO rpt_kpi_cache
                (tenant_id, as_of, kpi_name, currency, amount_minor, computed_at)
            VALUES ($1, $2, 'inventory_value', $3, $4, NOW())
            ON CONFLICT (tenant_id, as_of, kpi_name, currency) DO UPDATE SET
                amount_minor = EXCLUDED.amount_minor,
                computed_at  = NOW()
            "#,
        )
        .bind(tenant_id)
        .bind(p.as_of)
        .bind(&p.currency)
        .bind(p.total_value_minor)
        .execute(pool)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to upsert inventory KPI cache: {}", e))?;

        tracing::debug!(
            tenant_id,
            as_of = %p.as_of,
            currency = %p.currency,
            total_value_minor = p.total_value_minor,
            "Inventory valuation stored in KPI cache"
        );

        Ok(())
    }
}

// ── Consumer registration ─────────────────────────────────────────────────────

/// Register the inventory valuation consumer.
pub fn register_consumers(pool: PgPool, bus: Arc<dyn EventBus>) {
    let handler = Arc::new(InventoryValueHandler);
    let consumer = IngestConsumer::new(CONSUMER_INVENTORY_VALUE, pool, handler);
    start_consumer(consumer, bus, SUBJECT_VALUATION_SNAPSHOT);
}

// ── Integrated tests ──────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;
    use event_bus::BusMessage;
    use serial_test::serial;
    use sqlx::PgPool;

    const TENANT: &str = "test-inventory-kpi";

    fn test_db_url() -> String {
        std::env::var("REPORTING_DATABASE_URL")
            .unwrap_or_else(|_| "postgres://ap_user:ap_pass@localhost:5443/reporting_test".into())
    }

    async fn test_pool() -> PgPool {
        let pool = PgPool::connect(&test_db_url()).await.expect("connect");
        sqlx::migrate!("./db/migrations").run(&pool).await.expect("migrate");
        pool
    }

    async fn cleanup(pool: &PgPool) {
        sqlx::query("DELETE FROM rpt_kpi_cache WHERE tenant_id = $1")
            .bind(TENANT)
            .execute(pool)
            .await
            .ok();
        sqlx::query(
            "DELETE FROM rpt_ingestion_checkpoints WHERE consumer_name LIKE 'test-inv-kpi-%'",
        )
        .execute(pool)
        .await
        .ok();
    }

    fn make_valuation_envelope(event_id: &str, as_of: &str, currency: &str, total: i64) -> Vec<u8> {
        serde_json::to_vec(&serde_json::json!({
            "event_id": event_id,
            "tenant_id": TENANT,
            "payload": {
                "as_of": as_of,
                "currency": currency,
                "total_value_minor": total
            }
        }))
        .unwrap()
    }

    async fn fetch_kpi(pool: &PgPool, as_of: NaiveDate, currency: &str) -> Option<i64> {
        sqlx::query_as::<_, (i64,)>(
            r#"
            SELECT amount_minor FROM rpt_kpi_cache
            WHERE tenant_id = $1 AND kpi_name = 'inventory_value'
              AND as_of = $2 AND currency = $3
            "#,
        )
        .bind(TENANT)
        .bind(as_of)
        .bind(currency)
        .fetch_optional(pool)
        .await
        .expect("fetch")
        .map(|(v,)| v)
    }

    #[tokio::test]
    #[serial]
    async fn test_valuation_snapshot_stored_in_kpi_cache() {
        let pool = test_pool().await;
        cleanup(&pool).await;

        let handler = Arc::new(InventoryValueHandler);
        let consumer = IngestConsumer::new("test-inv-kpi-1", pool.clone(), handler);

        let msg = BusMessage::new(
            SUBJECT_VALUATION_SNAPSHOT.to_string(),
            make_valuation_envelope("evt-inv-val-1", "2026-03-01", "USD", 500000),
        );
        consumer.process_message(&msg).await.expect("process");

        let as_of = NaiveDate::from_ymd_opt(2026, 3, 1).unwrap();
        let val = fetch_kpi(&pool, as_of, "USD").await.expect("kpi row");
        assert_eq!(val, 500000, "inventory value mismatch");

        cleanup(&pool).await;
    }

    #[tokio::test]
    #[serial]
    async fn test_valuation_snapshot_idempotent_update() {
        let pool = test_pool().await;
        cleanup(&pool).await;

        let as_of = NaiveDate::from_ymd_opt(2026, 3, 15).unwrap();

        // First snapshot: $200,000
        let h1 = Arc::new(InventoryValueHandler);
        let c1 = IngestConsumer::new("test-inv-kpi-2a", pool.clone(), h1);
        c1.process_message(&BusMessage::new(
            SUBJECT_VALUATION_SNAPSHOT.to_string(),
            make_valuation_envelope("evt-inv-val-2a", "2026-03-15", "USD", 200000),
        ))
        .await
        .expect("first snapshot");

        // Updated snapshot on same date: $220,000
        let h2 = Arc::new(InventoryValueHandler);
        let c2 = IngestConsumer::new("test-inv-kpi-2b", pool.clone(), h2);
        c2.process_message(&BusMessage::new(
            SUBJECT_VALUATION_SNAPSHOT.to_string(),
            make_valuation_envelope("evt-inv-val-2b", "2026-03-15", "USD", 220000),
        ))
        .await
        .expect("second snapshot");

        let val = fetch_kpi(&pool, as_of, "USD").await.expect("kpi row");
        assert_eq!(val, 220000, "second snapshot should overwrite first");

        cleanup(&pool).await;
    }

    #[tokio::test]
    #[serial]
    async fn test_multi_currency_inventory_snapshots() {
        let pool = test_pool().await;
        cleanup(&pool).await;

        let as_of = NaiveDate::from_ymd_opt(2026, 4, 1).unwrap();

        let handler = Arc::new(InventoryValueHandler);
        for (i, (cur, val)) in [("USD", 300000_i64), ("EUR", 180000)].iter().enumerate() {
            IngestConsumer::new(
                format!("test-inv-kpi-mc-{}", i),
                pool.clone(),
                handler.clone(),
            )
            .process_message(&BusMessage::new(
                SUBJECT_VALUATION_SNAPSHOT.to_string(),
                make_valuation_envelope(
                    &format!("evt-inv-mc-{}", i),
                    "2026-04-01",
                    cur,
                    *val,
                ),
            ))
            .await
            .expect("process");
        }

        assert_eq!(fetch_kpi(&pool, as_of, "USD").await.expect("USD"), 300000);
        assert_eq!(fetch_kpi(&pool, as_of, "EUR").await.expect("EUR"), 180000);

        cleanup(&pool).await;
    }
}
