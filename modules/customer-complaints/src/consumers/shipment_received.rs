//! Consumer for shipping_receiving.inbound_closed events.
//!
//! When a shipment is received (inbound closed), adds a timeline activity log
//! entry to any open complaint whose source_entity_type = 'shipment' and
//! source_entity_id matches.
//!
//! ## Idempotency
//! INSERT into cc_processed_events ON CONFLICT DO NOTHING.

use chrono::{DateTime, Utc};
use event_bus::{BusMessage, EventBus, EventEnvelope};
use futures::StreamExt;
use sqlx::PgPool;
use std::sync::Arc;
use uuid::Uuid;

// ── Anti-corruption layer ─────────────────────────────────────────────────────

#[derive(Debug, Clone, serde::Deserialize)]
pub struct InboundClosedPayload {
    pub tenant_id: String,
    pub shipment_id: Uuid,
    pub closed_at: DateTime<Utc>,
}

// ── Processing ────────────────────────────────────────────────────────────────

pub async fn handle_shipment_received(
    pool: &PgPool,
    event_id: Uuid,
    payload: &InboundClosedPayload,
) -> Result<(), sqlx::Error> {
    let mut tx = pool.begin().await?;

    let inserted: u64 = sqlx::query(
        r#"INSERT INTO cc_processed_events (event_id, event_type, processor)
           VALUES ($1, 'shipping_receiving.inbound_closed', 'cc.shipment_received')
           ON CONFLICT (event_id) DO NOTHING"#,
    )
    .bind(event_id)
    .execute(&mut *tx)
    .await?
    .rows_affected();

    if inserted == 0 {
        tx.rollback().await?;
        tracing::debug!(event_id = %event_id, "cc: shipping_receiving.inbound_closed already processed, skipping");
        return Ok(());
    }

    let complaint_ids: Vec<Uuid> = sqlx::query_scalar(
        r#"SELECT id FROM complaints
           WHERE source_entity_type = 'shipment'
             AND source_entity_id = $1
             AND tenant_id = $2
             AND status NOT IN ('closed', 'cancelled')"#,
    )
    .bind(payload.shipment_id)
    .bind(&payload.tenant_id)
    .fetch_all(&mut *tx)
    .await?;

    for complaint_id in &complaint_ids {
        sqlx::query(
            r#"INSERT INTO complaint_activity_log
               (tenant_id, complaint_id, activity_type, content, visible_to_customer, recorded_by)
               VALUES ($1, $2, 'internal_communication', $3, FALSE, 'system:shipment-received-consumer')"#,
        )
        .bind(&payload.tenant_id)
        .bind(complaint_id)
        .bind(format!(
            "Shipment {} was received at {}.",
            payload.shipment_id, payload.closed_at
        ))
        .execute(&mut *tx)
        .await?;
    }

    tx.commit().await?;

    tracing::info!(
        event_id = %event_id,
        shipment_id = %payload.shipment_id,
        linked_complaints = complaint_ids.len(),
        "cc: shipping_receiving.inbound_closed processed"
    );

    Ok(())
}

// ── NATS consumer ─────────────────────────────────────────────────────────────

pub fn start_shipment_received_consumer(bus: Arc<dyn EventBus>, pool: PgPool) {
    tokio::spawn(async move {
        let subject = "shipping_receiving.inbound_closed";
        let mut stream = match bus.subscribe(subject).await {
            Ok(s) => s,
            Err(e) => {
                tracing::error!(subject, error = %e, "cc: failed to subscribe to shipping_receiving.inbound_closed");
                return;
            }
        };
        tracing::info!(subject, "cc: subscribed to shipping_receiving.inbound_closed");

        while let Some(msg) = stream.next().await {
            if let Err(e) = process_shipment_received_message(&pool, &msg).await {
                tracing::error!(error = %e, "cc: failed to process shipping_receiving.inbound_closed");
            }
        }

        tracing::warn!("cc: shipping_receiving.inbound_closed consumer stopped");
    });
}

