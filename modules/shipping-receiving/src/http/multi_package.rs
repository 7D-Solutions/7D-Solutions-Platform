//! HTTP handler: POST /api/shipping-receiving/shipments/multi-package
//!
//! Creates a multi-package shipment (master row + child rows) in one transaction.
//! Calls the carrier API synchronously and emits ONE ShippingCostIncurred event
//! for the master shipment — not per child.
//!
//! Invariant: one logical shipment = one billing event. Children share the master's
//! tracking number for AR/AP purposes. This endpoint enforces that.

use axum::{
    extract::State,
    http::StatusCode,
    response::IntoResponse,
    Extension, Json,
};
use chrono::Utc;
use event_bus::TracingContext;
use platform_http_contracts::ApiError;
use platform_sdk::extract_tenant;
use reqwest::Client;
use security::VerifiedClaims;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use uuid::Uuid;

use crate::domain::carrier_providers::{
    credentials, get_provider, MultiPackageLabelRequest, MultiPackageLabelResponse, PackageInfo,
};
use crate::events::contracts::shipping_cost::{
    build_shipping_cost_incurred_envelope, ShippingCostIncurredPayload,
    EVENT_TYPE_SHIPPING_COST_INCURRED,
};
use crate::outbox;
use crate::AppState;

// ── Request / Response types ──────────────────────────────────

