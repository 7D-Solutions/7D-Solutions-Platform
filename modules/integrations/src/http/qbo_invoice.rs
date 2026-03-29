//! HTTP handler for QBO invoice sparse updates.
//!
//! Route: POST /api/integrations/qbo/invoice/{invoice_id}/update
//!
//! Allows callers to update shipping fields on a QBO invoice without
//! touching other fields. Uses the platform-owned OAuth connection.

use axum::{
    extract::{Path, State},
    http::StatusCode,
    Extension, Json,
};
use security::VerifiedClaims;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::Arc;

use crate::domain::oauth::{service as oauth_service, OAuthError};
use crate::domain::qbo::{client::QboClient, QboError, TokenProvider};
use crate::AppState;

// ============================================================================
// Request / Response
// ============================================================================

#[derive(Debug, Deserialize)]
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

#[derive(Debug, Serialize)]
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

#[derive(Debug, Serialize)]
pub struct ErrorBody {
    pub error: String,
    pub message: String,
}

impl ErrorBody {
    fn new(error: &str, message: &str) -> Self {
        Self {
            error: error.to_string(),
            message: message.to_string(),
        }
    }
}

fn qbo_error_response(e: QboError) -> (StatusCode, Json<ErrorBody>) {
    match e {
        QboError::SyncTokenExhausted(_) => (
            StatusCode::CONFLICT,
            Json(ErrorBody::new(
                "sync_token_conflict",
                "Invoice was modified concurrently — retry the request",
            )),
        ),
        QboError::RateLimited => (
            StatusCode::TOO_MANY_REQUESTS,
            Json(ErrorBody::new("rate_limited", "QBO rate limit exceeded")),
        ),
        QboError::AuthFailed => (
            StatusCode::BAD_GATEWAY,
            Json(ErrorBody::new(
                "auth_failed",
                "QBO authentication failed — connection may need reauthorization",
            )),
        ),
        QboError::TokenError(msg) => (
            StatusCode::BAD_GATEWAY,
            Json(ErrorBody::new("token_error", &msg)),
        ),
        QboError::ApiFault {
            fault_type,
            message,
            code,
            ..
        } => {
            let status = if fault_type.to_lowercase().contains("validation") {
                StatusCode::UNPROCESSABLE_ENTITY
            } else {
                StatusCode::BAD_GATEWAY
            };
            (
                status,
                Json(ErrorBody::new(
                    "qbo_fault",
                    &format!("{}: {} (code {})", fault_type, message, code),
                )),
            )
        }
        QboError::Http(e) => (
            StatusCode::BAD_GATEWAY,
            Json(ErrorBody::new("network_error", &e.to_string())),
        ),
        QboError::Deserialize(msg) => (
            StatusCode::BAD_GATEWAY,
            Json(ErrorBody::new("parse_error", &msg)),
        ),
    }
}

fn oauth_error_response(e: OAuthError) -> (StatusCode, Json<ErrorBody>) {
    match e {
        OAuthError::NotFound => (
            StatusCode::NOT_FOUND,
            Json(ErrorBody::new(
                "no_connection",
                "No QuickBooks connection found for this tenant",
            )),
        ),
        OAuthError::MissingEncryptionKey => {
            tracing::error!("OAUTH_ENCRYPTION_KEY not set");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorBody::new(
                    "configuration_error",
                    "Server misconfiguration",
                )),
            )
        }
        OAuthError::Database(e) => {
            tracing::error!("OAuth DB error: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorBody::new("database_error", "Internal database error")),
            )
        }
        _ => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorBody::new("internal_error", "Internal error")),
        ),
    }
}

fn extract_tenant(
    claims: &Option<Extension<VerifiedClaims>>,
) -> Result<String, (StatusCode, Json<ErrorBody>)> {
    match claims {
        Some(Extension(c)) => Ok(c.tenant_id.to_string()),
        None => Err((
            StatusCode::UNAUTHORIZED,
            Json(ErrorBody::new(
                "unauthorized",
                "Missing or invalid authentication",
            )),
        )),
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

/// POST /api/integrations/qbo/invoice/{invoice_id}/update
///
/// Sparse-update a QBO invoice with shipping fields.
///
/// Requires `integrations.mutate` permission.
pub async fn update_invoice(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(invoice_id): Path<String>,
    Json(req): Json<UpdateInvoiceRequest>,
) -> Result<Json<UpdateInvoiceResponse>, (StatusCode, Json<ErrorBody>)> {
    // 1. Validate request has at least one field
    if !req.has_fields() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorBody::new(
                "no_fields",
                "At least one of ship_date, tracking_num, or carrier is required",
            )),
        ));
    }

    // 2. Extract tenant from JWT
    let app_id = extract_tenant(&claims)?;

    // 3. Look up QBO connection
    let connection = oauth_service::get_connection_status(&state.pool, &app_id, "quickbooks")
        .await
        .map_err(oauth_error_response)?
        .ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                Json(ErrorBody::new(
                    "no_connection",
                    "No QuickBooks connection found for this tenant",
                )),
            )
        })?;

    // 4. Validate connection is active
    if connection.connection_status != "connected" {
        return Err((
            StatusCode::PRECONDITION_FAILED,
            Json(ErrorBody::new(
                "not_connected",
                &format!(
                    "QuickBooks connection is '{}' — reconnection required",
                    connection.connection_status
                ),
            )),
        ));
    }

    // 5. Create QBO client with DB-backed token provider
    let base_url = std::env::var("QBO_API_BASE")
        .unwrap_or_else(|_| "https://quickbooks.api.intuit.com/v3".to_string());

    let tokens: Arc<dyn TokenProvider> = Arc::new(DbTokenProvider {
        pool: state.pool.clone(),
        app_id: app_id.clone(),
    });

    let client = QboClient::new(&base_url, &connection.realm_id, tokens);

    // 6. Fetch current invoice to get SyncToken
    let current = client
        .get_entity("Invoice", &invoice_id)
        .await
        .map_err(|e| {
            if matches!(
                e,
                QboError::ApiFault { ref code, .. } if code == "610" || code == "6210"
            ) {
                (
                    StatusCode::NOT_FOUND,
                    Json(ErrorBody::new(
                        "invoice_not_found",
                        &format!("Invoice '{}' not found in QuickBooks", invoice_id),
                    )),
                )
            } else {
                qbo_error_response(e)
            }
        })?;

    let sync_token = current["Invoice"]["SyncToken"]
        .as_str()
        .ok_or_else(|| {
            (
                StatusCode::BAD_GATEWAY,
                Json(ErrorBody::new(
                    "parse_error",
                    "Invoice response missing SyncToken",
                )),
            )
        })?
        .to_string();

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
    let result = client
        .update_entity("Invoice", body)
        .await
        .map_err(qbo_error_response)?;

    // 9. Extract confirmed values from response
    let invoice = &result["Invoice"];
    let new_sync_token = invoice["SyncToken"]
        .as_str()
        .unwrap_or("")
        .to_string();

    Ok(Json(UpdateInvoiceResponse {
        invoice_id,
        ship_date: invoice["ShipDate"].as_str().map(String::from),
        tracking_num: invoice["TrackingNum"].as_str().map(String::from),
        carrier: invoice["ShipMethodRef"]["value"].as_str().map(String::from),
        sync_token: new_sync_token,
    }))
}
