//! Generic outbox publisher for the platform SDK.
//!
//! Polls a declared outbox table for unpublished events and publishes
//! them to the event bus. This eliminates the per-module publisher
//! boilerplate that 9 of 25 modules needed (and some forgot).
//!
//! ## Standard outbox table
//!
//! [`STANDARD_OUTBOX_DDL`] is the canonical DDL for the outbox table.
//! Modules that set `auto_create = true` in `[events.publish]` get this
//! table created automatically at startup via [`ensure_outbox_table`].
//! Modules with custom migrations can reference the constant instead.

use std::sync::Arc;
use std::time::Duration;

use event_bus::EventBus;
use sqlx::Row;
use tokio::sync::watch;

use crate::startup::StartupError;

/// Canonical DDL for the events outbox table.
///
/// This is the standard schema that the SDK publisher expects.  Modules
/// can either set `auto_create = true` in their manifest (which runs
/// this DDL at startup) or copy this SQL into their own migration file.
///
/// Columns beyond what the publisher polls (`event_id`, `event_type`,
/// `payload`, `created_at`, `published_at`) are included because they
/// appear in the majority of existing module outbox tables and align
/// with the platform `EventEnvelope` fields.
pub const STANDARD_OUTBOX_DDL: &str = r#"
CREATE TABLE IF NOT EXISTS "{table}" (
    id             BIGSERIAL    PRIMARY KEY,
    event_id       UUID         NOT NULL UNIQUE,
    event_type     TEXT         NOT NULL,
    aggregate_type TEXT         NOT NULL DEFAULT '',
    aggregate_id   TEXT         NOT NULL DEFAULT '',
    tenant_id      TEXT         NOT NULL DEFAULT '',
    payload        JSONB        NOT NULL,
    created_at     TIMESTAMPTZ  NOT NULL DEFAULT NOW(),
    published_at   TIMESTAMPTZ
);

CREATE INDEX IF NOT EXISTS "idx_{table}_unpublished"
    ON "{table}" (created_at) WHERE published_at IS NULL;

CREATE INDEX IF NOT EXISTS "idx_{table}_published"
    ON "{table}" (published_at) WHERE published_at IS NOT NULL;
"#;

/// Create the standard outbox table if it does not already exist.
///
/// Called automatically during startup when `[events.publish].auto_create`
/// is `true`. Safe to call multiple times — uses `IF NOT EXISTS`.
pub async fn ensure_outbox_table(
    pool: &sqlx::PgPool,
    table_name: &str,
) -> Result<(), StartupError> {
    let ddl = STANDARD_OUTBOX_DDL.replace("{table}", table_name);
    sqlx::query(&ddl).execute(pool).await.map_err(|e| {
        StartupError::Database(format!("failed to create outbox table '{table_name}': {e}"))
    })?;
    tracing::info!(table = %table_name, "outbox table ensured");
    Ok(())
}

/// Query `information_schema.tables` for tables matching outbox patterns.
///
/// Returns `Some(table_name)` if an undeclared outbox table is found
/// (tables ending in `_outbox` in the `public` schema).
pub async fn detect_outbox_table(pool: &sqlx::PgPool) -> Result<Option<String>, StartupError> {
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

/// Publish a batch of outbox events to the bus and mark them published.
///
/// Uses exactly two pool connections regardless of batch size:
/// 1. SELECT — fetch unpublished events, connection released immediately.
/// 2. Batch UPDATE — mark all successfully published events in one query.
///
/// NATS publishes happen between the two DB calls, so no connection is
/// held while waiting for the bus.  This prevents pool starvation when
/// the bus is slow or the HTTP handler concurrency is high.
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

    // Phase 1: fetch batch — acquire one connection and release it immediately.
    let rows = sqlx::query(&select).fetch_all(pool).await?;
    if rows.is_empty() {
        return Ok(0);
    }

    // Phase 2: publish to bus — no DB connection held during network I/O.
    let mut published_ids: Vec<uuid::Uuid> = Vec::with_capacity(rows.len());
    for row in &rows {
        let event_id: uuid::Uuid = row.get("event_id");
        let event_type: String = row.get("event_type");
        let payload: serde_json::Value = row.get("payload");

        let subject = match subject_prefix {
            Some(prefix) => format!("{}.{}", prefix, event_type),
            None => event_type.clone(),
        };

        let bytes = serde_json::to_vec(&payload)?;
        match bus.publish(&subject, bytes).await {
            Ok(_) => {
                tracing::debug!(event_id = %event_id, subject = %subject, "event published");
                published_ids.push(event_id);
            }
            Err(e) => {
                tracing::error!(
                    event_id = %event_id, event_type = %event_type, error = %e,
                    "publish failed — will retry next tick"
                );
                // Continue; failed event is retried on the next tick.
            }
        }
    }

    if published_ids.is_empty() {
        return Ok(0);
    }

    // Phase 3: batch UPDATE — acquire one connection and release it immediately.
    let update = format!(
        "UPDATE \"{}\" SET published_at = NOW() \
         WHERE event_id = ANY($1) AND published_at IS NULL",
        outbox_table
    );
    sqlx::query(&update)
        .bind(&published_ids)
        .execute(pool)
        .await?;

    Ok(published_ids.len())
}
