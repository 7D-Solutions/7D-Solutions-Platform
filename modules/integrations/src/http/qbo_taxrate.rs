//! HTTP handler for QBO TaxRate list proxy.
//!
//! Route: GET /api/integrations/qbo/taxrate?realm_id=...&active=true
//!
//! Proxies the TaxRate list from QBO for the connected realm. Callers
//! use this to resolve TaxRate IDs before populating txn_tax_detail
//! on outbound invoices.
//!
//! Security: realm_id is validated against the stored oauth_connection —
//! a mismatched realm_id returns 403 to prevent cross-tenant data leaks.

use axum::{
    extract::{Query, State},
    response::IntoResponse,
    Extension, Json,
};
use platform_http_contracts::ApiError;
use security::VerifiedClaims;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use utoipa::ToSchema;

use crate::domain::oauth::{service as oauth_service, OAuthError};
use crate::domain::qbo::{client::QboClient, QboError, TokenProvider};
use crate::AppState;
use platform_sdk::extract_tenant;

// ============================================================================
// Request / Response
// ============================================================================

#[derive(Debug, Deserialize)]
pub struct ListTaxratesQuery {
    /// QBO realm (company) ID. Must match the connected realm — 403 if not.
    pub realm_id: String,
    /// When true, only Active=true rates are returned.
    pub active: Option<bool>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct TaxrateListResponse {
    pub taxrates: Vec<TaxRateItem>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct TaxRateItem {
    /// QBO TaxRate.Id — reference value for txn_tax_detail lines.
    pub id: String,
    pub name: String,
    pub active: bool,
    /// Rate as percentage (e.g. 8.5 = 8.5%).
    pub rate_value: Option<f64>,
    /// Tax agency name from QBO AgencyRef.
    pub agency_ref: Option<String>,
}

// ============================================================================
// Error mapping
// ============================================================================

fn qbo_error(e: QboError) -> ApiError {
    match e {
        QboError::RateLimited { .. } => {
            ApiError::new(429, "rate_limited", "QBO rate limit exceeded")
        }
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
        _ => ApiError::internal("Internal error"),
    }
}

fn oauth_err(e: OAuthError) -> ApiError {
    match e {
        OAuthError::NotFound => ApiError::not_found("No QuickBooks connection found for this tenant"),
        OAuthError::MissingEncryptionKey => {
            tracing::error!(error_code = "OPERATION_FAILED", "OAUTH_ENCRYPTION_KEY not set");
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
// DB-backed TokenProvider
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
        Err(QboError::AuthFailed)
    }
}

// ============================================================================
// Handler
// ============================================================================

#[utoipa::path(
    get,
    path = "/api/integrations/qbo/taxrate",
    params(
        ("realm_id" = String, Query, description = "QBO realm (company) ID"),
        ("active" = Option<bool>, Query, description = "Return only active rates when true"),
    ),
    responses(
        (status = 200, description = "TaxRate list", body = TaxrateListResponse),
        (status = 403, description = "realm_id does not match connected QBO realm"),
        (status = 412, description = "No active QBO connection for this tenant"),
        (status = 429, description = "QBO rate limit exceeded"),
        (status = 502, description = "QBO API error"),
    ),
    security(("bearer" = [])),
    tag = "QBO TaxRate"
)]
pub async fn list_taxrates(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Query(params): Query<ListTaxratesQuery>,
) -> impl IntoResponse {
    // 1. Extract tenant
    let app_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };

    // 2. Look up QBO connection
    let connection =
        match oauth_service::get_connection_status(&state.pool, &app_id, "quickbooks").await {
            Ok(Some(c)) => c,
            Ok(None) => {
                return ApiError::new(
                    412,
                    "not_connected",
                    "No QuickBooks connection found for this tenant",
                )
                .into_response()
            }
            Err(e) => return oauth_err(e).into_response(),
        };

    // 3. Validate connection is active
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

    // 4. Realm-mismatch check — load-bearing security gate.
    //    Without this, Tenant A with Realm X could GET ?realm_id=Y and see Tenant B's data.
    if connection.realm_id != params.realm_id {
        return ApiError::new(
            403,
            "realm_mismatch",
            "realm_id does not match the connected QBO realm for this tenant",
        )
        .into_response();
    }

    // 5. Build QBO client with DB-backed token provider
    let base_url = crate::domain::qbo::cdc::qbo_base_url();
    let tokens: Arc<dyn TokenProvider> = Arc::new(DbTokenProvider {
        pool: state.pool.clone(),
        app_id: app_id.clone(),
    });
    let client = QboClient::new(&base_url, &connection.realm_id, tokens);

    // 6. List taxrates
    let active_only = params.active.unwrap_or(false);
    let qbo_rates = match client.list_taxrates(active_only).await {
        Ok(r) => r,
        Err(e) => return qbo_error(e).into_response(),
    };

    let taxrates = qbo_rates
        .into_iter()
        .map(|r| TaxRateItem {
            id: r.id,
            name: r.name,
            active: r.active,
            rate_value: r.rate_value,
            agency_ref: r.agency_ref,
        })
        .collect();

    Json(TaxrateListResponse { taxrates }).into_response()
}
