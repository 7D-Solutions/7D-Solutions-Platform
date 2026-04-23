//! Per-tenant display label tables for canonical CRM values.
//!
//! Tables: lead_status_labels, lead_source_labels, opp_type_labels, opp_priority_labels.

use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use thiserror::Error;
use utoipa::ToSchema;

#[derive(Debug, Error)]
pub enum LabelError {
    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),
}

impl From<LabelError> for platform_http_contracts::ApiError {
    fn from(err: LabelError) -> Self {
        match err {
            LabelError::Database(e) => {
                tracing::error!("CRM labels DB error: {}", e);
                Self::internal("Internal database error")
            }
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow, ToSchema)]
pub struct Label {
    pub tenant_id: String,
    pub canonical: String,
    pub display_label: String,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct UpsertLabelRequest {
    pub canonical: String,
    pub display_label: String,
}

pub async fn list_status_labels(pool: &PgPool, tenant_id: &str) -> Result<Vec<Label>, LabelError> {
    list_from(pool, tenant_id, "lead_status_labels").await
}

pub async fn list_source_labels(pool: &PgPool, tenant_id: &str) -> Result<Vec<Label>, LabelError> {
    list_from(pool, tenant_id, "lead_source_labels").await
}

pub async fn list_type_labels(pool: &PgPool, tenant_id: &str) -> Result<Vec<Label>, LabelError> {
    list_from(pool, tenant_id, "opp_type_labels").await
}

pub async fn list_priority_labels(
    pool: &PgPool,
    tenant_id: &str,
) -> Result<Vec<Label>, LabelError> {
    list_from(pool, tenant_id, "opp_priority_labels").await
}

pub async fn upsert_label(
    pool: &PgPool,
    tenant_id: &str,
    table: &str,
    req: &UpsertLabelRequest,
) -> Result<Label, LabelError> {
    let sql = format!(
        r#"
        INSERT INTO {table} (tenant_id, canonical, display_label)
        VALUES ($1, $2, $3)
        ON CONFLICT (tenant_id, canonical) DO UPDATE SET display_label = EXCLUDED.display_label
        RETURNING *
        "#,
        table = table
    );
    let row = sqlx::query_as::<_, Label>(&sql)
        .bind(tenant_id)
        .bind(&req.canonical)
        .bind(&req.display_label)
        .fetch_one(pool)
        .await?;
    Ok(row)
}

async fn list_from(pool: &PgPool, tenant_id: &str, table: &str) -> Result<Vec<Label>, LabelError> {
    let sql = format!(
        "SELECT * FROM {table} WHERE tenant_id = $1 ORDER BY canonical ASC",
        table = table
    );
    let rows = sqlx::query_as::<_, Label>(&sql)
        .bind(tenant_id)
        .fetch_all(pool)
        .await?;
    Ok(rows)
}
