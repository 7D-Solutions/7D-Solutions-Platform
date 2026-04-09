//! Dead-letter queue (DLQ) for the event registry.
//!
//! When a handler registered with [`EventRegistry`] returns an error,
//! the failed event is written to the DLQ table with the error message,
//! retry count, and original payload. The table is queryable and entries
//! are replayable via [`replay_dlq_entry`].
//!
//! # Transaction isolation
//!
//! DLQ writes always run in a **separate** transaction from the handler.
//! After a handler fails its own work and rolls back, the DLQ write acquires
//! a fresh connection from the pool and commits independently. This guarantee
//! holds regardless of which pool the handler used internally.
//!
//! # Usage
//!
//! ```rust,ignore
//! use platform_sdk::dlq;
//!
//! // At startup — creates the table if absent
//! dlq::ensure_dlq_table(&pool, "event_dlq").await?;
//!
//! // Dispatch with automatic DLQ capture on failure
//! registry.dispatch_with_dlq(ctx, env, "event_dlq").await?;
//!
//! // Later — replay a failed event
//! if let Some(env) = dlq::replay_dlq_entry(&pool, "event_dlq", entry_id).await? {
//!     registry.dispatch(ctx, env).await?;
//! }
//! ```

use chrono::{DateTime, Utc};
use serde_json::Value;
use uuid::Uuid;

use crate::startup::StartupError;

/// Canonical DDL for the DLQ table.
///
/// Uses `{table}` as a placeholder — substitute with [`str::replace`]
/// before executing, or use [`ensure_dlq_table`] which handles this.
pub const STANDARD_DLQ_DDL: &str = r#"
CREATE TABLE IF NOT EXISTS "{table}" (
    id             BIGSERIAL    PRIMARY KEY,
    event_id       UUID         NOT NULL,
    event_type     TEXT         NOT NULL,
    schema_version TEXT         NOT NULL,
    tenant_id      TEXT         NOT NULL DEFAULT '',
    payload        JSONB        NOT NULL,
    error_message  TEXT         NOT NULL,
    retry_count    INTEGER      NOT NULL DEFAULT 0,
    failed_at      TIMESTAMPTZ  NOT NULL DEFAULT NOW(),
    replayed_at    TIMESTAMPTZ
);

CREATE INDEX IF NOT EXISTS "idx_{table}_pending"
    ON "{table}" (failed_at) WHERE replayed_at IS NULL;
"#;

/// A row fetched from the DLQ table.
#[derive(Debug, Clone)]
pub struct DlqEntry {
    /// Row primary key — use this as the `id` argument to [`replay_dlq_entry`].
    pub id: i64,
    /// Original event identifier from the [`EventEnvelope`].
    pub event_id: Uuid,
    /// Event type string (e.g. `"invoice.opened"`).
    pub event_type: String,
    /// Schema version string (e.g. `"1.0.0"`).
    pub schema_version: String,
    /// Tenant the event belonged to.
    pub tenant_id: String,
    /// Raw event payload as stored in the envelope.
    pub payload: Value,
    /// Error message from the handler that failed.
    pub error_message: String,
    /// Number of times this entry has been retried (starts at 0).
    pub retry_count: i32,
    /// When the entry was first written.
    pub failed_at: DateTime<Utc>,
    /// When the entry was successfully replayed, or `None` if still pending.
    pub replayed_at: Option<DateTime<Utc>>,
}

/// DDL for just the DLQ table itself (no indexes).
const DLQ_TABLE_DDL: &str = r#"
CREATE TABLE IF NOT EXISTS "{table}" (
    id             BIGSERIAL    PRIMARY KEY,
    event_id       UUID         NOT NULL,
    event_type     TEXT         NOT NULL,
    schema_version TEXT         NOT NULL,
    tenant_id      TEXT         NOT NULL DEFAULT '',
    payload        JSONB        NOT NULL,
    error_message  TEXT         NOT NULL,
    retry_count    INTEGER      NOT NULL DEFAULT 0,
    failed_at      TIMESTAMPTZ  NOT NULL DEFAULT NOW(),
    replayed_at    TIMESTAMPTZ
)
"#;

/// DDL for the pending-events index only.
const DLQ_INDEX_DDL: &str = r#"
CREATE INDEX IF NOT EXISTS "idx_{table}_pending"
    ON "{table}" (failed_at) WHERE replayed_at IS NULL
"#;

/// Create the DLQ table if it does not already exist.
///
/// Safe to call multiple times — uses `IF NOT EXISTS`. Typically called
/// once at module startup alongside [`ensure_outbox_table`][crate::publisher::ensure_outbox_table].
///
/// Each DDL statement is executed separately to avoid the PostgreSQL
/// "cannot insert multiple commands into a prepared statement" restriction.
pub async fn ensure_dlq_table(
    pool: &sqlx::PgPool,
    table_name: &str,
) -> Result<(), StartupError> {
    for template in [DLQ_TABLE_DDL, DLQ_INDEX_DDL] {
        let ddl = template.replace("{table}", table_name);
        sqlx::query(&ddl)
            .execute(pool)
            .await
            .map_err(|e| {
                StartupError::Database(format!(
                    "failed to create DLQ table '{table_name}': {e}"
                ))
            })?;
    }
    tracing::info!(table = %table_name, "DLQ table ensured");
    Ok(())
}

