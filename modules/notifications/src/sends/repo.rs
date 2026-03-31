use sqlx::PgPool;
use uuid::Uuid;

use super::models::{DeliveryReceipt, NotificationSend};

/// Insert a new notification send record.
pub async fn insert_send(
    pool: &PgPool,
    tenant_id: &str,
    template_key: &str,
    template_version: i32,
    channel: &str,
    recipients: &[String],
    payload_json: &serde_json::Value,
    correlation_id: Option<&str>,
    causation_id: Option<&str>,
    rendered_hash: Option<&str>,
) -> Result<NotificationSend, sqlx::Error> {
    let recipients_json =
        serde_json::to_value(recipients).unwrap_or_else(|_| serde_json::Value::Array(vec![]));

    sqlx::query_as::<_, NotificationSend>(
        r#"
        INSERT INTO notification_sends
            (tenant_id, template_key, template_version, channel, recipients,
             payload_json, correlation_id, causation_id, rendered_hash, status)
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, 'pending')
        RETURNING id, tenant_id, template_key, template_version, channel,
                  recipients, payload_json, correlation_id, causation_id,
                  rendered_hash, status, created_at, updated_at
        "#,
    )
    .bind(tenant_id)
    .bind(template_key)
    .bind(template_version)
    .bind(channel)
    .bind(&recipients_json)
    .bind(payload_json)
    .bind(correlation_id)
    .bind(causation_id)
    .bind(rendered_hash)
    .fetch_one(pool)
    .await
}

/// Get a send by ID, tenant-scoped.
pub async fn get_send(
    pool: &PgPool,
    tenant_id: &str,
    send_id: Uuid,
) -> Result<Option<NotificationSend>, sqlx::Error> {
    sqlx::query_as::<_, NotificationSend>(
        r#"
        SELECT id, tenant_id, template_key, template_version, channel,
               recipients, payload_json, correlation_id, causation_id,
               rendered_hash, status, created_at, updated_at
        FROM notification_sends
        WHERE id = $1 AND tenant_id = $2
        "#,
    )
    .bind(send_id)
    .bind(tenant_id)
    .fetch_optional(pool)
    .await
}

/// Get all delivery receipts for a send, tenant-scoped.
pub async fn get_receipts_for_send(
    pool: &PgPool,
    tenant_id: &str,
    send_id: Uuid,
) -> Result<Vec<DeliveryReceipt>, sqlx::Error> {
    sqlx::query_as::<_, DeliveryReceipt>(
        r#"
        SELECT id, tenant_id, send_id, recipient, channel, status, provider_id,
               attempt_count, last_attempt_at, succeeded_at, failed_at,
               error_class, error_message, created_at, updated_at
        FROM delivery_receipts
        WHERE send_id = $1 AND tenant_id = $2
        ORDER BY created_at
        "#,
    )
    .bind(send_id)
    .bind(tenant_id)
    .fetch_all(pool)
    .await
}

/// Insert a delivery receipt for a recipient.
pub async fn insert_receipt(
    pool: &PgPool,
    tenant_id: &str,
    send_id: Uuid,
    recipient: &str,
    channel: &str,
    status: &str,
    provider_id: Option<&str>,
    error_class: Option<&str>,
    error_message: Option<&str>,
) -> Result<DeliveryReceipt, sqlx::Error> {
    let now = chrono::Utc::now();
    let succeeded_at = if status == "succeeded" {
        Some(now)
    } else {
        None
    };
    let failed_at = if status == "failed" || status == "dlq" {
        Some(now)
    } else {
        None
    };

    sqlx::query_as::<_, DeliveryReceipt>(
        r#"
        INSERT INTO delivery_receipts
            (tenant_id, send_id, recipient, channel, status, provider_id,
             attempt_count, last_attempt_at, succeeded_at, failed_at,
             error_class, error_message)
        VALUES ($1, $2, $3, $4, $5, $6, 1, NOW(), $7, $8, $9, $10)
        RETURNING id, tenant_id, send_id, recipient, channel, status, provider_id,
                  attempt_count, last_attempt_at, succeeded_at, failed_at,
                  error_class, error_message, created_at, updated_at
        "#,
    )
    .bind(tenant_id)
    .bind(send_id)
    .bind(recipient)
    .bind(channel)
    .bind(status)
    .bind(provider_id)
    .bind(succeeded_at)
    .bind(failed_at)
    .bind(error_class)
    .bind(error_message)
    .fetch_one(pool)
    .await
}

/// Update send status based on receipt outcomes.
pub async fn update_send_status(
    pool: &PgPool,
    send_id: Uuid,
    status: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE notification_sends SET status = $1, updated_at = NOW() WHERE id = $2")
        .bind(status)
        .bind(send_id)
        .execute(pool)
        .await?;
    Ok(())
}

/// Count delivery receipts matching filters (for pagination metadata).
pub async fn count_receipts(
    pool: &PgPool,
    tenant_id: &str,
    correlation_id: Option<&str>,
    recipient: Option<&str>,
    from: Option<chrono::DateTime<chrono::Utc>>,
    to: Option<chrono::DateTime<chrono::Utc>>,
) -> Result<i64, sqlx::Error> {
    let (count,): (i64,) = sqlx::query_as(
        r#"
        SELECT COUNT(*)
        FROM delivery_receipts dr
        JOIN notification_sends ns ON ns.id = dr.send_id
        WHERE dr.tenant_id = $1
          AND ($2::text IS NULL OR ns.correlation_id = $2)
          AND ($3::text IS NULL OR dr.recipient = $3)
          AND ($4::timestamptz IS NULL OR dr.created_at >= $4)
          AND ($5::timestamptz IS NULL OR dr.created_at <= $5)
        "#,
    )
    .bind(tenant_id)
    .bind(correlation_id)
    .bind(recipient)
    .bind(from)
    .bind(to)
    .fetch_one(pool)
    .await?;
    Ok(count)
}

/// Query delivery receipts with filters (tenant-scoped).
pub async fn query_receipts(
    pool: &PgPool,
    tenant_id: &str,
    correlation_id: Option<&str>,
    recipient: Option<&str>,
    from: Option<chrono::DateTime<chrono::Utc>>,
    to: Option<chrono::DateTime<chrono::Utc>>,
    limit: i64,
    offset: i64,
) -> Result<Vec<DeliveryReceipt>, sqlx::Error> {
    sqlx::query_as::<_, DeliveryReceipt>(
        r#"
        SELECT dr.id, dr.tenant_id, dr.send_id, dr.recipient, dr.channel,
               dr.status, dr.provider_id, dr.attempt_count, dr.last_attempt_at,
               dr.succeeded_at, dr.failed_at, dr.error_class, dr.error_message,
               dr.created_at, dr.updated_at
        FROM delivery_receipts dr
        JOIN notification_sends ns ON ns.id = dr.send_id
        WHERE dr.tenant_id = $1
          AND ($2::text IS NULL OR ns.correlation_id = $2)
          AND ($3::text IS NULL OR dr.recipient = $3)
          AND ($4::timestamptz IS NULL OR dr.created_at >= $4)
          AND ($5::timestamptz IS NULL OR dr.created_at <= $5)
        ORDER BY dr.created_at DESC
        LIMIT $6 OFFSET $7
        "#,
    )
    .bind(tenant_id)
    .bind(correlation_id)
    .bind(recipient)
    .bind(from)
    .bind(to)
    .bind(limit)
    .bind(offset)
    .fetch_all(pool)
    .await
}