async fn process_shipment_received_message(
    pool: &PgPool,
    msg: &BusMessage,
) -> Result<(), Box<dyn std::error::Error>> {
    let envelope: EventEnvelope<InboundClosedPayload> =
        serde_json::from_slice(&msg.payload)
            .map_err(|e| format!("Failed to parse shipping_receiving.inbound_closed envelope: {}", e))?;

    tracing::info!(
        event_id = %envelope.event_id,
        shipment_id = %envelope.payload.shipment_id,
        "cc: processing shipping_receiving.inbound_closed"
    );

    handle_shipment_received(pool, envelope.event_id, &envelope.payload)
        .await
        .map_err(|e| format!("handle_shipment_received failed: {}", e).into())
}

// ── Integrated Tests ──────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;

    fn test_db_url() -> String {
        std::env::var("DATABASE_URL")
            .unwrap_or_else(|_| "postgres://cc_user:cc_pass@localhost:5468/cc_db".to_string())
    }

    async fn test_pool() -> PgPool {
        PgPool::connect(&test_db_url())
            .await
            .expect("Failed to connect to CC test DB")
    }

    fn unique_tenant() -> String {
        format!("cc-sr-{}", Uuid::new_v4().simple())
    }

    async fn seed_complaint_with_shipment(pool: &PgPool, tenant_id: &str, shipment_id: Uuid) -> Uuid {
        sqlx::query_scalar(
            r#"INSERT INTO complaints
               (tenant_id, complaint_number, status, party_id, source, title, created_by,
                source_entity_type, source_entity_id)
               VALUES ($1, $2, 'investigating', gen_random_uuid(), 'email', 'Shipment complaint', 'system', 'shipment', $3)
               RETURNING id"#,
        )
        .bind(tenant_id)
        .bind(format!("CC-{}", Uuid::new_v4().simple()))
        .bind(shipment_id)
        .fetch_one(pool)
        .await
        .expect("seed complaint failed")
    }

    async fn cleanup(pool: &PgPool, tenant_id: &str) {
        sqlx::query("DELETE FROM complaint_activity_log WHERE tenant_id = $1")
            .bind(tenant_id)
            .execute(pool)
            .await
            .ok();
        sqlx::query("DELETE FROM complaints WHERE tenant_id = $1")
            .bind(tenant_id)
            .execute(pool)
            .await
            .ok();
    }

    fn sample_payload(shipment_id: Uuid, tenant_id: &str) -> InboundClosedPayload {
        InboundClosedPayload {
            tenant_id: tenant_id.to_string(),
            shipment_id,
            closed_at: Utc::now(),
        }
    }

    #[tokio::test]
    #[serial]
    async fn test_adds_activity_log_for_linked_shipment() {
        let pool = test_pool().await;
        let tid = unique_tenant();
        cleanup(&pool, &tid).await;

        let shipment_id = Uuid::new_v4();
        let complaint_id = seed_complaint_with_shipment(&pool, &tid, shipment_id).await;

        let event_id = Uuid::new_v4();
        handle_shipment_received(&pool, event_id, &sample_payload(shipment_id, &tid))
            .await
            .expect("handle failed");

        let (count,): (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM complaint_activity_log WHERE complaint_id = $1 AND activity_type = 'internal_communication'",
        )
        .bind(complaint_id)
        .fetch_one(&pool)
        .await
        .expect("count failed");

        assert_eq!(count, 1);
        cleanup(&pool, &tid).await;
    }

    #[tokio::test]
    #[serial]
    async fn test_idempotent_on_redelivery() {
        let pool = test_pool().await;
        let tid = unique_tenant();
        cleanup(&pool, &tid).await;

        let shipment_id = Uuid::new_v4();
        let _complaint_id = seed_complaint_with_shipment(&pool, &tid, shipment_id).await;

        let event_id = Uuid::new_v4();
        let payload = sample_payload(shipment_id, &tid);

        handle_shipment_received(&pool, event_id, &payload).await.expect("first handle failed");
        handle_shipment_received(&pool, event_id, &payload).await.expect("second handle must not error");

        let (count,): (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM complaint_activity_log WHERE tenant_id = $1 AND activity_type = 'internal_communication'",
        )
        .bind(&tid)
        .fetch_one(&pool)
        .await
        .expect("count failed");

        assert_eq!(count, 1, "Redelivery must not duplicate activity log entries");
        cleanup(&pool, &tid).await;
    }
}
