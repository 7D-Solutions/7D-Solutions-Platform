//! Repository layer for outbound webhook persistence.

use sqlx::PgPool;
use uuid::Uuid;

use super::models::{OutboundWebhook, OutboundWebhookDelivery};

pub async fn get_by_id(
    pool: &PgPool,
    tenant_id: &str,
    webhook_id: Uuid,
) -> Result<Option<OutboundWebhook>, sqlx::Error> {
    sqlx::query_as::<_, OutboundWebhook>(
        r#"SELECT id, tenant_id, url, event_types, signing_secret_hash,
                  status, idempotency_key, description, created_at, updated_at, deleted_at
           FROM integrations_outbound_webhooks
           WHERE id = $1 AND tenant_id = $2 AND deleted_at IS NULL"#,
    )
    .bind(webhook_id)
    .bind(tenant_id)
    .fetch_optional(pool)
    .await
}

pub async fn list_by_tenant(
    pool: &PgPool,
    tenant_id: &str,
) -> Result<Vec<OutboundWebhook>, sqlx::Error> {
    sqlx::query_as::<_, OutboundWebhook>(
        r#"SELECT id, tenant_id, url, event_types, signing_secret_hash,
                  status, idempotency_key, description, created_at, updated_at, deleted_at
           FROM integrations_outbound_webhooks
           WHERE tenant_id = $1 AND deleted_at IS NULL
           ORDER BY created_at DESC"#,
    )
    .bind(tenant_id)
    .fetch_all(pool)
    .await
}

pub async fn find_by_idempotency_key(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    tenant_id: &str,
    idempotency_key: &str,
) -> Result<Option<OutboundWebhook>, sqlx::Error> {
    sqlx::query_as::<_, OutboundWebhook>(
        r#"SELECT id, tenant_id, url, event_types, signing_secret_hash,
                  status, idempotency_key, description, created_at, updated_at, deleted_at
           FROM integrations_outbound_webhooks
           WHERE tenant_id = $1 AND idempotency_key = $2 AND deleted_at IS NULL"#,
    )
    .bind(tenant_id)
    .bind(idempotency_key)
    .fetch_optional(&mut **tx)
    .await
}

pub async fn insert(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    id: Uuid,
    tenant_id: &str,
    url: &str,
    event_types: &serde_json::Value,
    secret_hash: &str,
    idempotency_key: &Option<String>,
    description: &Option<String>,
) -> Result<OutboundWebhook, sqlx::Error> {
    sqlx::query_as::<_, OutboundWebhook>(
        r#"INSERT INTO integrations_outbound_webhooks
               (id, tenant_id, url, event_types, signing_secret_hash,
                status, idempotency_key, description)
           VALUES ($1, $2, $3, $4, $5, 'active', $6, $7)
           RETURNING id, tenant_id, url, event_types, signing_secret_hash,
                     status, idempotency_key, description, created_at, updated_at, deleted_at"#,
    )
    .bind(id)
    .bind(tenant_id)
    .bind(url)
    .bind(event_types)
    .bind(secret_hash)
    .bind(idempotency_key)
    .bind(description)
    .fetch_one(&mut **tx)
    .await
}

pub async fn fetch_for_update(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    webhook_id: Uuid,
    tenant_id: &str,
) -> Result<Option<OutboundWebhook>, sqlx::Error> {
    sqlx::query_as::<_, OutboundWebhook>(
        r#"SELECT id, tenant_id, url, event_types, signing_secret_hash,
                  status, idempotency_key, description, created_at, updated_at, deleted_at
           FROM integrations_outbound_webhooks
           WHERE id = $1 AND tenant_id = $2 AND deleted_at IS NULL
           FOR UPDATE"#,
    )
    .bind(webhook_id)
    .bind(tenant_id)
    .fetch_optional(&mut **tx)
    .await
}

pub async fn update_fields(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    url: &str,
    event_types: &serde_json::Value,
    status: &str,
    description: Option<&str>,
    webhook_id: Uuid,
    tenant_id: &str,
) -> Result<OutboundWebhook, sqlx::Error> {
    sqlx::query_as::<_, OutboundWebhook>(
        r#"UPDATE integrations_outbound_webhooks
           SET url = $1, event_types = $2, status = $3, description = $4, updated_at = NOW()
           WHERE id = $5 AND tenant_id = $6 AND deleted_at IS NULL
           RETURNING id, tenant_id, url, event_types, signing_secret_hash,
                     status, idempotency_key, description, created_at, updated_at, deleted_at"#,
    )
    .bind(url)
    .bind(event_types)
    .bind(status)
    .bind(description)
    .bind(webhook_id)
    .bind(tenant_id)
    .fetch_one(&mut **tx)
    .await
}

pub async fn soft_delete(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    webhook_id: Uuid,
    tenant_id: &str,
) -> Result<sqlx::postgres::PgQueryResult, sqlx::Error> {
    sqlx::query(
        r#"UPDATE integrations_outbound_webhooks
           SET deleted_at = NOW(), status = 'disabled', updated_at = NOW()
           WHERE id = $1 AND tenant_id = $2 AND deleted_at IS NULL"#,
    )
    .bind(webhook_id)
    .bind(tenant_id)
    .execute(&mut **tx)
    .await
}

pub async fn insert_delivery(
    pool: &PgPool,
    req: &super::models::RecordDeliveryRequest,
) -> Result<OutboundWebhookDelivery, sqlx::Error> {
    sqlx::query_as::<_, OutboundWebhookDelivery>(
        r#"INSERT INTO integrations_outbound_webhook_deliveries
               (webhook_id, tenant_id, event_type, payload, status_code,
                response_body, error_message, attempt_number, next_retry_at, delivered_at)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
           RETURNING id, webhook_id, tenant_id, event_type, payload, status_code,
                     response_body, error_message, attempt_number, next_retry_at,
                     delivered_at, created_at"#,
    )
    .bind(req.webhook_id)
    .bind(&req.tenant_id)
    .bind(&req.event_type)
    .bind(&req.payload)
    .bind(req.status_code)
    .bind(&req.response_body)
    .bind(&req.error_message)
    .bind(req.attempt_number)
    .bind(req.next_retry_at)
    .bind(req.delivered_at)
    .fetch_one(pool)
    .await
}

pub async fn list_deliveries(
    pool: &PgPool,
    tenant_id: &str,
    webhook_id: Uuid,
) -> Result<Vec<OutboundWebhookDelivery>, sqlx::Error> {
    sqlx::query_as::<_, OutboundWebhookDelivery>(
        r#"SELECT id, webhook_id, tenant_id, event_type, payload, status_code,
                  response_body, error_message, attempt_number, next_retry_at,
                  delivered_at, created_at
           FROM integrations_outbound_webhook_deliveries
           WHERE webhook_id = $1 AND tenant_id = $2
           ORDER BY created_at DESC"#,
    )
    .bind(webhook_id)
    .bind(tenant_id)
    .fetch_all(pool)
    .await
}
