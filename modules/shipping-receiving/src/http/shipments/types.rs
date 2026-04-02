use axum::Extension;
use chrono::{DateTime, Utc};
use event_bus::TracingContext;
use platform_http_contracts::ApiError;
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

use axum::http::HeaderMap;

pub fn idempotency_key(headers: &HeaderMap) -> Option<String> {
    headers
        .get("idempotency-key")
        .and_then(|v| v.to_str().ok())
        .map(String::from)
}

/// Enrich an `ApiError` with the `request_id` from `TracingContext`.
pub fn with_request_id(err: ApiError, ctx: &Option<Extension<TracingContext>>) -> ApiError {
    match ctx {
        Some(Extension(c)) => {
            if let Some(tid) = &c.trace_id {
                err.with_request_id(tid.clone())
            } else {
                err
            }
        }
        None => err,
    }
}

// ── Request / response types ─────────────────────────────────

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateShipmentRequest {
    pub direction: crate::domain::shipments::Direction,
    pub carrier_party_id: Option<Uuid>,
    pub tracking_number: Option<String>,
    pub freight_cost_minor: Option<i64>,
    pub currency: Option<String>,
    pub expected_arrival_date: Option<DateTime<Utc>>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct AddLineRequest {
    pub sku: Option<String>,
    pub uom: Option<String>,
    pub warehouse_id: Option<Uuid>,
    pub qty_expected: i64,
    pub source_ref_type: Option<String>,
    pub source_ref_id: Option<Uuid>,
    pub po_id: Option<Uuid>,
    pub po_line_id: Option<Uuid>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct TransitionStatusRequest {
    pub status: String,
    pub arrived_at: Option<DateTime<Utc>>,
    pub shipped_at: Option<DateTime<Utc>>,
    pub delivered_at: Option<DateTime<Utc>>,
    pub closed_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct ReceiveLineRequest {
    pub qty_received: i64,
    pub qty_accepted: i64,
    pub qty_rejected: i64,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct ShipLineQtyRequest {
    pub qty_shipped: i64,
}

#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct ListShipmentsQuery {
    pub direction: Option<String>,
    pub status: Option<String>,
    pub page: Option<i64>,
    pub page_size: Option<i64>,
}

#[derive(Debug, Serialize, sqlx::FromRow, ToSchema)]
pub struct ShipmentLineRow {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub shipment_id: Uuid,
    pub sku: Option<String>,
    pub uom: Option<String>,
    pub warehouse_id: Option<Uuid>,
    pub qty_expected: i64,
    pub qty_shipped: i64,
    pub qty_received: i64,
    pub qty_accepted: i64,
    pub qty_rejected: i64,
    pub source_ref_type: Option<String>,
    pub source_ref_id: Option<Uuid>,
    pub po_id: Option<Uuid>,
    pub po_line_id: Option<Uuid>,
    pub inventory_ref_id: Option<Uuid>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}
