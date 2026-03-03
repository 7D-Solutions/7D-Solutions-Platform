use chrono::{DateTime, Duration, Utc};
use serde_json::Value;
use sqlx::PgPool;
use uuid::Uuid;

use crate::event_bus::{create_notifications_envelope, enqueue_event};
use super::models::ScheduledNotification;
use super::sender::{NotificationError, SendReceipt};

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
        sqlx::query("UPDATE scheduled_notifications SET status = 'failed' WHERE id = $1")
            .bind(id)
            .execute(pool)
            .await?;
    }

    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttemptApplyOutcome {
    Succeeded,
    FailedRetryable,
    FailedPermanent,
    DuplicateStored,
}

fn tenant_from_recipient_ref(recipient_ref: &str) -> String {
    recipient_ref
        .split(':')
        .next()
        .filter(|s| !s.is_empty())
        .unwrap_or("unknown")
        .to_string()
}

fn next_status_after_failure(retry_count: i32, retryable: bool) -> (&'static str, Option<DateTime<Utc>>, bool) {
    if retryable && retry_count < 5 {
        let backoff_minutes = (retry_count + 1) as i64 * 5;
        let next_deliver_at = Utc::now() + Duration::minutes(backoff_minutes);
        ("pending", Some(next_deliver_at), true)
    } else {
        ("failed", None, false)
    }
}

/// Persist one delivery attempt + scheduled-notification mutation + outbox event atomically.
///
/// This enforces idempotency on `idempotency_key`: if the key already exists, the
/// previously stored outcome is returned and no new send side effect should be executed.
pub async fn record_delivery_attempt_and_mutate(
    pool: &PgPool,
    notif: &ScheduledNotification,
    idempotency_key: &str,
    send_result: Result<SendReceipt, NotificationError>,
) -> Result<AttemptApplyOutcome, sqlx::Error> {
    let mut tx = pool.begin().await?;

    // Idempotency guard: if attempt already recorded, return stored result.
    let existing = sqlx::query_as::<_, (String,)>(
        r#"
        SELECT status
        FROM notification_delivery_attempts
        WHERE idempotency_key = $1
        "#,
    )
    .bind(idempotency_key)
    .fetch_optional(&mut *tx)
    .await?;

    if existing.is_some() {
        tx.commit().await?;
        return Ok(AttemptApplyOutcome::DuplicateStored);
    }

    let (attempt_status, next_sn_status, next_deliver_at, retry_increment, event_type, provider_message_id, error_class, error_message, outcome) =
        match send_result {
            Ok(receipt) => (
                "succeeded",
                "sent",
                None,
                0,
                "notifications.delivery.succeeded",
                receipt.provider_message_id,
                None,
                None,
                AttemptApplyOutcome::Succeeded,
            ),
            Err(err) => {
                let (sn_status, deliver_at, retryable) =
                    next_status_after_failure(notif.retry_count, err.retryable());
                let out = if retryable {
                    AttemptApplyOutcome::FailedRetryable
                } else {
                    AttemptApplyOutcome::FailedPermanent
                };
                (
                    if retryable {
                        "failed_retryable"
                    } else {
                        "failed_permanent"
                    },
                    sn_status,
                    deliver_at,
                    if retryable { 1 } else { 0 },
                    "notifications.delivery.failed",
                    None,
                    Some(err.class().to_string()),
                    Some(err.to_string()),
                    out,
                )
            }
        };

    sqlx::query(
        r#"
        INSERT INTO notification_delivery_attempts
            (notification_id, idempotency_key, attempt_no, status, provider_message_id, error_class, error_message)
        VALUES ($1, $2, $3, $4, $5, $6, $7)
        "#,
    )
    .bind(notif.id)
    .bind(idempotency_key)
    .bind(notif.retry_count + 1)
    .bind(attempt_status)
    .bind(provider_message_id.clone())
    .bind(error_class.clone())
    .bind(error_message.clone())
    .execute(&mut *tx)
    .await?;

    if let Some(deliver_at) = next_deliver_at {
        sqlx::query(
            r#"
            UPDATE scheduled_notifications
            SET status = $1, deliver_at = $2, retry_count = retry_count + $3
            WHERE id = $4
            "#,
        )
        .bind(next_sn_status)
        .bind(deliver_at)
        .bind(retry_increment)
        .bind(notif.id)
        .execute(&mut *tx)
        .await?;
    } else {
        sqlx::query(
            r#"
            UPDATE scheduled_notifications
            SET status = $1, retry_count = retry_count + $2
            WHERE id = $3
            "#,
        )
        .bind(next_sn_status)
        .bind(retry_increment)
        .bind(notif.id)
        .execute(&mut *tx)
        .await?;
    }

    let payload = serde_json::json!({
        "notification_id": notif.id,
        "channel": notif.channel,
        "template_key": notif.template_key,
        "attempt_no": notif.retry_count + 1,
        "attempt_status": attempt_status,
        "scheduled_status": next_sn_status,
        "idempotency_key": idempotency_key,
        "provider_message_id": provider_message_id,
        "error_class": error_class,
        "error_message": error_message,
    });
    let envelope = create_notifications_envelope(
        Uuid::new_v4(),
        tenant_from_recipient_ref(&notif.recipient_ref),
        event_type.to_string(),
        None,
        None,
        "SIDE_EFFECT".to_string(),
        payload,
    );
    enqueue_event(&mut tx, event_type, &envelope).await?;

    tx.commit().await?;
    Ok(outcome)
}
