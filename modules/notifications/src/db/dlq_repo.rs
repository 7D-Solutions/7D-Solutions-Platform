//! Repository for DLQ (dead-letter queue) queries and mutations.

use chrono::{DateTime, Utc};
use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

#[derive(sqlx::FromRow)]
pub struct DlqRow {
    pub id: Uuid,
    pub recipient_ref: String,
    pub channel: String,
    pub template_key: String,
    pub payload_json: serde_json::Value,
    pub retry_count: i32,
    pub last_error: Option<String>,
    pub dead_lettered_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

#[derive(sqlx::FromRow)]
pub struct AttemptRow {
    pub id: Uuid,
    pub attempt_no: i32,
    pub status: String,
    pub provider_message_id: Option<String>,
    pub error_class: Option<String>,
    pub error_message: Option<String>,
    pub rendered_subject: Option<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(sqlx::FromRow)]
pub struct StatusOnly {
    pub status: String,
}

pub async fn count_dead_lettered(pool: &PgPool, tenant_id: &str) -> Result<i64, sqlx::Error> {
    let (count,): (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM scheduled_notifications \
         WHERE status = 'dead_lettered' AND tenant_id = $1",
    )
    .bind(tenant_id)
    .fetch_one(pool)
    .await?;
    Ok(count)
}

pub async fn list_dead_lettered(
    pool: &PgPool,
    tenant_id: &str,
    channel: Option<&str>,
    template_key: Option<&str>,
    limit: i64,
    offset: i64,
) -> Result<Vec<DlqRow>, sqlx::Error> {
    let mut bind_idx = 1u32;
    let mut binds: Vec<String> = vec![tenant_id.to_string()];

    let mut query = String::from(
        "SELECT id, recipient_ref, channel, template_key, payload_json, \
         retry_count, last_error, dead_lettered_at, created_at \
         FROM scheduled_notifications WHERE status = 'dead_lettered' AND tenant_id = $1",
    );

    if let Some(ch) = channel {
        bind_idx += 1;
        query.push_str(&format!(" AND channel = ${bind_idx}"));
        binds.push(ch.to_string());
    }
    if let Some(tk) = template_key {
        bind_idx += 1;
        query.push_str(&format!(" AND template_key = ${bind_idx}"));
        binds.push(tk.to_string());
    }

    query.push_str(" ORDER BY dead_lettered_at DESC");
    bind_idx += 1;
    query.push_str(&format!(" LIMIT ${bind_idx}"));
    bind_idx += 1;
    query.push_str(&format!(" OFFSET ${bind_idx}"));

    let mut q = sqlx::query_as::<_, DlqRow>(&query);
    for b in &binds {
        q = q.bind(b);
    }
    q = q.bind(limit).bind(offset);

    q.fetch_all(pool).await
}

pub async fn get_dead_lettered_item(
    pool: &PgPool,
    id: Uuid,
    tenant_id: &str,
) -> Result<Option<DlqRow>, sqlx::Error> {
    sqlx::query_as::<_, DlqRow>(
        "SELECT id, recipient_ref, channel, template_key, payload_json, \
         retry_count, last_error, dead_lettered_at, created_at \
         FROM scheduled_notifications \
         WHERE id = $1 AND status = 'dead_lettered' AND tenant_id = $2",
    )
    .bind(id)
    .bind(tenant_id)
    .fetch_optional(pool)
    .await
}

pub async fn get_delivery_attempts(
    pool: &PgPool,
    notification_id: Uuid,
) -> Result<Vec<AttemptRow>, sqlx::Error> {
    sqlx::query_as::<_, AttemptRow>(
        "SELECT id, attempt_no, status, provider_message_id, error_class, \
         error_message, rendered_subject, created_at \
         FROM notification_delivery_attempts \
         WHERE notification_id = $1 ORDER BY created_at ASC",
    )
    .bind(notification_id)
    .fetch_all(pool)
    .await
}

pub async fn get_status_for_update_tx(
    tx: &mut Transaction<'_, Postgres>,
    id: Uuid,
    tenant_id: &str,
) -> Result<Option<StatusOnly>, sqlx::Error> {
    sqlx::query_as::<_, StatusOnly>(
        "SELECT status FROM scheduled_notifications \
         WHERE id = $1 AND tenant_id = $2 FOR UPDATE",
    )
    .bind(id)
    .bind(tenant_id)
    .fetch_optional(&mut **tx)
    .await
}

pub async fn replay_notification_tx(
    tx: &mut Transaction<'_, Postgres>,
    id: Uuid,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE scheduled_notifications \
         SET status = 'pending', \
             deliver_at = NOW(), \
             retry_count = 0, \
             replay_generation = replay_generation + 1, \
             last_error = NULL, \
             dead_lettered_at = NULL, \
             failed_at = NULL \
         WHERE id = $1",
    )
    .bind(id)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

pub async fn abandon_notification_tx(
    tx: &mut Transaction<'_, Postgres>,
    id: Uuid,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE scheduled_notifications \
         SET status = 'abandoned', \
             abandoned_at = NOW() \
         WHERE id = $1",
    )
    .bind(id)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

pub async fn ping(pool: &PgPool) -> Result<(), sqlx::Error> {
    sqlx::query("SELECT 1").execute(pool).await?;
    Ok(())
}
