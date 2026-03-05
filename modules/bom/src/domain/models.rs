use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct BomHeader {
    pub id: Uuid,
    pub tenant_id: String,
    pub part_id: Uuid,
    pub description: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
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

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
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

#[derive(Debug, Clone, Serialize, Deserialize)]
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WhereUsedRow {
    pub bom_id: Uuid,
    pub part_id: Uuid,
    pub revision_id: Uuid,
    pub revision_label: String,
    pub quantity: f64,
    pub uom: Option<String>,
}

// Request types

#[derive(Debug, Deserialize)]
pub struct CreateBomRequest {
    pub part_id: Uuid,
    pub description: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct CreateRevisionRequest {
    pub revision_label: String,
}

#[derive(Debug, Deserialize)]
pub struct SetEffectivityRequest {
    pub effective_from: DateTime<Utc>,
    pub effective_to: Option<DateTime<Utc>>,
}

#[derive(Debug, Deserialize)]
pub struct AddLineRequest {
    pub component_item_id: Uuid,
    pub quantity: f64,
    pub uom: Option<String>,
    pub scrap_factor: Option<f64>,
    pub find_number: Option<i32>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateLineRequest {
    pub quantity: Option<f64>,
    pub uom: Option<String>,
    pub scrap_factor: Option<f64>,
    pub find_number: Option<i32>,
}

#[derive(Debug, Deserialize)]
pub struct ExplosionQuery {
    pub date: Option<DateTime<Utc>>,
    pub max_depth: Option<i32>,
}

#[derive(Debug, Deserialize)]
pub struct WhereUsedQuery {
    pub date: Option<DateTime<Utc>>,
}
