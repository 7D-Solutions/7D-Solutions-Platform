//! Repository layer for webhook ingest persistence.

use chrono::{DateTime, Utc};

/// Insert a webhook ingest record with dedup constraint.
/// Returns `Some(id)` if inserted, `None` if duplicate.
pub async fn insert_ingest(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    app_id: &str,
    system: &str,
    event_type: &Option<String>,
    raw_payload: &serde_json::Value,
    headers: &serde_json::Value,
    received_at: DateTime<Utc>,
    idempotency_key: &Option<String>,
) -> Result<Option<(i64,)>, sqlx::Error> {
    sqlx::query_as::<_, (i64,)>(
        r#"
        INSERT INTO integrations_webhook_ingest
            (app_id, system, event_type, raw_payload, headers, received_at, idempotency_key)
        VALUES ($1, $2, $3, $4, $5, $6, $7)
        ON CONFLICT ON CONSTRAINT integrations_webhook_ingest_dedup DO NOTHING
        RETURNING id
        "#,
    )
    .bind(app_id)
    .bind(system)
    .bind(event_type)
    .bind(raw_payload)
    .bind(headers)
    .bind(received_at)
    .bind(idempotency_key)
    .fetch_optional(&mut **tx)
    .await
}

/// Look up an existing ingest record by dedup key.
pub async fn lookup_existing_ingest(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    app_id: &str,
    system: &str,
    idempotency_key: &Option<String>,
) -> Result<Option<(i64,)>, sqlx::Error> {
    sqlx::query_as::<_, (i64,)>(
        "SELECT id FROM integrations_webhook_ingest
         WHERE app_id = $1 AND system = $2 AND idempotency_key = $3",
    )
    .bind(app_id)
    .bind(system)
    .bind(idempotency_key)
    .fetch_optional(&mut **tx)
    .await
}

/// Mark an ingest record as processed.
pub async fn mark_ingest_processed(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    ingest_id: i64,
    processed_at: DateTime<Utc>,
) -> Result<sqlx::postgres::PgQueryResult, sqlx::Error> {
    sqlx::query("UPDATE integrations_webhook_ingest SET processed_at = $1 WHERE id = $2")
        .bind(processed_at)
        .bind(ingest_id)
        .execute(&mut **tx)
        .await
}

// -- QBO normalizer queries --

/// Insert a batch-level ingest record for QBO webhook POST.
/// Returns `Some(id)` if inserted, `None` if duplicate body hash.
pub async fn insert_batch_ingest(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    raw_payload: &serde_json::Value,
    headers: &serde_json::Value,
    received_at: DateTime<Utc>,
    body_hash: &str,
) -> Result<Option<(i64,)>, sqlx::Error> {
    sqlx::query_as::<_, (i64,)>(
        r#"
        INSERT INTO integrations_webhook_ingest
            (app_id, system, event_type, raw_payload, headers, received_at, idempotency_key)
        VALUES ('_qbo_batch_', 'quickbooks', NULL, $1, $2, $3, $4)
        ON CONFLICT ON CONSTRAINT integrations_webhook_ingest_dedup DO NOTHING
        RETURNING id
        "#,
    )
    .bind(raw_payload)
    .bind(headers)
    .bind(received_at)
    .bind(body_hash)
    .fetch_optional(&mut **tx)
    .await
}

/// Batch-resolve QBO realm_ids to app_ids via connected OAuth records.
pub async fn batch_resolve_realms(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    realm_ids: &[String],
) -> Result<Vec<(String, String)>, sqlx::Error> {
    sqlx::query_as::<_, (String, String)>(
        r#"
        SELECT realm_id, app_id FROM integrations_oauth_connections
        WHERE provider = 'quickbooks' AND realm_id = ANY($1) AND connection_status = 'connected'
        "#,
    )
    .bind(realm_ids)
    .fetch_all(&mut **tx)
    .await
}

/// Insert a per-event ingest record for a single QBO CloudEvent.
/// Returns `Some(id)` if inserted, `None` if duplicate event ID.
pub async fn insert_event_ingest(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    app_id: &str,
    event_type: &str,
    event_payload: &serde_json::Value,
    received_at: DateTime<Utc>,
    event_id: &str,
) -> Result<Option<(i64,)>, sqlx::Error> {
    sqlx::query_as::<_, (i64,)>(
        r#"
        INSERT INTO integrations_webhook_ingest
            (app_id, system, event_type, raw_payload, headers, received_at, idempotency_key)
        VALUES ($1, 'quickbooks', $2, $3, '{}'::jsonb, $4, $5)
        ON CONFLICT ON CONSTRAINT integrations_webhook_ingest_dedup DO NOTHING
        RETURNING id
        "#,
    )
    .bind(app_id)
    .bind(event_type)
    .bind(event_payload)
    .bind(received_at)
    .bind(event_id)
    .fetch_optional(&mut **tx)
    .await
}
