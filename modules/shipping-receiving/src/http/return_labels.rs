//! HTTP handler for carrier return label generation.
//!
//! Route: POST /api/shipping-receiving/returns/label
//!
//! Calls the carrier API to generate a pre-paid return label, then emits a
//! ShippingCostIncurred cost event. The cost is seller-paid (platform tenant).
//!
//! Invariant: only seller-paid returns are in scope. Customer-paid (carrier-
//! billed) returns require a separate bead and are explicitly not supported here.

use axum::{extract::State, response::IntoResponse, Extension, Json};
use chrono::Utc;
use platform_http_contracts::ApiError;
use platform_sdk::extract_tenant;
use reqwest::Client;
use security::VerifiedClaims;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use uuid::Uuid;

use crate::domain::carrier_providers::{credentials, get_provider};
use crate::events::contracts::shipping_cost::{ShippingCostIncurredPayload, EVENT_TYPE_SHIPPING_COST_INCURRED};
use crate::outbox;
use crate::AppState;

// ── Request / Response ─────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct CreateReturnLabelRequest {
    pub carrier_code: String,

    // Return origin — the customer's address (packages are picked up here)
    pub from_name: String,
    pub from_address: String,
    pub from_city: String,
    pub from_state: String,
    pub from_zip: String,

    // Return destination — the warehouse address
    pub to_name: String,
    pub to_address: String,
    pub to_city: String,
    pub to_state: String,
    pub to_zip: String,

    // Shipment details
    pub weight_lbs: Option<f64>,
    pub freight_class: Option<String>,
    pub pieces: Option<u32>,
    pub description: Option<String>,

    // Cost the caller wants to record (from a prior rate quote)
    pub charge_minor: Option<i64>,
    pub currency: Option<String>,

    // Optional references
    pub original_tracking_number: Option<String>,
    pub rma_id: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ReturnLabelResponse {
    pub tracking_number: String,
    pub label_url: String,
    pub label_format: String,
    pub carrier_code: String,
    pub charge_minor: i64,
    pub currency: String,
    pub event_id: Option<Uuid>,
}

// ── Handler ────────────────────────────────────────────────────

pub async fn create_return_label(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Json(req): Json<CreateReturnLabelRequest>,
) -> impl IntoResponse {
    let app_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };

    // Resolve the carrier provider
    let provider = match get_provider(&req.carrier_code) {
        Some(p) => p,
        None => {
            return ApiError::bad_request(format!(
                "unknown carrier_code: {}",
                req.carrier_code
            ))
            .into_response();
        }
    };

    // Fetch carrier credentials from Integrations module
    let http_client = Client::new();
    let config = match credentials::get_carrier_credentials(
        &http_client,
        &app_id,
        &req.carrier_code,
    )
    .await
    {
        Ok(c) => c,
        Err(credentials::CredentialsError::NotFound(_)) => {
            return ApiError::new(
                412,
                "credentials_not_found",
                format!("No credentials configured for carrier '{}'", req.carrier_code),
            )
            .into_response();
        }
        Err(credentials::CredentialsError::MissingConfig) => {
            tracing::error!("INTEGRATIONS_SERVICE_URL not configured");
            return ApiError::internal("Integration service not reachable").into_response();
        }
        Err(e) => {
            tracing::error!(error = %e, carrier = %req.carrier_code, "failed to fetch carrier credentials");
            return ApiError::internal("Failed to fetch carrier credentials").into_response();
        }
    };

    // Build the request value that the provider expects (same shape as create_label)
    let provider_req = serde_json::json!({
        "from_name":    req.from_name,
        "from_address": req.from_address,
        "from_city":    req.from_city,
        "from_state":   req.from_state,
        "from_zip":     req.from_zip,
        "to_name":      req.to_name,
        "to_address":   req.to_address,
        "to_city":      req.to_city,
        "to_state":     req.to_state,
        "to_zip":       req.to_zip,
        "weight_lbs":   req.weight_lbs.unwrap_or(10.0),
        "freight_class": req.freight_class.as_deref().unwrap_or("70"),
        "pieces":       req.pieces.unwrap_or(1),
        "description":  req.description.as_deref().unwrap_or("Return Shipment"),
    });

    // Call the carrier API
    let label = match provider.create_return_label(&provider_req, &config).await {
        Ok(l) => l,
        Err(e) => {
            tracing::error!(error = %e, carrier = %req.carrier_code, "carrier return label API error");
            return ApiError::new(502, "carrier_error", e.to_string()).into_response();
        }
    };

    // Emit ShippingCostIncurred event if charge is provided
    let charge_minor = req.charge_minor.unwrap_or(0);
    let currency = req.currency.clone().unwrap_or_else(|| "USD".to_string());

    let event_id = if charge_minor > 0 {
        match emit_return_cost_event(
            &state,
            &app_id,
            &label.tracking_number,
            &req.carrier_code,
            charge_minor,
            &currency,
            req.rma_id.as_deref(),
        )
        .await
        {
            Ok(eid) => Some(eid),
            Err(e) => {
                // Non-fatal: label was created successfully; cost event failure is logged
                tracing::error!(
                    error = %e,
                    tracking_number = %label.tracking_number,
                    "failed to emit return label cost event"
                );
                None
            }
        }
    } else {
        None
    };

    Json(ReturnLabelResponse {
        tracking_number: label.tracking_number,
        label_url: label.label_data,
        label_format: label.label_format,
        carrier_code: label.carrier_code,
        charge_minor,
        currency,
        event_id,
    })
    .into_response()
}

async fn emit_return_cost_event(
    state: &AppState,
    app_id: &str,
    tracking_number: &str,
    carrier_code: &str,
    charge_minor: i64,
    currency: &str,
    rma_id: Option<&str>,
) -> Result<Uuid, sqlx::Error> {
    let tenant_id: Uuid = app_id
        .parse()
        .unwrap_or_else(|_| Uuid::new_v5(&Uuid::NAMESPACE_OID, app_id.as_bytes()));

    let event_id = Uuid::new_v4();
    let dummy_shipment_id = Uuid::new_v5(
        &Uuid::NAMESPACE_OID,
        format!("return:{}", tracking_number).as_bytes(),
    );

    let payload = ShippingCostIncurredPayload {
        tenant_id: tenant_id.to_string(),
        shipment_id: dummy_shipment_id,
        tracking_number: tracking_number.to_string(),
        carrier_code: carrier_code.to_string(),
        carrier_account_ref: None,
        direction: "return".to_string(),
        charge_minor,
        customer_charge_minor: None,
        currency: currency.to_string(),
        order_ref: rma_id.map(|s| s.to_string()),
        incurred_at: Utc::now(),
        correlation_id: event_id.to_string(),
    };

    let mut tx = state.pool.begin().await?;
    outbox::enqueue_event_tx(
        &mut tx,
        event_id,
        EVENT_TYPE_SHIPPING_COST_INCURRED,
        "shipment",
        &dummy_shipment_id.to_string(),
        &tenant_id.to_string(),
        &payload,
    )
    .await?;
    tx.commit().await?;

    Ok(event_id)
}
