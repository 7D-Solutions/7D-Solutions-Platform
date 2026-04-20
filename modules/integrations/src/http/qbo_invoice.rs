//! HTTP handler for QBO invoice sparse updates.
//!
//! Route: POST /api/integrations/qbo/invoice/{invoice_id}/update
//!
//! Allows callers to update shipping fields on a QBO invoice without
//! touching other fields. Uses the platform-owned OAuth connection.

use axum::{
    extract::{Path, State},
    response::IntoResponse,
    Extension, Json,
};
use platform_http_contracts::ApiError;
use security::VerifiedClaims;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::Arc;
use utoipa::ToSchema;
use uuid::Uuid;

use crate::domain::oauth::{service as oauth_service, OAuthError};
use crate::domain::qbo::{client::QboClient, QboError, TokenProvider};
use crate::AppState;
use platform_sdk::extract_tenant;

// ============================================================================
// Request / Response
// ============================================================================

#[derive(Debug, Deserialize, ToSchema)]
pub struct UpdateInvoiceRequest {
    /// Ship date in YYYY-MM-DD format
    pub ship_date: Option<String>,
    /// Tracking number
    pub tracking_num: Option<String>,
    /// Carrier name (e.g., "FedEx", "UPS")
    pub carrier: Option<String>,
}

impl UpdateInvoiceRequest {
    fn has_fields(&self) -> bool {
        self.ship_date.is_some() || self.tracking_num.is_some() || self.carrier.is_some()
    }
}

#[derive(Debug, Serialize, ToSchema)]
pub struct UpdateInvoiceResponse {
    pub invoice_id: String,
    pub ship_date: Option<String>,
    pub tracking_num: Option<String>,
    pub carrier: Option<String>,
    pub sync_token: String,
}

// ============================================================================
// Error handling
// ============================================================================

fn qbo_error(e: QboError) -> ApiError {
    match e {
        QboError::SyncTokenExhausted(_) => {
            ApiError::conflict("Invoice was modified concurrently — retry the request")
        }
        QboError::RateLimited { .. } => ApiError::new(429, "rate_limited", "QBO rate limit exceeded"),
        QboError::AuthFailed => ApiError::new(
            502,
            "auth_failed",
            "QBO authentication failed — connection may need reauthorization",
        ),
        QboError::TokenError(msg) => ApiError::new(502, "token_error", msg),
        QboError::ApiFault {
            fault_type,
            message,
            code,
            ..
        } => {
            let status = if fault_type.to_lowercase().contains("validation") {
                422
            } else {
                502
            };
            ApiError::new(
                status,
                "qbo_fault",
                format!("{}: {} (code {})", fault_type, message, code),
            )
        }
        QboError::Http(e) => ApiError::new(502, "network_error", e.to_string()),
        QboError::Deserialize(msg) => ApiError::new(502, "parse_error", msg),
    }
}

fn oauth_err(e: OAuthError) -> ApiError {
    match e {
        OAuthError::NotFound => {
            ApiError::not_found("No QuickBooks connection found for this tenant")
        }
        OAuthError::MissingEncryptionKey => {
            tracing::error!(
                error_code = "OPERATION_FAILED",
                "OAUTH_ENCRYPTION_KEY not set"
            );
            ApiError::internal("Server misconfiguration")
        }
        OAuthError::Database(e) => {
            tracing::error!(error = %e, "OAuth DB error");
            ApiError::internal("Internal database error")
        }
        _ => ApiError::internal("Internal error"),
    }
}

// ============================================================================
// DB-backed TokenProvider (same pattern as cdc.rs)
// ============================================================================

struct DbTokenProvider {
    pool: sqlx::PgPool,
    app_id: String,
}

#[async_trait::async_trait]
impl TokenProvider for DbTokenProvider {
    async fn get_token(&self) -> Result<String, QboError> {
        oauth_service::get_access_token(&self.pool, &self.app_id, "quickbooks")
            .await
            .map_err(|e| QboError::TokenError(e.to_string()))
    }

    async fn refresh_token(&self) -> Result<String, QboError> {
        // Token refresh is handled by the background refresh worker.
        // If we get here, the connection needs reauthorization.
        Err(QboError::AuthFailed)
    }
}

