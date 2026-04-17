//! Status label service.

use sqlx::PgPool;
use uuid::Uuid;

use super::{repo, LabelError, ListLabelsQuery, StatusLabel, UpsertLabelRequest};

pub async fn list_labels(
    pool: &PgPool,
    tenant_id: &str,
    query: &ListLabelsQuery,
) -> Result<Vec<StatusLabel>, LabelError> {
    Ok(repo::list_labels(pool, tenant_id, query.label_type.as_deref()).await?)
}

pub async fn upsert_label(
    pool: &PgPool,
    tenant_id: &str,
    label_type: &str,
    status_key: &str,
    req: UpsertLabelRequest,
) -> Result<StatusLabel, LabelError> {
    Ok(repo::upsert_label(
        pool,
        Uuid::new_v4(),
        tenant_id,
        label_type,
        status_key,
        &req.display_name,
        req.color_hex.as_deref(),
        req.sort_order.unwrap_or(0),
    )
    .await?)
}

pub async fn delete_label(
    pool: &PgPool,
    tenant_id: &str,
    label_type: &str,
    status_key: &str,
) -> Result<(), LabelError> {
    let deleted = repo::delete_label(pool, tenant_id, label_type, status_key).await?;
    if !deleted {
        return Err(LabelError::NotFound(
            label_type.to_string(),
            status_key.to_string(),
        ));
    }
    Ok(())
}