/// Write a failed event to the DLQ.
///
/// Acquires a fresh connection from `pool` — always in a separate transaction
/// from whatever the handler was doing. Returns the new row's `id`.
pub async fn write_dlq_entry(
    pool: &sqlx::PgPool,
    table: &str,
    event_id: Uuid,
    event_type: &str,
    schema_version: &str,
    tenant_id: &str,
    payload: &Value,
    error_message: &str,
    retry_count: i32,
) -> Result<i64, sqlx::Error> {
    let q = format!(
        r#"INSERT INTO "{table}"
               (event_id, event_type, schema_version, tenant_id,
                payload, error_message, retry_count)
           VALUES ($1, $2, $3, $4, $5, $6, $7)
           RETURNING id"#
    );
    let row: (i64,) = sqlx::query_as(&q)
        .bind(event_id)
        .bind(event_type)
        .bind(schema_version)
        .bind(tenant_id)
        .bind(payload)
        .bind(error_message)
        .bind(retry_count)
        .fetch_one(pool)
        .await?;
    Ok(row.0)
}

/// List entries from the DLQ table, ordered by `failed_at` ascending.
///
/// When `include_replayed` is `false` (the default for most callers),
/// only entries where `replayed_at IS NULL` are returned.
pub async fn list_dlq_entries(
    pool: &sqlx::PgPool,
    table: &str,
    limit: i64,
    include_replayed: bool,
) -> Result<Vec<DlqEntry>, sqlx::Error> {
    let where_clause = if include_replayed {
        String::new()
    } else {
        "WHERE replayed_at IS NULL".to_string()
    };
    let q = format!(
        r#"SELECT id, event_id, event_type, schema_version, tenant_id, payload,
                  error_message, retry_count, failed_at, replayed_at
           FROM "{table}"
           {where_clause}
           ORDER BY failed_at ASC
           LIMIT $1"#
    );
    let rows = sqlx::query(&q).bind(limit).fetch_all(pool).await?;

    let mut entries = Vec::with_capacity(rows.len());
    for row in rows {
        use sqlx::Row;
        entries.push(DlqEntry {
            id: row.get("id"),
            event_id: row.get("event_id"),
            event_type: row.get("event_type"),
            schema_version: row.get("schema_version"),
            tenant_id: row.get("tenant_id"),
            payload: row.get("payload"),
            error_message: row.get("error_message"),
            retry_count: row.get("retry_count"),
            failed_at: row.get("failed_at"),
            replayed_at: row.get("replayed_at"),
        });
    }
    Ok(entries)
}

/// Fetch a DLQ entry by its `id` and return a reconstructed
/// [`EventEnvelope`][event_bus::EventEnvelope] ready for re-dispatch.
///
/// On success the entry's `replayed_at` is set to `NOW()`. The caller is
/// responsible for dispatching the returned envelope — typically via
/// [`EventRegistry::dispatch`][crate::event_registry::EventRegistry::dispatch].
///
/// Returns `None` if no row with `id` exists.
pub async fn replay_dlq_entry(
    pool: &sqlx::PgPool,
    table: &str,
    id: i64,
) -> Result<Option<event_bus::EventEnvelope<serde_json::Value>>, sqlx::Error> {
    let select = format!(
        r#"SELECT event_id, event_type, schema_version, tenant_id, payload
           FROM "{table}"
           WHERE id = $1"#
    );
    let row_opt = sqlx::query(&select)
        .bind(id)
        .fetch_optional(pool)
        .await?;

    let row = match row_opt {
        Some(r) => r,
        None => return Ok(None),
    };

    use sqlx::Row;
    let event_id: Uuid = row.get("event_id");
    let event_type: String = row.get("event_type");
    let schema_version: String = row.get("schema_version");
    let tenant_id: String = row.get("tenant_id");
    let payload: serde_json::Value = row.get("payload");

    // Mark replayed before returning — if dispatch fails again the caller
    // may call replay again (retry_count will be incremented externally).
    let update = format!(r#"UPDATE "{table}" SET replayed_at = NOW() WHERE id = $1"#);
    sqlx::query(&update).bind(id).execute(pool).await?;

    let mut env = event_bus::EventEnvelope::new(
        tenant_id,
        String::new(), // source_module not stored in DLQ
        event_type,
        payload,
    );
    // Restore the original event identity so idempotency keys remain stable.
    env.event_id = event_id;
    env.schema_version = schema_version;

    Ok(Some(env))
}
