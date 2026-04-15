//! Webhook repository — all SQL operations for webhook record management.

use serde_json::Value as JsonValue;
use sqlx::PgExecutor;

use crate::models::Webhook;

// ============================================================================
// Reads
// ============================================================================

/// Check for duplicate event (idempotency).
pub async fn check_duplicate_event<'e>(
    executor: impl PgExecutor<'e>,
    event_id: &str,
    app_id: &str,
) -> Result<Option<i32>, sqlx::Error> {
    sqlx::query_scalar::<_, i32>(
        r#"
        SELECT id FROM ar_webhooks
        WHERE event_id = $1 AND app_id = $2
        "#,
    )
    .bind(event_id)
    .bind(app_id)
    .fetch_optional(executor)
    .await
}

/// Fetch a webhook by ID with tenant isolation.
pub async fn fetch_by_id<'e>(
    executor: impl PgExecutor<'e>,
    id: i32,
    app_id: &str,
) -> Result<Option<Webhook>, sqlx::Error> {
    sqlx::query_as::<_, Webhook>(
        r#"
        SELECT
            id, app_id, event_id, event_type, status, error, payload,
            attempt_count, last_attempt_at, next_attempt_at, dead_at,
            error_code, received_at, processed_at
        FROM ar_webhooks
        WHERE id = $1 AND app_id = $2
        "#,
    )
    .bind(id)
    .bind(app_id)
    .fetch_optional(executor)
    .await
}

/// Count webhooks matching filters.
pub async fn count_webhooks<'e>(
    executor: impl PgExecutor<'e>,
    app_id: &str,
    event_type: Option<&str>,
    status: Option<&str>,
) -> Result<i64, sqlx::Error> {
    let mut sql = String::from("SELECT COUNT(*) as total FROM ar_webhooks WHERE app_id = $1");
    let mut param = 1;

    if event_type.is_some() {
        param += 1;
        sql.push_str(&format!(" AND event_type = ${param}"));
    }
    if status.is_some() {
        param += 1;
        sql.push_str(&format!(" AND status = ${param}::ar_webhooks_status"));
    }

    let mut q = sqlx::query_scalar::<_, i64>(&sql).bind(app_id);
    if let Some(et) = event_type {
        q = q.bind(et);
    }
    if let Some(st) = status {
        q = q.bind(st);
    }
    q.fetch_one(executor).await
}

/// List webhooks with optional filters and pagination.
pub async fn list_webhooks<'e>(
    executor: impl PgExecutor<'e>,
    app_id: &str,
    event_type: Option<&str>,
    status: Option<&str>,
    limit: i32,
    offset: i32,
) -> Result<Vec<Webhook>, sqlx::Error> {
    let mut sql = String::from(
        r#"
        SELECT
            id, app_id, event_id, event_type, status, error, payload,
            attempt_count, last_attempt_at, next_attempt_at, dead_at,
            error_code, received_at, processed_at
        FROM ar_webhooks
        WHERE app_id = $1
        "#,
    );

    let mut param = 1;

    if event_type.is_some() {
        param += 1;
        sql.push_str(&format!(" AND event_type = ${param}"));
    }

    if status.is_some() {
        param += 1;
        sql.push_str(&format!(" AND status = ${param}::ar_webhooks_status"));
    }

    sql.push_str(" ORDER BY received_at DESC LIMIT $");
    param += 1;
    sql.push_str(&param.to_string());
    sql.push_str(" OFFSET $");
    param += 1;
    sql.push_str(&param.to_string());

    let mut q = sqlx::query_as::<_, Webhook>(&sql).bind(app_id);

    if let Some(et) = event_type {
        q = q.bind(et);
    }
    if let Some(st) = status {
        q = q.bind(st);
    }

    q.bind(limit).bind(offset).fetch_all(executor).await
}

// ============================================================================
// Writes
// ============================================================================

/// Store a new webhook in the database (status: received).
pub async fn insert_webhook<'e>(
    executor: impl PgExecutor<'e>,
    app_id: &str,
    event_id: &str,
    event_type: &str,
    payload: JsonValue,
) -> Result<i32, sqlx::Error> {
    sqlx::query_scalar::<_, i32>(
        r#"
        INSERT INTO ar_webhooks (
            app_id, event_id, event_type, status, payload, attempt_count, received_at
        )
        VALUES ($1, $2, $3, 'received', $4, 1, NOW())
        RETURNING id
        "#,
    )
    .bind(app_id)
    .bind(event_id)
    .bind(event_type)
    .bind(payload)
    .fetch_one(executor)
    .await
}

/// Set webhook status to processing.
pub async fn set_processing<'e>(executor: impl PgExecutor<'e>, id: i32) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        UPDATE ar_webhooks
        SET status = 'processing', last_attempt_at = NOW()
        WHERE id = $1
        "#,
    )
    .bind(id)
    .execute(executor)
    .await?;
    Ok(())
}

/// Set webhook status to processing with incremented attempt count (replay).
pub async fn set_replay_processing<'e>(
    executor: impl PgExecutor<'e>,
    id: i32,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        UPDATE ar_webhooks
        SET status = 'processing', last_attempt_at = NOW(), attempt_count = attempt_count + 1
        WHERE id = $1
        "#,
    )
    .bind(id)
    .execute(executor)
    .await?;
    Ok(())
}

/// Set webhook status to processed.
pub async fn set_processed<'e>(executor: impl PgExecutor<'e>, id: i32) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        UPDATE ar_webhooks
        SET status = 'processed', processed_at = NOW()
        WHERE id = $1
        "#,
    )
    .bind(id)
    .execute(executor)
    .await
    .ok();
    Ok(())
}

/// Set webhook as processed with error cleared (replay success).
pub async fn set_replay_processed<'e>(
    executor: impl PgExecutor<'e>,
    id: i32,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        UPDATE ar_webhooks
        SET status = 'processed', processed_at = NOW(), error = NULL, error_code = NULL
        WHERE id = $1
        "#,
    )
    .bind(id)
    .execute(executor)
    .await
    .ok();
    Ok(())
}

/// Set webhook status to failed with error details.
pub async fn set_failed<'e>(
    executor: impl PgExecutor<'e>,
    id: i32,
    error: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        UPDATE ar_webhooks
        SET status = 'failed', error = $1, error_code = 'processing_error'
        WHERE id = $2
        "#,
    )
    .bind(error)
    .bind(id)
    .execute(executor)
    .await
    .ok();
    Ok(())
}
