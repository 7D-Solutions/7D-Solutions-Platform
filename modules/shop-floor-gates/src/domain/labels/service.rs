use sqlx::PgPool;
use uuid::Uuid;

use platform_http_contracts::ApiError;

use super::{repo, StatusLabel, UpsertLabelRequest, LABEL_TABLES};

fn validate_table(table: &str) -> Result<(), ApiError> {
    if !LABEL_TABLES.contains(&table) {
        return Err(ApiError::bad_request(format!("Unknown label table: {}", table)));
    }
    Ok(())
}

pub async fn upsert_label(
    pool: &PgPool,
    table: &str,
    tenant_id: &str,
    req: UpsertLabelRequest,
) -> Result<StatusLabel, ApiError> {
    validate_table(table)?;

    let label = StatusLabel {
        id: Uuid::new_v4(),
        tenant_id: tenant_id.to_string(),
        status_key: req.status_key,
        display_name: req.display_name,
        color_hex: req.color_hex,
        sort_order: req.sort_order.unwrap_or(0),
    };

    repo::upsert_label(pool, table, tenant_id, &label)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))
}

pub async fn list_labels(pool: &PgPool, table: &str, tenant_id: &str) -> Result<Vec<StatusLabel>, ApiError> {
    validate_table(table)?;
    repo::list_labels(pool, table, tenant_id).await.map_err(|e| ApiError::internal(e.to_string()))
}

pub async fn delete_label(pool: &PgPool, table: &str, id: Uuid, tenant_id: &str) -> Result<bool, ApiError> {
    validate_table(table)?;
    repo::delete_label(pool, table, id, tenant_id).await.map_err(|e| ApiError::internal(e.to_string()))
}
