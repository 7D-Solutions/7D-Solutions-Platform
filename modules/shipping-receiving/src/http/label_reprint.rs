//! HTTP handler for carrier label reprint.
//!
//! Route: GET /api/shipping-receiving/shipments/{shipment_id}/label
//!
//! Invariant: no local PDF storage. Every reprint is a live pass-through to the
//! carrier. Carriers are the system of record for label PDFs.
//!
//! When a carrier has purged the label (USPS after ~30 days; rare in practice),
//! returns a structured JSON error the vertical UI can display gracefully.

use axum::{
    body::Body,
    extract::{Path, State},
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    Extension, Json,
};
use platform_http_contracts::ApiError;
use platform_sdk::extract_tenant;
use reqwest::Client;
use security::VerifiedClaims;
use serde::Serialize;
use std::sync::Arc;
use uuid::Uuid;

use crate::db::carrier_request_repo::CarrierRequestRepo;
use crate::db::repository::ShipmentRepository;
use crate::domain::carrier_providers::{credentials, get_provider, CarrierProviderError};
use crate::AppState;

// ── Structured error for purged labels ────────────────────────

#[derive(Debug, Serialize)]
pub struct LabelPurgedError {
    pub error: &'static str,
    pub carrier_code: String,
    pub tracking_number: String,
    /// Carrier's stated purge window in days (null = unknown).
    pub purge_window_days: Option<u32>,
}

fn purge_window_for(carrier_code: &str) -> Option<u32> {
    match carrier_code {
        "usps" => Some(30),
        _ => None,
    }
}

// ── Handler ───────────────────────────────────────────────────

pub async fn reprint_label(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(shipment_id): Path<Uuid>,
) -> impl IntoResponse {
    let tenant_id: Uuid = match extract_tenant(&claims).and_then(|s| {
        s.parse()
            .map_err(|_| ApiError::bad_request("malformed tenant_id"))
    }) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };

    // 1. Look up the shipment to confirm it exists and get the tracking number.
    let shipment = match ShipmentRepository::get_shipment(&state.pool, shipment_id, tenant_id).await
    {
        Ok(Some(s)) => s,
        Ok(None) => {
            return ApiError::new(404, "shipment_not_found", "Shipment not found").into_response()
        }
        Err(e) => {
            tracing::error!(error = %e, shipment_id = %shipment_id, "DB error looking up shipment");
            return ApiError::internal("Failed to look up shipment").into_response();
        }
    };

    let tracking_number = match shipment.tracking_number {
        Some(tn) if !tn.is_empty() => tn,
        _ => {
            return ApiError::new(
                409,
                "no_tracking_number",
                "Shipment has no tracking number — label was not created via this platform",
            )
            .into_response()
        }
    };

    // 2. Find the carrier_code from the most recent label request.
    let carrier_request = match CarrierRequestRepo::find_latest_label(
        &state.pool,
        shipment_id,
        tenant_id,
    )
    .await
    {
        Ok(Some(cr)) => cr,
        Ok(None) => {
            return ApiError::new(
                409,
                "no_label_request",
                "No carrier label request found for this shipment",
            )
            .into_response()
        }
        Err(e) => {
            tracing::error!(error = %e, shipment_id = %shipment_id, "DB error looking up carrier request");
            return ApiError::internal("Failed to look up carrier request").into_response();
        }
    };

    let carrier_code = &carrier_request.carrier_code;

    // 3. Resolve the carrier provider.
    let provider = match get_provider(carrier_code) {
        Some(p) => p,
        None => {
            return ApiError::new(
                409,
                "unknown_carrier",
                format!("Carrier '{carrier_code}' is not supported"),
            )
            .into_response()
        }
    };

    // 4. Fetch carrier credentials.
    let http_client = Client::new();
    let config = match credentials::get_carrier_credentials(&http_client, &tenant_id.to_string(), carrier_code).await {
        Ok(c) => c,
        Err(credentials::CredentialsError::NotFound(_)) => {
            return ApiError::new(
                412,
                "credentials_not_found",
                format!("No credentials configured for carrier '{carrier_code}'"),
            )
            .into_response();
        }
        Err(credentials::CredentialsError::MissingConfig) => {
            tracing::error!("INTEGRATIONS_SERVICE_URL not configured");
            return ApiError::internal("Integration service not reachable").into_response();
        }
        Err(e) => {
            tracing::error!(error = %e, carrier = %carrier_code, "failed to fetch carrier credentials");
            return ApiError::internal("Failed to fetch carrier credentials").into_response();
        }
    };

    // 5. Call the carrier API to fetch the label PDF.
    let result = provider.fetch_label(&tracking_number, &config).await;

    match result {
        Ok(pdf) => {
            state
                .metrics
                .label_reprint_total
                .with_label_values(&[carrier_code, "ok"])
                .inc();

            let filename = format!("label-{tracking_number}.pdf");
            Response::builder()
                .status(StatusCode::OK)
                .header(header::CONTENT_TYPE, &pdf.content_type)
                .header(
                    header::CONTENT_DISPOSITION,
                    format!("inline; filename=\"{filename}\""),
                )
                .body(Body::from(pdf.pdf_bytes))
                .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
        }

        Err(CarrierProviderError::NotFound(_)) => {
            state
                .metrics
                .label_reprint_total
                .with_label_values(&[carrier_code, "carrier_not_found"])
                .inc();

            let body = LabelPurgedError {
                error: "label_purged_by_carrier",
                carrier_code: carrier_code.clone(),
                tracking_number: tracking_number.clone(),
                purge_window_days: purge_window_for(carrier_code),
            };
            (StatusCode::GONE, Json(body)).into_response()
        }

        Err(e) => {
            state
                .metrics
                .label_reprint_total
                .with_label_values(&[carrier_code, "carrier_error"])
                .inc();

            tracing::error!(
                error = %e,
                carrier = %carrier_code,
                tracking_number = %tracking_number,
                "carrier label reprint API error"
            );
            ApiError::new(502, "carrier_error", e.to_string()).into_response()
        }
    }
}
