//! Consumer for party.deactivated events.
//!
//! When a party is deactivated, adds a warning activity log entry to all open
//! complaints referencing that party. Does not auto-close complaints.
//!
//! ## Idempotency
//! INSERT into cc_processed_events ON CONFLICT DO NOTHING. If the event_id is
//! already recorded, the handler returns early.

use chrono::{DateTime, Utc};
use event_bus::{BusMessage, EventBus, EventEnvelope};
use futures::StreamExt;
use sqlx::PgPool;
use std::sync::Arc;
use uuid::Uuid;

// ── Anti-corruption layer ─────────────────────────────────────────────────────

#[derive(Debug, Clone, serde::Deserialize)]
pub struct PartyDeactivatedPayload {
    pub party_id: Uuid,
    pub app_id: String,
    pub deactivated_by: String,
    pub deactivated_at: DateTime<Utc>,
}

// ── Processing ────────────────────────────────────────────────────────────────

pub async fn handle_party_deactivated(
    pool: &PgPool,
    event_id: Uuid,
    payload: &PartyDeactivatedPayload,
) -> Result<(), sqlx::Error> {
    let mut tx = pool.begin().await?;

    // Idempotency guard — skip if already processed
    let inserted: u64 = sqlx::query(
        r#"INSERT INTO cc_processed_events (event_id, event_type, processor)
           VALUES ($1, 'party.deactivated', 'cc.party_deactivated')
           ON CONFLICT (event_id) DO NOTHING"#,
    )
    .bind(event_id)
    .execute(&mut *tx)
    .await?
    .rows_affected();

    if inserted == 0 {
        tx.rollback().await?;
        tracing::debug!(event_id = %event_id, "cc: party.deactivated already processed, skipping");
        return Ok(());
    }

    // Find all open complaints referencing this party (tenant scoped to app_id)
    let complaint_ids: Vec<Uuid> = sqlx::query_scalar(
        r#"SELECT id FROM complaints
           WHERE party_id = $1 AND tenant_id = $2
             AND status NOT IN ('closed', 'cancelled')"#,
    )
    .bind(payload.party_id)
    .bind(&payload.app_id)
    .fetch_all(&mut *tx)
    .await?;

    for complaint_id in &complaint_ids {
        sqlx::query(
            r#"INSERT INTO complaint_activity_log
               (tenant_id, complaint_id, activity_type, content, visible_to_customer, recorded_by)
               VALUES ($1, $2, 'internal_communication', $3, FALSE, 'system:party-consumer')"#,
        )
        .bind(&payload.app_id)
        .bind(complaint_id)
        .bind(format!(
            "Warning: party {} was deactivated by {} at {}. Review this complaint.",
            payload.party_id, payload.deactivated_by, payload.deactivated_at
        ))
        .execute(&mut *tx)
        .await?;
    }

    tx.commit().await?;

    tracing::info!(
        event_id = %event_id,
        party_id = %payload.party_id,
        flagged_complaints = complaint_ids.len(),
        "cc: party.deactivated processed"
    );

    Ok(())
}

// ── NATS consumer ─────────────────────────────────────────────────────────────

pub fn start_party_deactivated_consumer(bus: Arc<dyn EventBus>, pool: PgPool) {
    tokio::spawn(async move {
        let subject = "party.deactivated";
        let mut stream = match bus.subscribe(subject).await {
            Ok(s) => s,
            Err(e) => {
                tracing::error!(subject, error = %e, "cc: failed to subscribe to party.deactivated");
                return;
            }
        };
        tracing::info!(subject, "cc: subscribed to party.deactivated");

        while let Some(msg) = stream.next().await {
            if let Err(e) = process_party_deactivated_message(&pool, &msg).await {
                tracing::error!(error = %e, "cc: failed to process party.deactivated");
            }
        }

        tracing::warn!("cc: party.deactivated consumer stopped");
    });
}

