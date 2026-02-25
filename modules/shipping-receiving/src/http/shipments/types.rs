use axum::{http::StatusCode, Extension, Json};
use chrono::{DateTime, Utc};
use security::VerifiedClaims;
use serde::{Deserialize, Serialize};
use serde_json::json;
use uuid::Uuid;

use crate::domain::shipments::ShipmentError;
use axum::http::HeaderMap;
use axum::response::IntoResponse;

#[derive(Debug, Serialize)]
pub struct ErrorBody {
    pub error: String,
    pub message: String,
}

impl ErrorBody {
    pub fn new(error: &str, message: &str) -> Self {
        Self {
            error: error.to_string(),
            message: message.to_string(),
        }
    }
}

// ── Tenant extraction ────────────────────────────────────────

pub fn extract_tenant(
    claims: &Option<Extension<VerifiedClaims>>,
) -> Result<Uuid, (StatusCode, Json<ErrorBody>)> {
    match claims {
        Some(Extension(c)) => Ok(c.tenant_id),
        None => Err((
            StatusCode::UNAUTHORIZED,
            Json(ErrorBody::new("unauthorized", "Missing or invalid authentication")),
        )),
    }
}

pub fn idempotency_key(headers: &HeaderMap) -> Option<String> {
    headers
        .get("idempotency-key")
        .and_then(|v| v.to_str().ok())
        .map(String::from)
}

// ── Error mapping ────────────────────────────────────────────

pub fn error_response(err: ShipmentError) -> (StatusCode, Json<ErrorBody>) {
    match err {
        ShipmentError::NotFound => (
            StatusCode::NOT_FOUND,
            Json(ErrorBody::new("not_found", "Shipment not found")),
        ),
        ShipmentError::Validation(msg) => (
            StatusCode::BAD_REQUEST,
            Json(ErrorBody::new("validation_error", &msg)),
        ),
        ShipmentError::Transition(t) => (
            StatusCode::BAD_REQUEST,
            Json(ErrorBody::new("invalid_transition", &t.to_string())),
        ),
        ShipmentError::Guard(g) => (
            StatusCode::BAD_REQUEST,
            Json(ErrorBody::new("guard_failed", &g.to_string())),
        ),
        ShipmentError::Database(e) => {
            tracing::error!("database error: {e}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorBody::new("internal_error", "Internal server error")),
            )
        }
        ShipmentError::InventoryIntegration(msg) => {
            tracing::error!("inventory integration error: {msg}");
            (
                StatusCode::BAD_GATEWAY,
                Json(ErrorBody::new("inventory_error", "Inventory integration failed")),
            )
        }
    }
}

// ── Request / response types ─────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct CreateShipmentRequest {
    pub direction: crate::domain::shipments::Direction,
    pub carrier_party_id: Option<Uuid>,
    pub tracking_number: Option<String>,
    pub freight_cost_minor: Option<i64>,
    pub currency: Option<String>,
    pub expected_arrival_date: Option<DateTime<Utc>>,
}

#[derive(Debug, Deserialize)]
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

#[derive(Debug, Deserialize)]
pub struct TransitionStatusRequest {
    pub status: String,
    pub arrived_at: Option<DateTime<Utc>>,
    pub shipped_at: Option<DateTime<Utc>>,
    pub delivered_at: Option<DateTime<Utc>>,
    pub closed_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Deserialize)]
pub struct ReceiveLineRequest {
    pub qty_received: i64,
    pub qty_accepted: i64,
    pub qty_rejected: i64,
}

#[derive(Debug, Deserialize)]
pub struct ShipLineQtyRequest {
    pub qty_shipped: i64,
}

#[derive(Debug, Deserialize)]
pub struct ListShipmentsQuery {
    pub direction: Option<String>,
    pub status: Option<String>,
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

#[derive(Debug, Serialize, sqlx::FromRow)]
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
