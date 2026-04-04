//! Label repository — database operations for label generation.
//!
//! `Label` is defined here (it has `sqlx::FromRow`) and re-exported from
//! `labels.rs` for API compatibility. All `sqlx::query` calls live here;
//! `labels.rs` handles business-logic guards and error translation only.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

use crate::domain::revisions::ItemRevision;

// ============================================================================
// Domain model (re-exported via labels.rs)
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow, utoipa::ToSchema)]
pub struct Label {
    pub id: Uuid,
    pub tenant_id: String,
    pub item_id: Uuid,
    pub revision_id: Uuid,
    pub label_type: String,
    pub barcode_format: String,
    pub payload: serde_json::Value,
    pub idempotency_key: Option<String>,
    pub actor_id: Option<Uuid>,
    pub created_at: DateTime<Utc>,
}

// ============================================================================
// Internal DB row types
// ============================================================================

#[derive(sqlx::FromRow)]
pub(crate) struct IdempotencyRecord {
    pub response_body: String,
    pub request_hash: String,
}

#[derive(sqlx::FromRow)]
pub(crate) struct ItemRow {
    pub sku: String,
    pub active: bool,
}

// ============================================================================
// Idempotency
// ============================================================================

pub(crate) async fn find_idempotency_key(
    pool: &PgPool,
    tenant_id: &str,
    idempotency_key: &str,
) -> Result<Option<IdempotencyRecord>, sqlx::Error> {
    sqlx::query_as::<_, IdempotencyRecord>(
        r#"
        SELECT response_body::TEXT AS response_body, request_hash
        FROM inv_idempotency_keys
        WHERE tenant_id = $1 AND idempotency_key = $2
        "#,
    )
    .bind(tenant_id)
    .bind(idempotency_key)
    .fetch_optional(pool)
    .await
}

pub(crate) async fn store_idempotency_key(
    tx: &mut Transaction<'_, Postgres>,
    tenant_id: &str,
    idempotency_key: &str,
    request_hash: &str,
    response_json: &str,
    expires_at: DateTime<Utc>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        INSERT INTO inv_idempotency_keys
            (tenant_id, idempotency_key, request_hash, response_body, status_code, expires_at)
        VALUES ($1, $2, $3, $4::JSONB, 201, $5)
        "#,
    )
    .bind(tenant_id)
    .bind(idempotency_key)
    .bind(request_hash)
    .bind(response_json)
    .bind(expires_at)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

// ============================================================================
// Guard reads
// ============================================================================

/// Fetch item fields needed for label guards. Returns None if not found.
pub(crate) async fn find_item(
    pool: &PgPool,
    item_id: Uuid,
    tenant_id: &str,
) -> Result<Option<ItemRow>, sqlx::Error> {
    sqlx::query_as::<_, ItemRow>(
        "SELECT sku, active FROM items WHERE id = $1 AND tenant_id = $2",
    )
    .bind(item_id)
    .bind(tenant_id)
    .fetch_optional(pool)
    .await
}

/// Fetch an item revision. Returns None if not found for tenant.
pub(crate) async fn find_revision(
    pool: &PgPool,
    revision_id: Uuid,
    tenant_id: &str,
) -> Result<Option<ItemRevision>, sqlx::Error> {
    sqlx::query_as::<_, ItemRevision>(
        r#"
        SELECT * FROM item_revisions
        WHERE id = $1 AND tenant_id = $2
        "#,
    )
    .bind(revision_id)
    .bind(tenant_id)
    .fetch_optional(pool)
    .await
}

// ============================================================================
// Mutations
// ============================================================================

/// Insert a label row. Returns the inserted Label.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn insert_label(
    tx: &mut Transaction<'_, Postgres>,
    tenant_id: &str,
    item_id: Uuid,
    revision_id: Uuid,
    label_type: &str,
    barcode_format: &str,
    label_payload: &serde_json::Value,
    idempotency_key: &str,
    actor_id: Option<Uuid>,
    now: DateTime<Utc>,
) -> Result<Label, sqlx::Error> {
    sqlx::query_as::<_, Label>(
        r#"
        INSERT INTO inv_labels
            (tenant_id, item_id, revision_id, label_type, barcode_format,
             payload, idempotency_key, actor_id, created_at)
        VALUES ($1, $2, $3, $4, $5, $6::JSONB, $7, $8, $9)
        RETURNING *
        "#,
    )
    .bind(tenant_id)
    .bind(item_id)
    .bind(revision_id)
    .bind(label_type)
    .bind(barcode_format)
    .bind(label_payload)
    .bind(idempotency_key)
    .bind(actor_id)
    .bind(now)
    .fetch_one(&mut **tx)
    .await
}

pub(crate) async fn insert_outbox_event(
    tx: &mut Transaction<'_, Postgres>,
    event_id: Uuid,
    event_type: &str,
    aggregate_id: &str,
    tenant_id: &str,
    envelope_json: &str,
    correlation_id: &str,
    causation_id: Option<&str>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        INSERT INTO inv_outbox
            (event_id, event_type, aggregate_type, aggregate_id, tenant_id,
             payload, correlation_id, causation_id, schema_version)
        VALUES ($1, $2, 'label', $3, $4, $5::JSONB, $6, $7, '1.0.0')
        "#,
    )
    .bind(event_id)
    .bind(event_type)
    .bind(aggregate_id)
    .bind(tenant_id)
    .bind(envelope_json)
    .bind(correlation_id)
    .bind(causation_id)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

// ============================================================================
// Queries
// ============================================================================

/// Fetch a single label by ID, scoped to tenant.
pub(crate) async fn get_label(
    pool: &PgPool,
    tenant_id: &str,
    label_id: Uuid,
) -> Result<Option<Label>, sqlx::Error> {
    sqlx::query_as::<_, Label>("SELECT * FROM inv_labels WHERE id = $1 AND tenant_id = $2")
        .bind(label_id)
        .bind(tenant_id)
        .fetch_optional(pool)
        .await
}

/// List all labels for an item, ordered by created_at descending.
pub(crate) async fn list_labels(
    pool: &PgPool,
    tenant_id: &str,
    item_id: Uuid,
) -> Result<Vec<Label>, sqlx::Error> {
    sqlx::query_as::<_, Label>(
        r#"
        SELECT * FROM inv_labels
        WHERE tenant_id = $1 AND item_id = $2
        ORDER BY created_at DESC
        "#,
    )
    .bind(tenant_id)
    .bind(item_id)
    .fetch_all(pool)
    .await
}