async fn process_party_deactivated_message(
    pool: &PgPool,
    msg: &BusMessage,
) -> Result<(), Box<dyn std::error::Error>> {
    let envelope: EventEnvelope<PartyDeactivatedPayload> = serde_json::from_slice(&msg.payload)
        .map_err(|e| format!("Failed to parse party.deactivated envelope: {}", e))?;

    tracing::info!(
        event_id = %envelope.event_id,
        party_id = %envelope.payload.party_id,
        "cc: processing party.deactivated"
    );

    handle_party_deactivated(pool, envelope.event_id, &envelope.payload)
        .await
        .map_err(|e| format!("handle_party_deactivated failed: {}", e).into())
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
        format!("cc-pd-{}", Uuid::new_v4().simple())
    }

    async fn seed_complaint(pool: &PgPool, tenant_id: &str, party_id: Uuid) -> Uuid {
        let id: Uuid = sqlx::query_scalar(
            r#"INSERT INTO complaints
               (tenant_id, complaint_number, status, party_id, source, title, created_by)
               VALUES ($1, $2, 'intake', $3, 'email', 'Test complaint', 'system')
               RETURNING id"#,
        )
        .bind(tenant_id)
        .bind(format!("CC-{}", Uuid::new_v4().simple()))
        .bind(party_id)
        .fetch_one(pool)
        .await
        .expect("seed complaint failed");
        id
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
        sqlx::query(
            "DELETE FROM cc_processed_events WHERE event_id IN \
             (SELECT event_id FROM cc_processed_events WHERE processor = 'cc.party_deactivated')",
        )
        .execute(pool)
        .await
        .ok();
    }

    fn sample_payload(party_id: Uuid, app_id: &str) -> PartyDeactivatedPayload {
        PartyDeactivatedPayload {
            party_id,
            app_id: app_id.to_string(),
            deactivated_by: "admin".to_string(),
            deactivated_at: Utc::now(),
        }
    }

    #[tokio::test]
    #[serial]
    async fn test_flags_open_complaints_for_deactivated_party() {
        let pool = test_pool().await;
        let tid = unique_tenant();
        cleanup(&pool, &tid).await;

        let party_id = Uuid::new_v4();
        let complaint_id = seed_complaint(&pool, &tid, party_id).await;

        let event_id = Uuid::new_v4();
        let payload = sample_payload(party_id, &tid);
        handle_party_deactivated(&pool, event_id, &payload)
            .await
            .expect("handle failed");

        let (count,): (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM complaint_activity_log WHERE complaint_id = $1 AND activity_type = 'internal_communication'",
        )
        .bind(complaint_id)
        .fetch_one(&pool)
        .await
        .expect("count query failed");

        assert_eq!(count, 1, "Expected one warning activity log entry");

        cleanup(&pool, &tid).await;
    }

    #[tokio::test]
    #[serial]
    async fn test_idempotent_on_redelivery() {
        let pool = test_pool().await;
        let tid = unique_tenant();
        cleanup(&pool, &tid).await;

        let party_id = Uuid::new_v4();
        let _complaint_id = seed_complaint(&pool, &tid, party_id).await;

        let event_id = Uuid::new_v4();
        let payload = sample_payload(party_id, &tid);

        handle_party_deactivated(&pool, event_id, &payload)
            .await
            .expect("first handle failed");
        handle_party_deactivated(&pool, event_id, &payload)
            .await
            .expect("second handle must not error (idempotent)");

        let (count,): (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM complaint_activity_log WHERE tenant_id = $1 AND activity_type = 'internal_communication'",
        )
        .bind(&tid)
        .fetch_one(&pool)
        .await
        .expect("count query failed");

        assert_eq!(
            count, 1,
            "Redelivery must not duplicate activity log entries"
        );

        cleanup(&pool, &tid).await;
    }

    #[tokio::test]
    #[serial]
    async fn test_skips_closed_complaints() {
        let pool = test_pool().await;
        let tid = unique_tenant();
        cleanup(&pool, &tid).await;

        let party_id = Uuid::new_v4();

        // Insert a closed complaint
        sqlx::query(
            r#"INSERT INTO complaints
               (tenant_id, complaint_number, status, party_id, source, title, created_by)
               VALUES ($1, $2, 'closed', $3, 'email', 'Closed complaint', 'system')"#,
        )
        .bind(&tid)
        .bind(format!("CC-CLO-{}", Uuid::new_v4().simple()))
        .bind(party_id)
        .execute(&pool)
        .await
        .expect("seed closed complaint failed");

        let event_id = Uuid::new_v4();
        let payload = sample_payload(party_id, &tid);
        handle_party_deactivated(&pool, event_id, &payload)
            .await
            .expect("handle failed");

        let (count,): (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM complaint_activity_log WHERE tenant_id = $1")
                .bind(&tid)
                .fetch_one(&pool)
                .await
                .expect("count query failed");

        assert_eq!(
            count, 0,
            "Closed complaints must not receive activity log entries"
        );

        cleanup(&pool, &tid).await;
    }
}
