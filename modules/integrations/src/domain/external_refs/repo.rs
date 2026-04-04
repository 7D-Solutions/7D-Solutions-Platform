//! Repository layer for external refs persistence.

use chrono::{DateTime, Utc};
use sqlx::PgPool;

use super::models::ExternalRef;

pub async fn get_by_id(
    pool: &PgPool,
    app_id: &str,
    ref_id: i64,
) -> Result<Option<ExternalRef>, sqlx::Error> {
    sqlx::query_as::<_, ExternalRef>(
        r#"
        SELECT id, app_id, entity_type, entity_id, system, external_id,
               label, metadata, created_at, updated_at
        FROM integrations_external_refs
        WHERE id = $1 AND app_id = $2
        "#,
    )
    .bind(ref_id)
    .bind(app_id)
    .fetch_optional(pool)
    .await
}

pub async fn list_by_entity(
    pool: &PgPool,
    app_id: &str,
    entity_type: &str,
    entity_id: &str,
) -> Result<Vec<ExternalRef>, sqlx::Error> {
    sqlx::query_as::<_, ExternalRef>(
        r#"
        SELECT id, app_id, entity_type, entity_id, system, external_id,
               label, metadata, created_at, updated_at
        FROM integrations_external_refs
        WHERE app_id = $1 AND entity_type = $2 AND entity_id = $3
        ORDER BY system, external_id
        "#,
    )
    .bind(app_id)
    .bind(entity_type)
    .bind(entity_id)
    .fetch_all(pool)
    .await
}

pub async fn get_by_external(
    pool: &PgPool,
    app_id: &str,
    system: &str,
    external_id: &str,
) -> Result<Option<ExternalRef>, sqlx::Error> {
    sqlx::query_as::<_, ExternalRef>(
        r#"
        SELECT id, app_id, entity_type, entity_id, system, external_id,
               label, metadata, created_at, updated_at
        FROM integrations_external_refs
        WHERE app_id = $1 AND system = $2 AND external_id = $3
        "#,
    )
    .bind(app_id)
    .bind(system)
    .bind(external_id)
    .fetch_optional(pool)
    .await
}

pub async fn upsert(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    app_id: &str,
    entity_type: &str,
    entity_id: &str,
    system: &str,
    external_id: &str,
    label: &Option<String>,
    metadata: &Option<serde_json::Value>,
) -> Result<ExternalRef, sqlx::Error> {
    sqlx::query_as(
        r#"
        INSERT INTO integrations_external_refs
            (app_id, entity_type, entity_id, system, external_id, label, metadata,
             created_at, updated_at)
        VALUES ($1, $2, $3, $4, $5, $6, $7, NOW(), NOW())
        ON CONFLICT (app_id, system, external_id) DO UPDATE SET
            label      = COALESCE(EXCLUDED.label, integrations_external_refs.label),
            metadata   = COALESCE(EXCLUDED.metadata, integrations_external_refs.metadata),
            updated_at = NOW()
        RETURNING id, app_id, entity_type, entity_id, system, external_id,
                  label, metadata, created_at, updated_at
        "#,
    )
    .bind(app_id)
    .bind(entity_type)
    .bind(entity_id)
    .bind(system)
    .bind(external_id)
    .bind(label)
    .bind(metadata)
    .fetch_one(&mut **tx)
    .await
}

pub async fn fetch_for_update(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    ref_id: i64,
    app_id: &str,
) -> Result<Option<ExternalRef>, sqlx::Error> {
    sqlx::query_as(
        r#"
        SELECT id, app_id, entity_type, entity_id, system, external_id,
               label, metadata, created_at, updated_at
        FROM integrations_external_refs
        WHERE id = $1 AND app_id = $2
        FOR UPDATE
        "#,
    )
    .bind(ref_id)
    .bind(app_id)
    .fetch_optional(&mut **tx)
    .await
}

pub async fn update(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    label: &Option<String>,
    metadata: &Option<serde_json::Value>,
    now: DateTime<Utc>,
    ref_id: i64,
    app_id: &str,
) -> Result<ExternalRef, sqlx::Error> {
    sqlx::query_as(
        r#"
        UPDATE integrations_external_refs
        SET label = $1, metadata = $2, updated_at = $3
        WHERE id = $4 AND app_id = $5
        RETURNING id, app_id, entity_type, entity_id, system, external_id,
                  label, metadata, created_at, updated_at
        "#,
    )
    .bind(label)
    .bind(metadata)
    .bind(now)
    .bind(ref_id)
    .bind(app_id)
    .fetch_one(&mut **tx)
    .await
}

/// Fetch by id within a transaction (no row lock).
/// Reuses the same query as `fetch_for_update` without `FOR UPDATE`.
pub async fn get_by_id_in_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    ref_id: i64,
    app_id: &str,
) -> Result<Option<ExternalRef>, sqlx::Error> {
    sqlx::query_as(
        r#"
        SELECT id, app_id, entity_type, entity_id, system, external_id,
               label, metadata, created_at, updated_at
        FROM integrations_external_refs
        WHERE id = $1 AND app_id = $2
        "#,
    )
    .bind(ref_id)
    .bind(app_id)
    .fetch_optional(&mut **tx)
    .await
}

pub async fn delete(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    ref_id: i64,
    app_id: &str,
) -> Result<sqlx::postgres::PgQueryResult, sqlx::Error> {
    sqlx::query("DELETE FROM integrations_external_refs WHERE id = $1 AND app_id = $2")
        .bind(ref_id)
        .bind(app_id)
        .execute(&mut **tx)
        .await
}
