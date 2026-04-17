//! Shared request/response types for the Sales Orders client.

use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

// ── Sales Orders ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SalesOrder {
    pub id: Uuid,
    pub tenant_id: String,
    pub order_number: String,
    pub status: String,
    pub customer_id: Option<Uuid>,
    pub party_id: Option<Uuid>,
    pub currency: String,
    pub subtotal_cents: i64,
    pub tax_cents: i64,
    pub total_cents: i64,
    pub order_date: NaiveDate,
    pub required_date: Option<NaiveDate>,
    pub promised_date: Option<NaiveDate>,
    pub external_quote_ref: Option<String>,
    pub blanket_order_id: Option<Uuid>,
    pub blanket_release_id: Option<Uuid>,
    pub notes: Option<String>,
    pub created_by: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SalesOrderLine {
    pub id: Uuid,
    pub tenant_id: String,
    pub sales_order_id: Uuid,
    pub line_number: i32,
    pub item_id: Option<Uuid>,
    pub part_number: Option<String>,
    pub description: String,
    pub uom: String,
    pub quantity: f64,
    pub unit_price_cents: i64,
    pub line_total_cents: i64,
    pub required_date: Option<NaiveDate>,
    pub promised_date: Option<NaiveDate>,
    pub shipped_qty: f64,
    pub warehouse_id: Option<Uuid>,
    pub reservation_id: Option<Uuid>,
    pub invoiced_at: Option<DateTime<Utc>>,
    pub notes: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SalesOrderWithLines {
    #[serde(flatten)]
    pub order: SalesOrder,
    pub lines: Vec<SalesOrderLine>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CreateOrderRequest {
    pub customer_id: Option<Uuid>,
    pub party_id: Option<Uuid>,
    pub currency: String,
    pub order_date: Option<NaiveDate>,
    pub required_date: Option<NaiveDate>,
    pub promised_date: Option<NaiveDate>,
    pub external_quote_ref: Option<String>,
    pub notes: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct UpdateOrderRequest {
    pub customer_id: Option<Uuid>,
    pub party_id: Option<Uuid>,
    pub required_date: Option<NaiveDate>,
    pub promised_date: Option<NaiveDate>,
    pub external_quote_ref: Option<String>,
    pub notes: Option<String>,
    pub tax_cents: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CreateOrderLineRequest {
    pub item_id: Option<Uuid>,
    pub part_number: Option<String>,
    pub description: String,
    pub uom: Option<String>,
    pub quantity: f64,
    pub unit_price_cents: i64,
    pub required_date: Option<NaiveDate>,
    pub promised_date: Option<NaiveDate>,
    pub warehouse_id: Option<Uuid>,
    pub notes: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ListOrdersQuery {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub customer_id: Option<Uuid>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub blanket_order_id: Option<Uuid>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from_date: Option<NaiveDate>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub to_date: Option<NaiveDate>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub offset: Option<i64>,
}

// ── Blanket Orders ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlanketOrder {
    pub id: Uuid,
    pub tenant_id: String,
    pub blanket_number: String,
    pub status: String,
    pub customer_id: Option<Uuid>,
    pub party_id: Option<Uuid>,
    pub currency: String,
    pub committed_cents: i64,
    pub released_cents: i64,
    pub effective_date: NaiveDate,
    pub expiry_date: Option<NaiveDate>,
    pub notes: Option<String>,
    pub created_by: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CreateBlanketRequest {
    pub customer_id: Option<Uuid>,
    pub party_id: Option<Uuid>,
    pub currency: String,
    pub effective_date: Option<NaiveDate>,
    pub expiry_date: Option<NaiveDate>,
    pub notes: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CreateReleaseRequest {
    pub blanket_line_id: Uuid,
    pub release_qty: f64,
    pub release_date: Option<NaiveDate>,
    pub notes: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlanketOrderRelease {
    pub id: Uuid,
    pub tenant_id: String,
    pub blanket_order_id: Uuid,
    pub blanket_line_id: Uuid,
    pub sales_order_id: Option<Uuid>,
    pub status: String,
    pub release_qty: f64,
    pub release_date: NaiveDate,
    pub notes: Option<String>,
    pub created_at: DateTime<Utc>,
}
