use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Extension, Json,
};
use chrono::Utc;
use event_bus::TracingContext;
use platform_http_contracts::ApiError;
use platform_sdk::extract_tenant;
use security::VerifiedClaims;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use utoipa::ToSchema;
use uuid::Uuid;

use crate::domain::shipments::{ShipmentError, ShipmentService};
use crate::events::contracts::shipping_cost::{
    build_shipping_cost_incurred_envelope, ShippingCostIncurredPayload,
    EVENT_TYPE_SHIPPING_COST_INCURRED,
};
use crate::outbox;
use crate::AppState;

use super::types::with_request_id;

// ============================================================================
// Request / Response types
// ============================================================================

/// Request body for recording a carrier label cost on a shipment.
#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateLabelRequest {
    /// Master tracking number returned by the carrier.
    pub tracking_number: String,
    /// ups | fedex | usps | rl | xpo | odfl | saia
    pub carrier_code: String,
    /// Per-shipment carrier account reference (billing attribution).
    pub carrier_account_ref: Option<String>,
    /// "outbound" | "return"
    pub direction: String,
    /// What the carrier is charging us, in minor currency units (AP obligation).
    pub charge_minor: i64,
    /// What we are charging the customer, in minor units. None = free shipping.
    pub customer_charge_minor: Option<i64>,
    /// ISO 4217 currency code.
    pub currency: String,
    /// Order or invoice reference in AR for automatic line attachment.
    pub order_ref: Option<String>,
}

/// Response returned after recording a label cost.
#[derive(Debug, Serialize, ToSchema)]
pub struct LabelCostResponse {
    pub event_id: Uuid,
    pub shipment_id: Uuid,
    pub tracking_number: String,
    pub carrier_code: String,
    pub charge_minor: i64,
    pub customer_charge_minor: Option<i64>,
    pub currency: String,
    pub incurred_at: chrono::DateTime<Utc>,
}

// ============================================================================
// Domain service (testable without HTTP)
// ============================================================================

/// Record a shipping label cost into the sr_events_outbox within a transaction.
///
/// Emits `shipping_receiving.shipping_cost.incurred` atomically with the caller's
/// transaction. Returns the assigned event_id.
///
/// Invariant: one call per logical shipment (master). Do not call per child package.
pub async fn record_label_cost_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    shipment_id: Uuid,
    tenant_id: Uuid,
    req: &CreateLabelRequest,
    correlation_id: &str,
) -> Result<Uuid, sqlx::Error> {
    let event_id = Uuid::new_v4();
    let incurred_at = Utc::now();

    let payload = ShippingCostIncurredPayload {
        tenant_id: tenant_id.to_string(),
        shipment_id,
        tracking_number: req.tracking_number.clone(),
        carrier_code: req.carrier_code.clone(),
        carrier_account_ref: req.carrier_account_ref.clone(),
        direction: req.direction.clone(),
        charge_minor: req.charge_minor,
        customer_charge_minor: req.customer_charge_minor,
        currency: req.currency.clone(),
        order_ref: req.order_ref.clone(),
        incurred_at,
        correlation_id: correlation_id.to_string(),
    };

    let _envelope = build_shipping_cost_incurred_envelope(
        event_id,
        tenant_id.to_string(),
        correlation_id.to_string(),
        None,
        payload.clone(),
    );

    outbox::enqueue_event_tx(
        tx,
        event_id,
        EVENT_TYPE_SHIPPING_COST_INCURRED,
        "shipment",
        &shipment_id.to_string(),
        &tenant_id.to_string(),
        &payload,
    )
    .await?;

    Ok(event_id)
}

// ============================================================================
// HTTP handler
// ============================================================================

#[utoipa::path(
    post,
    path = "/api/shipping-receiving/shipments/{id}/label",
    tag = "Shipments",
    params(("id" = Uuid, Path, description = "Shipment ID")),
    request_body = CreateLabelRequest,
    responses(
        (status = 201, description = "Label cost recorded", body = LabelCostResponse),
        (status = 400, description = "Validation error", body = ApiError),
        (status = 404, description = "Shipment not found", body = ApiError),
        (status = 500, description = "Internal error", body = ApiError),
    ),
    security(("bearer" = [])),
)]
pub async fn create_label(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    tracing_ctx: Option<Extension<TracingContext>>,
    Path(id): Path<Uuid>,
    Json(req): Json<CreateLabelRequest>,
) -> impl IntoResponse {
    let tenant_id: Uuid = match extract_tenant(&claims).and_then(|s| {
        s.parse()
            .map_err(|_| ApiError::bad_request("malformed tenant_id"))
    }) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };

    if req.charge_minor < 0 {
        return with_request_id(
            ApiError::bad_request("charge_minor must be >= 0"),
            &tracing_ctx,
        )
        .into_response();
    }

    if let Some(cc) = req.customer_charge_minor {
        if cc < 0 {
            return with_request_id(
                ApiError::bad_request("customer_charge_minor must be >= 0"),
                &tracing_ctx,
            )
            .into_response();
        }
    }

    if !["outbound", "return"].contains(&req.direction.as_str()) {
        return with_request_id(
            ApiError::bad_request("direction must be 'outbound' or 'return'"),
            &tracing_ctx,
        )
        .into_response();
    }

    match ShipmentService::find_by_id(&state.pool, id, tenant_id).await {
        Ok(None) => {
            return with_request_id(ApiError::from(ShipmentError::NotFound), &tracing_ctx)
                .into_response();
        }
        Err(e) => {
            return with_request_id(ApiError::from(e), &tracing_ctx).into_response();
        }
        Ok(Some(_)) => {}
    }

    let correlation_id = tracing_ctx
        .as_ref()
        .and_then(|Extension(c)| c.trace_id.clone())
        .unwrap_or_else(|| Uuid::new_v4().to_string());

    let mut tx = match state.pool.begin().await {
        Ok(tx) => tx,
        Err(e) => {
            return with_request_id(
                ApiError::from(ShipmentError::Database(e)),
                &tracing_ctx,
            )
            .into_response();
        }
    };

    let event_id =
        match record_label_cost_tx(&mut tx, id, tenant_id, &req, &correlation_id).await {
            Ok(eid) => eid,
            Err(e) => {
                return with_request_id(
                    ApiError::from(ShipmentError::Database(e)),
                    &tracing_ctx,
                )
                .into_response();
            }
        };

    if let Err(e) = tx.commit().await {
        return with_request_id(
            ApiError::from(ShipmentError::Database(e)),
            &tracing_ctx,
        )
        .into_response();
    }

    let resp = LabelCostResponse {
        event_id,
        shipment_id: id,
        tracking_number: req.tracking_number.clone(),
        carrier_code: req.carrier_code.clone(),
        charge_minor: req.charge_minor,
        customer_charge_minor: req.customer_charge_minor,
        currency: req.currency.clone(),
        incurred_at: Utc::now(),
    };

    (StatusCode::CREATED, Json(resp)).into_response()
}
