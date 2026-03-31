//! Generic outbox publisher for the platform SDK.
//!
//! Polls a declared outbox table for unpublished events and publishes
//! them to the event bus. This eliminates the per-module publisher
//! boilerplate that 9 of 25 modules needed (and some forgot).

use std::sync::Arc;
use std::time::Duration;

use event_bus::EventBus;
use sqlx::Row;

use crate::startup::StartupError;

/// Query `information_schema.tables` for tables matching outbox patterns.
///
/// Returns `Some(table_name)` if an undeclared outbox table is found
/// (tables ending in `_outbox` in the `public` schema).
pub(crate) async fn detect_outbox_table(
    pool: &sqlx::PgPool,
) -> Result<Option<String>, StartupError> {
    let row = sqlx::query(
        r#"
        SELECT table_name::text
        FROM information_schema.tables
        WHERE table_schema = 'public'
          AND table_name LIKE '%\_outbox' ESCAPE '\'
        LIMIT 1
        "#,
    )
    .fetch_optional(pool)
    .await
    .map_err(|e| StartupError::Database(format!("outbox detection query failed: {e}")))?;

    Ok(row.map(|r| r.get::<String, _>("table_name")))
}

/// Generic outbox publisher loop — polls the declared outbox table and
/// publishes pending events to the bus.
///
/// This is the centralized publisher that modules no longer need to
/// implement individually. The SDK spawns it automatically when
/// `[events.publish].outbox_table` is declared in the manifest.
pub async fn run_outbox_publisher(
    pool: sqlx::PgPool,
    bus: Arc<dyn EventBus>,
    outbox_table: &str,
    module_name: &str,
) {
    tracing::info!(module = %module_name, table = %outbox_table, "outbox publisher started");

    let mut interval = tokio::time::interval(Duration::from_secs(1));
    let mut tick: u64 = 0;

    loop {
        interval.tick().await;
        tick += 1;

        match publish_batch(&pool, &bus, outbox_table).await {
            Ok(n) if n > 0 => {
                tracing::info!(
                    module = %module_name, tick, published = n,
                    "outbox publisher: events published"
                );
            }
            Ok(_) => {
                if tick <= 3 || tick % 60 == 0 {
                    tracing::debug!(
                        module = %module_name, tick,
                        "outbox publisher: no pending events"
                    );
                }
            }
            Err(e) => {
                tracing::error!(
                    module = %module_name, tick, error = %e,
                    "outbox publisher: error"
                );
            }
        }
    }
}

async fn publish_batch(
    pool: &sqlx::PgPool,
    bus: &Arc<dyn EventBus>,
    outbox_table: &str,
) -> Result<usize, Box<dyn std::error::Error + Send + Sync>> {
    let select = format!(
        "SELECT event_id, event_type, payload FROM \"{}\" \
         WHERE published_at IS NULL ORDER BY created_at ASC LIMIT 100",
        outbox_table
    );

    let rows = sqlx::query(&select).fetch_all(pool).await?;
    let count = rows.len();

    for row in &rows {
        let event_id: uuid::Uuid = row.get("event_id");
        let event_type: String = row.get("event_type");
        let payload: serde_json::Value = row.get("payload");

        let bytes = serde_json::to_vec(&payload)?;
        bus.publish(&event_type, bytes).await.map_err(|e| {
            tracing::error!(
                event_id = %event_id, event_type = %event_type, error = %e,
                "publish failed"
            );
            e
        })?;

        let update = format!(
            "UPDATE \"{}\" SET published_at = NOW() WHERE event_id = $1",
            outbox_table
        );
        sqlx::query(&update).bind(event_id).execute(pool).await?;

        tracing::debug!(event_id = %event_id, event_type = %event_type, "event published");
    }

    Ok(count)
}
