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

// ============================================================================
// MRP (Net Requirements) models
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize, FromRow, ToSchema)]
pub struct MrpSnapshot {
    pub id: Uuid,
    pub tenant_id: String,
    pub bom_id: Uuid,
    pub demand_quantity: f64,
    pub effectivity_date: DateTime<Utc>,
    #[schema(value_type = Object)]
    pub on_hand_snapshot: serde_json::Value,
    pub created_at: DateTime<Utc>,
    pub created_by: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow, ToSchema)]
pub struct MrpRequirementLine {
    pub id: i64,
    pub snapshot_id: Uuid,
    pub level: i32,
    pub parent_part_id: Uuid,
    pub component_item_id: Uuid,
    pub gross_quantity: f64,
    pub scrap_factor: f64,
    pub scrap_adjusted_quantity: f64,
    pub on_hand_quantity: f64,
    pub net_quantity: f64,
    pub uom: Option<String>,
    pub revision_id: Uuid,
    pub revision_label: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct MrpSnapshotWithLines {
    #[serde(flatten)]
    pub snapshot: MrpSnapshot,
    pub lines: Vec<MrpRequirementLine>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct OnHandEntry {
    pub item_id: Uuid,
    pub quantity: f64,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct MrpExplodeRequest {
    pub bom_id: Uuid,
    pub demand_quantity: f64,
    pub effectivity_date: DateTime<Utc>,
    pub on_hand: Vec<OnHandEntry>,
    pub created_by: String,
}

#[derive(Debug, Deserialize, IntoParams)]
pub struct MrpSnapshotListQuery {
    pub bom_id: Option<Uuid>,
    #[serde(default = "default_page")]
    pub page: i64,
    #[serde(default = "default_page_size")]
    pub page_size: i64,
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

/// Resolved item details embedded in a BOM line when `?include=item_details` is requested.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ItemDetails {
    pub item_id: Uuid,
    pub sku: String,
    pub name: String,
    pub description: Option<String>,
    /// Standard or last-known unit cost in minor currency units (e.g. cents).
    /// `None` when the inventory service does not expose a cost for this item.
    pub unit_cost_minor: Option<i64>,
}

/// BOM line with optional embedded item details (returned when `?include=item_details`).
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct BomLineEnriched {
    pub id: Uuid,
    pub revision_id: Uuid,
    pub tenant_id: String,
    pub component_item_id: Uuid,
    pub quantity: f64,
    pub uom: Option<String>,
    pub scrap_factor: Option<f64>,
    pub find_number: Option<i32>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
    /// Resolved item details. `None` when the part_id does not resolve in inventory.
    pub item: Option<ItemDetails>,
}

// ============================================================================
// Kit Readiness models
// ============================================================================

#[derive(Debug, Deserialize, ToSchema)]
pub struct KitReadinessCheckRequest {
    pub bom_id: Uuid,
    pub required_quantity: f64,
    pub check_date: DateTime<Utc>,
    pub created_by: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow, ToSchema)]
pub struct KitReadinessSnapshot {
    pub id: Uuid,
    pub tenant_id: String,
    pub bom_id: Uuid,
    pub required_quantity: f64,
    pub check_date: DateTime<Utc>,
    pub overall_status: String,
    #[schema(value_type = Object)]
    pub issue_summary: serde_json::Value,
    pub created_by: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct KitReadinessLine {
    pub component_item_id: Uuid,
    pub required_qty: f64,
    pub on_hand_qty: i64,
    pub expired_qty: i64,
    pub available_qty: i64,
    pub status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct KitReadinessResult {
    #[serde(flatten)]
    pub snapshot: KitReadinessSnapshot,
    pub lines: Vec<KitReadinessLine>,
}

/// Query parameters for the `GET /api/bom/revisions/{revision_id}/lines` endpoint.
///
/// Extends pagination with an optional `include` flag.
#[derive(Debug, Deserialize, IntoParams)]
pub struct LinesQuery {
    #[serde(default = "default_page")]
    pub page: i64,
    #[serde(default = "default_page_size")]
    pub page_size: i64,
    /// Pass `item_details` to embed resolved inventory item data in each line.
    pub include: Option<String>,
}
