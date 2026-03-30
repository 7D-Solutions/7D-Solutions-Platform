use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, FromRow, ToSchema)]
pub struct BomHeader {
    pub id: Uuid,
    pub tenant_id: String,
    pub part_id: Uuid,
    pub description: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow, ToSchema)]
pub struct BomRevision {
    pub id: Uuid,
    pub bom_id: Uuid,
    pub tenant_id: String,
    pub revision_label: String,
    pub status: String,
    pub effective_from: Option<DateTime<Utc>>,
    pub effective_to: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow, ToSchema)]
pub struct BomLine {
    pub id: Uuid,
    pub revision_id: Uuid,
    pub tenant_id: String,
    pub component_item_id: Uuid,
    pub quantity: f64,
    pub uom: Option<String>,
    pub scrap_factor: Option<f64>,
    pub find_number: Option<i32>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// A single row from the multi-level BOM explosion. The SQL recursive CTE
/// flattens the tree into rows, each tagged with its depth `level` and the
/// parent it was expanded from.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ExplosionRow {
    pub level: i32,
    pub parent_part_id: Uuid,
    pub component_item_id: Uuid,
    pub quantity: f64,
    pub uom: Option<String>,
    pub scrap_factor: f64,
    pub revision_id: Uuid,
    pub revision_label: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct WhereUsedRow {
    pub bom_id: Uuid,
    pub part_id: Uuid,
    pub revision_id: Uuid,
    pub revision_label: String,
    pub quantity: f64,
    pub uom: Option<String>,
}

// Request types

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateBomRequest {
    pub part_id: Uuid,
    pub description: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateRevisionRequest {
    pub revision_label: String,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct SetEffectivityRequest {
    pub effective_from: DateTime<Utc>,
    pub effective_to: Option<DateTime<Utc>>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct AddLineRequest {
    pub component_item_id: Uuid,
    pub quantity: f64,
    pub uom: Option<String>,
    pub scrap_factor: Option<f64>,
    pub find_number: Option<i32>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct UpdateLineRequest {
    pub quantity: Option<f64>,
    pub uom: Option<String>,
    pub scrap_factor: Option<f64>,
    pub find_number: Option<i32>,
}

#[derive(Debug, Deserialize, IntoParams)]
pub struct ExplosionQuery {
    /// Point-in-time date for effectivity filtering (defaults to now).
    pub date: Option<DateTime<Utc>>,
    /// Maximum recursion depth (1–100, defaults to 20).
    pub max_depth: Option<i32>,
}

#[derive(Debug, Deserialize, IntoParams)]
pub struct WhereUsedQuery {
    /// Point-in-time date for effectivity filtering (defaults to now).
    pub date: Option<DateTime<Utc>>,
}

fn default_page() -> i64 {
    1
}
fn default_page_size() -> i64 {
    50
}

#[derive(Debug, Deserialize, IntoParams)]
pub struct PaginationQuery {
    #[serde(default = "default_page")]
    pub page: i64,
    #[serde(default = "default_page_size")]
    pub page_size: i64,
}