// ============================================================================
// Handler
// ============================================================================

#[utoipa::path(
    post,
    path = "/api/integrations/qbo/invoice/{invoice_id}/update",
    params(("invoice_id" = String, Path, description = "QBO invoice ID")),
    request_body = UpdateInvoiceRequest,
    responses(
        (status = 200, description = "Invoice updated", body = UpdateInvoiceResponse),
        (status = 400, description = "No fields provided"),
        (status = 404, description = "Invoice or connection not found"),
        (status = 409, description = "Concurrent modification"),
        (status = 412, description = "QBO connection not active"),
    ),
    security(("bearer" = [])),
    tag = "QBO Invoice"
)]
pub async fn update_invoice(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(invoice_id): Path<String>,
    Json(req): Json<UpdateInvoiceRequest>,
) -> impl IntoResponse {
    // 1. Validate request has at least one field
    if !req.has_fields() {
        return ApiError::bad_request(
            "At least one of ship_date, tracking_num, or carrier is required",
        )
        .into_response();
    }

    // 2. Extract tenant from JWT
    let app_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };

    // 3. Look up QBO connection
    let connection =
        match oauth_service::get_connection_status(&state.pool, &app_id, "quickbooks").await {
            Ok(Some(c)) => c,
            Ok(None) => {
                return ApiError::not_found("No QuickBooks connection found for this tenant")
                    .into_response()
            }
            Err(e) => return oauth_err(e).into_response(),
        };

    // 4. Validate connection is active
    if connection.connection_status != "connected" {
        return ApiError::new(
            412,
            "not_connected",
            format!(
                "QuickBooks connection is '{}' — reconnection required",
                connection.connection_status
            ),
        )
        .into_response();
    }

    // 5. Create QBO client with DB-backed token provider
    let base_url = crate::domain::qbo::cdc::qbo_base_url();

    let tokens: Arc<dyn TokenProvider> = Arc::new(DbTokenProvider {
        pool: state.pool.clone(),
        app_id: app_id.clone(),
    });

    let client = QboClient::new(&base_url, &connection.realm_id, tokens);

    // 6. Fetch current invoice to get SyncToken
    let current = match client.get_entity("Invoice", &invoice_id).await {
        Ok(c) => c,
        Err(e) => {
            if matches!(
                e,
                QboError::ApiFault { ref code, .. } if code == "610" || code == "6210"
            ) {
                return ApiError::not_found(format!(
                    "Invoice '{}' not found in QuickBooks",
                    invoice_id
                ))
                .into_response();
            }
            return qbo_error(e).into_response();
        }
    };

    let sync_token = match current["Invoice"]["SyncToken"].as_str() {
        Some(t) => t.to_string(),
        None => {
            return ApiError::new(502, "parse_error", "Invoice response missing SyncToken")
                .into_response()
        }
    };

    // 7. Build sparse update body
    let mut body = serde_json::json!({
        "Id": invoice_id,
        "SyncToken": sync_token,
        "sparse": true,
    });

    if let Some(ref ship_date) = req.ship_date {
        body["ShipDate"] = Value::String(ship_date.clone());
    }
    if let Some(ref tracking_num) = req.tracking_num {
        body["TrackingNum"] = Value::String(tracking_num.clone());
    }
    if let Some(ref carrier) = req.carrier {
        body["ShipMethodRef"] = serde_json::json!({ "value": carrier });
    }

    // 8. Call QBO update (handles SyncToken retry internally)
    let result = match client.update_entity("Invoice", body, Uuid::new_v4()).await {
        Ok(r) => r,
        Err(e) => return qbo_error(e).into_response(),
    };

    // 9. Extract confirmed values from response
    let invoice = &result["Invoice"];
    let new_sync_token = invoice["SyncToken"].as_str().unwrap_or("").to_string();

    Json(UpdateInvoiceResponse {
        invoice_id,
        ship_date: invoice["ShipDate"].as_str().map(String::from),
        tracking_num: invoice["TrackingNum"].as_str().map(String::from),
        carrier: invoice["ShipMethodRef"]["value"].as_str().map(String::from),
        sync_token: new_sync_token,
    })
    .into_response()
}
