use std::sync::Arc;
use std::time::Duration;

use event_bus::EventBus;
use sqlx::PgPool;
use uuid::Uuid;

use super::{fetch_unpublished, OutboxEvent};

const DEFAULT_BATCH_SIZE: i64 = 100;
pub const DEFAULT_MAX_RETRIES: i32 = 5;

pub async fn run_publisher_task(pool: PgPool, bus: Arc<dyn EventBus>) {
    tracing::info!("Integrations: starting outbox publisher task");

    let mut interval = tokio::time::interval(Duration::from_secs(1));
    loop {
        interval.tick().await;

        match publish_batch(&pool, &bus, DEFAULT_MAX_RETRIES).await {
            Ok(n) if n > 0 => tracing::info!("Integrations: published {} outbox events", n),
            Ok(_) => {}
            Err(e) => tracing::error!("Integrations: outbox relay error: {}", e),
        }
    }
}

pub async fn publish_batch(
    pool: &PgPool,
    bus: &Arc<dyn EventBus>,
    max_retries: i32,
) -> Result<usize, Box<dyn std::error::Error + Send + Sync>> {
    let mut tx = pool.begin().await?;
    let events = fetch_unpublished(&mut tx, DEFAULT_BATCH_SIZE).await?;

    let mut published_ids: Vec<Uuid> = Vec::new();

    for event in events {
        let payload = serde_json::to_vec(&event.payload)?;

        match bus.publish(&event.event_type, payload).await {
            Ok(()) => {
                published_ids.push(event.event_id);
            }
            Err(err) => {
                let err_text = err.to_string();
                let retry_count =
                    record_publish_failure(&mut tx, &event, &err_text, max_retries).await?;

                tracing::warn!(
                    event_id = %event.event_id,
                    event_type = %event.event_type,
                    retry_count,
                    error = %err_text,
                    "Integrations: failed to publish outbox event"
                );
            }
        }
    }

    let published_count = published_ids.len();

    // Batch-update all successfully published events in a single query
    if !published_ids.is_empty() {
        sqlx::query(
            r#"
            UPDATE integrations_outbox
            SET published_at = NOW(), error_message = NULL
            WHERE event_id = ANY($1)
            "#,
        )
        .bind(&published_ids)
        .execute(&mut *tx)
        .await?;
    }

    tx.commit().await?;
    Ok(published_count)
}

async fn record_publish_failure(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    event: &OutboxEvent,
    error: &str,
    max_retries: i32,
) -> Result<i32, sqlx::Error> {
    let (retry_count, failed): (i32, bool) = sqlx::query_as(
        r#"
        UPDATE integrations_outbox
        SET retry_count = retry_count + 1,
            error_message = $2,
            failed_at = CASE
                WHEN retry_count + 1 >= $3 THEN NOW()
                ELSE failed_at
            END,
            failure_reason = CASE
                WHEN retry_count + 1 >= $3 THEN 'retry_exhausted'
                ELSE 'bus_publish_failed'
            END
        WHERE event_id = $1
        RETURNING retry_count, failed_at IS NOT NULL
        "#,
    )
    .bind(event.event_id)
    .bind(error)
    .bind(max_retries)
    .fetch_one(&mut **tx)
    .await?;

    if failed {
        move_to_failed_events(tx, event, retry_count, error).await?;
    }

    Ok(retry_count)
}

async fn move_to_failed_events(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    event: &OutboxEvent,
    retry_count: i32,
    error: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        INSERT INTO failed_events (event_id, subject, tenant_id, envelope_json, error, retry_count)
        VALUES ($1, $2, $3, $4, $5, $6)
        ON CONFLICT (event_id) DO UPDATE
        SET subject = EXCLUDED.subject,
            tenant_id = EXCLUDED.tenant_id,
            envelope_json = EXCLUDED.envelope_json,
            error = EXCLUDED.error,
            retry_count = EXCLUDED.retry_count,
            failed_at = NOW()
        "#,
    )
    .bind(event.event_id)
    .bind(&event.event_type)
    .bind(&event.app_id)
    .bind(&event.payload)
    .bind(error)
    .bind(retry_count)
    .execute(&mut **tx)
    .await?;

    Ok(())
}

#[allow(dead_code)]
async fn _clear_failed_event(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    event_id: Uuid,
) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM failed_events WHERE event_id = $1")
        .bind(event_id)
        .execute(&mut **tx)
        .await?;
    Ok(())
}
