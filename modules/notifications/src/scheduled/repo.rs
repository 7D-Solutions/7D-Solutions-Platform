use chrono::{DateTime, Duration, Utc};
use sqlx::PgPool;
use uuid::Uuid;
use serde_json::Value;

use super::models::ScheduledNotification;

/// Insert a new pending scheduled notification. Returns the generated id.
pub async fn insert_pending(
    pool: &PgPool,
    recipient_ref: &str,
    channel: &str,
    template_key: &str,
    payload_json: Value,
    deliver_at: DateTime<Utc>,
) -> Result<Uuid, sqlx::Error> {
    let row = sqlx::query_as::<_, (Uuid,)>(
        r#"
        INSERT INTO scheduled_notifications
            (recipient_ref, channel, template_key, payload_json, deliver_at)
        VALUES ($1, $2, $3, $4, $5)
        RETURNING id
        "#,
    )
    .bind(recipient_ref)
    .bind(channel)
    .bind(template_key)
    .bind(payload_json)
    .bind(deliver_at)
    .fetch_one(pool)
    .await?;

    Ok(row.0)
}

/// Reset claimed rows whose `last_attempt_at` is older than `cutoff` back to
/// `pending` so they can be retried by the next dispatcher cycle.
pub async fn reset_orphaned_claims(
    pool: &PgPool,
    cutoff: DateTime<Utc>,
) -> Result<u64, sqlx::Error> {
    let result = sqlx::query(
        r#"
        UPDATE scheduled_notifications
        SET status = 'pending'
        WHERE status = 'claimed'
          AND last_attempt_at < $1
        "#,
    )
    .bind(cutoff)
    .execute(pool)
    .await?;

    Ok(result.rows_affected())
}

/// Atomically claim up to `limit` due notifications using `FOR UPDATE SKIP LOCKED`.
/// Returns the claimed rows so the dispatcher can process them.
pub async fn claim_due_batch(
    pool: &PgPool,
    limit: i64,
) -> Result<Vec<ScheduledNotification>, sqlx::Error> {
    let rows = sqlx::query_as::<_, ScheduledNotification>(
        r#"
        UPDATE scheduled_notifications
        SET status = 'claimed',
            last_attempt_at = NOW()
        WHERE id IN (
            SELECT id FROM scheduled_notifications
            WHERE deliver_at <= NOW()
              AND status = 'pending'
            ORDER BY deliver_at
            LIMIT $1
            FOR UPDATE SKIP LOCKED
        )
        RETURNING
            id,
            recipient_ref,
            channel,
            template_key,
            payload_json,
            deliver_at,
            status,
            retry_count,
            last_attempt_at,
            created_at
        "#,
    )
    .bind(limit)
    .fetch_all(pool)
    .await?;

    Ok(rows)
}

/// Mark a notification as successfully sent.
pub async fn mark_sent(pool: &PgPool, id: Uuid) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE scheduled_notifications SET status = 'sent' WHERE id = $1")
        .bind(id)
        .execute(pool)
        .await?;

    Ok(())
}

/// Reschedule a failed notification for retry, or mark it `failed` if
/// `retry_count` has reached the limit (5 attempts).
///
/// Back-off: `(retry_count + 1) * 5` minutes from now.
pub async fn reschedule_or_fail(
    pool: &PgPool,
    id: Uuid,
    retry_count: i32,
) -> Result<(), sqlx::Error> {
    if retry_count < 5 {
        let backoff_minutes = (retry_count + 1) as i64 * 5;
        let next_deliver_at = Utc::now() + Duration::minutes(backoff_minutes);

        sqlx::query(
            r#"
            UPDATE scheduled_notifications
            SET status = 'pending',
                deliver_at = $1,
                retry_count = retry_count + 1
            WHERE id = $2
            "#,
        )
        .bind(next_deliver_at)
        .bind(id)
        .execute(pool)
        .await?;
    } else {
        sqlx::query(
            "UPDATE scheduled_notifications SET status = 'failed' WHERE id = $1",
        )
        .bind(id)
        .execute(pool)
        .await?;
    }

    Ok(())
}
