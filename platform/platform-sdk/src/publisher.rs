//! Generic outbox publisher for the platform SDK.
//!
//! Polls a declared outbox table for unpublished events and publishes
//! them to the event bus. This eliminates the per-module publisher
//! boilerplate that 9 of 25 modules needed (and some forgot).

use std::sync::Arc;
use std::time::Duration;

use event_bus::EventBus;
use sqlx::Row;
use tokio::sync::watch;

use crate::startup::StartupError;

/// Query `information_schema.tables` for tables matching outbox patterns.
///
/// Returns `Some(table_name)` if an undeclared outbox table is found
/// (tables ending in `_outbox` in the `public` schema).
pub async fn detect_outbox_table(
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

/// Multi-tenant outbox publisher loop — iterates all tenant pools and
/// publishes pending events from each tenant's outbox table.
///
/// Spawned automatically when a [`TenantPoolResolver`] is registered
/// and the manifest declares `[events.publish].outbox_table`.
///
/// The `shutdown_rx` watch channel is checked after each interval tick.
/// When `true` is received the current batch completes and the loop exits.
pub async fn run_multi_tenant_outbox_publisher(
    resolver: Arc<dyn crate::context::TenantPoolResolver>,
    bus: Arc<dyn EventBus>,
    outbox_table: &str,
    module_name: &str,
    subject_prefix: Option<&str>,
    mut shutdown_rx: watch::Receiver<bool>,
) {
    tracing::info!(
        module = %module_name, table = %outbox_table,
        subject_prefix = ?subject_prefix,
        "multi-tenant outbox publisher started"
    );

    let mut interval = tokio::time::interval(Duration::from_secs(1));
    let mut tick: u64 = 0;

    loop {
        tokio::select! {
            biased;
            _ = shutdown_rx.changed() => {
                if *shutdown_rx.borrow() {
                    break;
                }
            }
            _ = interval.tick() => {}
        }

        tick += 1;

        let pools = match resolver.all_pools().await {
            Ok(p) => p,
            Err(e) => {
                tracing::error!(
                    module = %module_name, tick, error = %e,
                    "multi-tenant outbox publisher: failed to list tenant pools"
                );
                continue;
            }
        };

        for (tenant_id, pool) in &pools {
            match publish_batch(pool, &bus, outbox_table, subject_prefix).await {
                Ok(n) if n > 0 => {
                    tracing::info!(
                        module = %module_name, tick, published = n,
                        tenant = %tenant_id,
                        "multi-tenant outbox publisher: events published"
                    );
                }
                Ok(_) => {}
                Err(e) => {
                    tracing::error!(
                        module = %module_name, tick, error = %e,
                        tenant = %tenant_id,
                        "multi-tenant outbox publisher: error"
                    );
                }
            }
        }

        if tick <= 3 || tick % 60 == 0 {
            tracing::debug!(
                module = %module_name, tick,
                tenant_count = pools.len(),
                "multi-tenant outbox publisher: poll cycle complete"
            );
        }
    }

    tracing::info!(module = %module_name, "multi-tenant outbox publisher stopped");
}

/// Generic outbox publisher loop — polls the declared outbox table and
/// publishes pending events to the bus.
///
/// This is the centralized publisher that modules no longer need to
/// implement individually. The SDK spawns it automatically when
/// `[events.publish].outbox_table` is declared in the manifest.
///
/// The `shutdown_rx` watch channel is checked after each interval tick.
/// When `true` is received the current batch completes and the loop exits.
pub async fn run_outbox_publisher(
    pool: sqlx::PgPool,
    bus: Arc<dyn EventBus>,
    outbox_table: &str,
    module_name: &str,
    subject_prefix: Option<&str>,
    mut shutdown_rx: watch::Receiver<bool>,
) {
    tracing::info!(
        module = %module_name, table = %outbox_table,
        subject_prefix = ?subject_prefix,
        "outbox publisher started"
    );

    let mut interval = tokio::time::interval(Duration::from_secs(1));
    let mut tick: u64 = 0;

    loop {
        tokio::select! {
            biased;
            _ = shutdown_rx.changed() => {
                if *shutdown_rx.borrow() {
                    break;
                }
            }
            _ = interval.tick() => {}
        }

        tick += 1;

        match publish_batch(&pool, &bus, outbox_table, subject_prefix).await {
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

    tracing::info!(module = %module_name, "outbox publisher stopped");
}

async fn publish_batch(
    pool: &sqlx::PgPool,
    bus: &Arc<dyn EventBus>,
    outbox_table: &str,
    subject_prefix: Option<&str>,
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

        let subject = match subject_prefix {
            Some(prefix) => format!("{}.{}", prefix, event_type),
            None => event_type.clone(),
        };

        let bytes = serde_json::to_vec(&payload)?;
        bus.publish(&subject, bytes).await.map_err(|e| {
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

        tracing::debug!(event_id = %event_id, subject = %subject, "event published");
    }

    Ok(count)
}