/// HTTP request body for multi-package label creation.
#[derive(Debug, Deserialize)]
pub struct MultiPackageRequest {
    /// Carrier code: ups | fedex | rl | xpo | odfl | saia
    pub carrier_code: String,
    /// One entry per physical package.
    pub packages: Vec<PackageInfoDto>,
    /// Origin address fields: name, address, city, state, zip
    pub origin: serde_json::Value,
    /// Destination address fields: name, address, city, state, zip
    pub destination: serde_json::Value,
    /// Optional carrier service level (e.g. "03" for UPS Ground, "FEDEX_GROUND")
    pub service_level: Option<String>,
    /// Carrier account billing reference
    pub billing_ref: Option<String>,
    /// What the carrier charges us (AP cost), in minor currency units (cents)
    pub charge_minor: i64,
    /// ISO 4217 currency code
    pub currency: String,
    /// Optional: what we charge the customer (AR line), None = free shipping
    pub customer_charge_minor: Option<i64>,
    /// Optional: order or invoice reference for AR attachment
    pub order_ref: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct PackageInfoDto {
    pub weight_lbs: f64,
    pub length_in: f64,
    pub width_in: f64,
    pub height_in: f64,
    pub declared_value_cents: Option<i64>,
}

/// HTTP response returned after a successful multi-package label creation.
#[derive(Debug, Serialize)]
pub struct MultiPackageLabelHttpResponse {
    pub master_shipment_id: Uuid,
    pub master_tracking_number: String,
    pub carrier_code: String,
    pub package_count: usize,
    pub cost_event_id: Uuid,
    pub children: Vec<ChildShipmentDto>,
}

#[derive(Debug, Serialize)]
pub struct ChildShipmentDto {
    pub shipment_id: Uuid,
    pub tracking_number: String,
    pub package_index: usize,
}

// ── Handler ───────────────────────────────────────────────────

pub async fn create_multi_package_shipment(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    tracing_ctx: Option<Extension<TracingContext>>,
    Json(req): Json<MultiPackageRequest>,
) -> impl IntoResponse {
    let tenant_id: Uuid = match extract_tenant(&claims).and_then(|s| {
        s.parse()
            .map_err(|_| ApiError::bad_request("malformed tenant_id"))
    }) {
        Ok(id) => id,
        Err(e) => return (StatusCode::UNAUTHORIZED, Json(serde_json::json!({"error": e.to_string()}))).into_response(),
    };

    if req.packages.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "packages must not be empty"})),
        )
            .into_response();
    }

    if req.charge_minor < 0 {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "charge_minor must be >= 0"})),
        )
            .into_response();
    }

    let provider = match get_provider(&req.carrier_code) {
        Some(p) => p,
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": format!("unknown carrier_code: {}", req.carrier_code)})),
            )
                .into_response()
        }
    };

    // Fetch carrier credentials from the Integrations service.
    let http_client = Client::new();
    let carrier_config = match credentials::get_carrier_credentials(
        &http_client,
        &tenant_id.to_string(),
        &req.carrier_code,
    )
    .await
    {
        Ok(cfg) => cfg,
        Err(credentials::CredentialsError::MissingConfig) => {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(serde_json::json!({"error": "INTEGRATIONS_SERVICE_URL not configured"})),
            )
                .into_response()
        }
        Err(credentials::CredentialsError::NotFound(_)) => {
            return (
                StatusCode::UNPROCESSABLE_ENTITY,
                Json(serde_json::json!({"error": format!("no credentials found for carrier {}", req.carrier_code)})),
            )
                .into_response()
        }
        Err(e) => {
            return (
                StatusCode::BAD_GATEWAY,
                Json(serde_json::json!({"error": format!("credentials fetch failed: {e}")})),
            )
                .into_response()
        }
    };

    let label_req = MultiPackageLabelRequest {
        packages: req
            .packages
            .iter()
            .map(|p| PackageInfo {
                weight_lbs: p.weight_lbs,
                length_in: p.length_in,
                width_in: p.width_in,
                height_in: p.height_in,
                declared_value_cents: p.declared_value_cents,
            })
            .collect(),
        origin: req.origin.clone(),
        destination: req.destination.clone(),
        service_level: req.service_level.clone(),
        billing_ref: req.billing_ref.clone(),
    };

    let label_resp: MultiPackageLabelResponse =
        match provider.create_multi_package_label(&label_req, &carrier_config).await {
            Ok(r) => r,
            Err(e) => {
                return (
                    StatusCode::BAD_GATEWAY,
                    Json(serde_json::json!({"error": format!("carrier API error: {e}")})),
                )
                    .into_response()
            }
        };

    let package_count = req.packages.len();
    let correlation_id = tracing_ctx
        .as_ref()
        .and_then(|Extension(c)| c.trace_id.clone())
        .unwrap_or_else(|| Uuid::new_v4().to_string());

    let mut tx = match state.pool.begin().await {
        Ok(tx) => tx,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": format!("db error: {e}")})),
            )
                .into_response()
        }
    };

    // Insert master shipment row.
    let master_id: Uuid = match sqlx::query_scalar(
        r#"
        INSERT INTO shipments
            (tenant_id, direction, status, tracking_number,
             master_tracking_number, package_count, carrier_party_id)
        VALUES ($1, 'outbound', 'draft', $2, $3, $4, NULL)
        RETURNING id
        "#,
    )
    .bind(tenant_id)
    .bind(&label_resp.master_tracking_number)
    .bind(&label_resp.master_tracking_number)
    .bind(package_count as i32)
    .fetch_one(&mut *tx)
    .await
    {
        Ok(id) => id,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": format!("db error inserting master: {e}")})),
            )
                .into_response()
        }
    };

    // Insert child shipment rows (one per child label returned by carrier).
    let mut children_out: Vec<ChildShipmentDto> = Vec::with_capacity(label_resp.children.len());
    for child in &label_resp.children {
        let child_id: Uuid = match sqlx::query_scalar(
            r#"
            INSERT INTO shipments
                (tenant_id, direction, status, tracking_number,
                 parent_shipment_id, package_count)
            VALUES ($1, 'outbound', 'draft', $2, $3, 1)
            RETURNING id
            "#,
        )
        .bind(tenant_id)
        .bind(&child.tracking_number)
        .bind(master_id)
        .fetch_one(&mut *tx)
        .await
        {
            Ok(id) => id,
            Err(e) => {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({"error": format!("db error inserting child: {e}")})),
                )
                    .into_response()
            }
        };
        children_out.push(ChildShipmentDto {
            shipment_id: child_id,
            tracking_number: child.tracking_number.clone(),
            package_index: child.package_index,
        });
    }

    // Emit ONE ShippingCostIncurred for the master — never per child.
    let event_id = Uuid::new_v4();
    let cost_payload = ShippingCostIncurredPayload {
        tenant_id: tenant_id.to_string(),
        shipment_id: master_id,
        tracking_number: label_resp.master_tracking_number.clone(),
        carrier_code: req.carrier_code.clone(),
        carrier_account_ref: req.billing_ref.clone(),
        direction: "outbound".to_string(),
        charge_minor: req.charge_minor,
        customer_charge_minor: req.customer_charge_minor,
        currency: req.currency.clone(),
        order_ref: req.order_ref.clone(),
        incurred_at: Utc::now(),
        correlation_id: correlation_id.clone(),
    };

    let _envelope = build_shipping_cost_incurred_envelope(
        event_id,
        tenant_id.to_string(),
        correlation_id.clone(),
        None,
        cost_payload.clone(),
    );

    if let Err(e) = outbox::enqueue_event_tx(
        &mut tx,
        event_id,
        EVENT_TYPE_SHIPPING_COST_INCURRED,
        "shipment",
        &master_id.to_string(),
        &tenant_id.to_string(),
        &cost_payload,
    )
    .await
    {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": format!("outbox error: {e}")})),
        )
            .into_response();
    }

    if let Err(e) = tx.commit().await {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": format!("commit error: {e}")})),
        )
            .into_response();
    }

    (
        StatusCode::CREATED,
        Json(MultiPackageLabelHttpResponse {
            master_shipment_id: master_id,
            master_tracking_number: label_resp.master_tracking_number,
            carrier_code: req.carrier_code,
            package_count,
            cost_event_id: event_id,
            children: children_out,
        }),
    )
        .into_response()
}
