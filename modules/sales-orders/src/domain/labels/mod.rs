//! Status label domain — canonical status display names per tenant.

pub mod repo;
pub mod service;

use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use utoipa::ToSchema;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, FromRow)]
pub struct StatusLabel {
    pub id: Uuid,
    pub tenant_id: String,
    pub label_type: String,
    pub status_key: String,
    pub display_name: String,
    pub color_hex: Option<String>,
    pub sort_order: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct UpsertLabelRequest {
    pub display_name: String,
    pub color_hex: Option<String>,
    pub sort_order: Option<i32>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ListLabelsQuery {
    pub label_type: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum LabelError {
    #[error("Status label not found: {0}/{1}")]
    NotFound(String, String),

    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),
}

impl From<LabelError> for platform_http_contracts::ApiError {
    fn from(err: LabelError) -> Self {
        match err {
            LabelError::NotFound(lt, sk) => {
                Self::not_found(format!("Status label {}/{} not found", lt, sk))
            }
            LabelError::Database(e) => {
                tracing::error!("Label DB error: {}", e);
                Self::internal("Internal database error")
            }
        }
    }
}
