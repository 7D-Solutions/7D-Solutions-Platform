//! Atomic idempotency guards for the event registry.
//!
//! Maintains an `event_dedupe` table that records processed event IDs.
//! Before a handler runs, the registry calls [`check_and_mark`] within an
//! open database transaction.  If the `event_id` already exists the INSERT
//! is a no-op and the caller skips the handler.  If the INSERT succeeds the
//! handler proceeds; if the handler later fails the transaction is rolled
//! back — removing the dedup entry so the event can be retried.
//!
//! # Table layout
//!
//! ```sql
//! CREATE TABLE IF NOT EXISTS "event_dedupe" (
//!     event_id       UUID         PRIMARY KEY,
//!     processed_at   TIMESTAMPTZ  NOT NULL DEFAULT NOW()
//! );
//! ```
//!
//! # Usage
//!
//! ```rust,ignore
//! use platform_sdk::idempotency;
//!
//! // At startup
//! idempotency::ensure_dedupe_table(&pool, "event_dedupe").await?;
//!
//! // In dispatch (inside a transaction)
//! let mut tx = pool.begin().await?;
//! if idempotency::check_and_mark(&mut tx, "event_dedupe", env.event_id).await? {
//!     handler(ctx, env).await?;
//!     tx.commit().await?;
//! } else {
//!     // duplicate — skip silently
//! }
//! ```

use uuid::Uuid;

use crate::startup::StartupError;

/// DDL for the dedup table.
const DEDUPE_TABLE_DDL: &str = r#"
CREATE TABLE IF NOT EXISTS "{table}" (
    event_id       UUID         PRIMARY KEY,
    processed_at   TIMESTAMPTZ  NOT NULL DEFAULT NOW()
)
"#;

/// Canonical DDL for the event dedup table.
///
/// Uses `{table}` as a placeholder — substitute with [`str::replace`] before
/// executing, or use [`ensure_dedupe_table`] which handles this automatically.
pub const STANDARD_DEDUPE_DDL: &str = r#"
CREATE TABLE IF NOT EXISTS "{table}" (
    event_id       UUID         PRIMARY KEY,
    processed_at   TIMESTAMPTZ  NOT NULL DEFAULT NOW()
);
"#;

/// Create the dedup table if it does not already exist.
///
/// Safe to call multiple times — uses `IF NOT EXISTS`. Call once at module
/// startup alongside [`ensure_outbox_table`][crate::publisher::ensure_outbox_table].
pub async fn ensure_dedupe_table(
    pool: &sqlx::PgPool,
    table_name: &str,
) -> Result<(), StartupError> {
    let ddl = DEDUPE_TABLE_DDL.replace("{table}", table_name);
    sqlx::query(&ddl).execute(pool).await.map_err(|e| {
        StartupError::Database(format!("failed to create dedup table '{table_name}': {e}"))
    })?;
    tracing::info!(table = %table_name, "dedup table ensured");
    Ok(())
}

/// Atomically check whether `event_id` has been processed and mark it if not.
///
/// **Must be called within an already-open transaction.**  The INSERT and the
/// handler's DB mutations must share the same transaction so that a crash
/// between them leaves neither committed.
///
/// Returns `true` if this is a **new** event (INSERT succeeded — proceed with
/// the handler).  Returns `false` if this is a **duplicate** (INSERT
/// conflicted on the primary key — skip the handler silently).
pub async fn check_and_mark(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    table: &str,
    event_id: Uuid,
) -> Result<bool, sqlx::Error> {
    let q = format!(
        r#"INSERT INTO "{table}" (event_id, processed_at)
           VALUES ($1, NOW())
           ON CONFLICT (event_id) DO NOTHING"#
    );
    let result = sqlx::query(&q).bind(event_id).execute(&mut **tx).await?;
    Ok(result.rows_affected() == 1)
}
