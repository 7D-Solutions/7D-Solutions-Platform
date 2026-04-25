//! HTTP handler: POST /api/shipping-receiving/inbound-shipments/{id}/expected-tracking
//!
//! Records the carrier code and tracking number that a supplier has provided for
//! an inbound PO shipment. Once populated, the carrier webhook pipeline will
//! match events against expected_tracking_number and update latest_tracking_*
//! visibility fields.
//!
//! Invariant: this endpoint does NOT advance inbound_status. It is pure metadata
//! capture. State advance requires a dock-scan or manual receipt API call.

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Extension, Json,
};
use platform_http_contracts::ApiError;
use platform_sdk::extract_tenant;
use security::VerifiedClaims;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use uuid::Uuid;

use crate::AppState;

#[derive(Debug, Deserialize)]
pub struct ExpectedTrackingRequest {
    pub carrier_code: String,
    pub tracking_number: String,
}

#[derive(Debug, Serialize)]
pub struct ExpectedTrackingResponse {
    pub shipment_id: Uuid,
    pub expected_carrier_code: String,
    pub expected_tracking_number: String,
}

pub async fn set_expected_tracking(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(id): Path<Uuid>,
    Json(req): Json<ExpectedTrackingRequest>,
) -> impl IntoResponse {
    let tenant_id: Uuid = match extract_tenant(&claims).and_then(|s| {
        s.parse().map_err(|_| ApiError::bad_request("malformed tenant_id"))
    }) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };

    if req.carrier_code.trim().is_empty() {
        return ApiError::bad_request("carrier_code is required").into_response();
    }
    if req.tracking_number.trim().is_empty() {
        return ApiError::bad_request("tracking_number is required").into_response();
    }

    let result = sqlx::query(
        r#"
        UPDATE shipments
           SET expected_carrier_code    = $1,
               expected_tracking_number = $2
         WHERE id = $3
           AND tenant_id = $4
           AND direction = 'inbound'
        "#,
    )
    .bind(&req.carrier_code)
    .bind(&req.tracking_number)
    .bind(id)
    .bind(tenant_id)
    .execute(&state.pool)
    .await;

    match result {
        Ok(r) if r.rows_affected() == 0 => {
            ApiError::not_found("Inbound shipment not found").into_response()
        }
        Ok(_) => Json(ExpectedTrackingResponse {
            shipment_id: id,
            expected_carrier_code: req.carrier_code,
            expected_tracking_number: req.tracking_number,
        })
        .into_response(),
        Err(e) => {
            tracing::error!(error = %e, shipment_id = %id, "SR: set_expected_tracking: DB error");
            (StatusCode::INTERNAL_SERVER_ERROR, "internal error").into_response()
        }
    }
}
