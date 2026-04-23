//! HTTP handlers for carrier credential admin API.
//!
//! Routes:
//!   POST /api/integrations/carriers/{carrier_type}/credentials       — upsert credentials (MUTATE)
//!   GET  /api/integrations/carriers/{carrier_type}/credentials/status — check status (READ)

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Extension, Json,
};
use chrono::Datelike;
use platform_http_contracts::ApiError;
use security::VerifiedClaims;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use crate::domain::webhooks::secret_store;
use crate::AppState;
use platform_sdk::extract_tenant;

// ── Request / response types ──────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct SetCarrierCredsRequest {
    // UPS / FedEx
    pub client_id: Option<String>,
    pub client_secret: Option<String>,
    pub account_number: Option<String>,
    // USPS
    pub user_id: Option<String>,
    pub password: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct CredentialsStatusResponse {
    pub configured: bool,
    pub last_set_at: Option<String>,
    pub summary: Option<String>,
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn validate_carrier(carrier_type: &str) -> Result<(), Response> {
    if matches!(carrier_type, "ups" | "fedex" | "usps") {
        Ok(())
    } else {
        Err(ApiError::new(
            400,
            "validation_error",
            format!("Unknown carrier type '{}'. Must be ups, fedex, or usps.", carrier_type),
        )
        .into_response())
    }
}

fn last4(s: &str) -> &str {
    if s.len() >= 4 { &s[s.len() - 4..] } else { s }
}

fn summary_for(carrier_type: &str, creds: &serde_json::Value, date: &str) -> Option<String> {
    match carrier_type {
        "ups" => {
            let acct = creds["account_number"].as_str()?;
            Some(format!("Account ...{}, set {}", last4(acct), date))
        }
        "fedex" => {
            let acct = creds["account_number"].as_str()?;
            Some(format!("Account ...{}, set {}", last4(acct), date))
        }
        "usps" => {
            let uid = creds["user_id"].as_str()?;
            Some(format!("User ID ...{}, set {}", last4(uid), date))
        }
        _ => None,
    }
}

// ── Handlers ──────────────────────────────────────────────────────────────────

/// POST /api/integrations/carriers/{carrier_type}/credentials
pub async fn set_carrier_credentials(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(carrier_type): Path<String>,
    Json(body): Json<SetCarrierCredsRequest>,
) -> Response {
    if let Err(e) = validate_carrier(&carrier_type) {
        return e;
    }

    let app_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };

    let creds = match carrier_type.as_str() {
        "ups" | "fedex" => {
            let client_id = match body.client_id.as_deref().filter(|s| !s.is_empty()) {
                Some(v) => v.to_string(),
                None => {
                    return ApiError::new(400, "validation_error", "client_id is required")
                        .into_response()
                }
            };
            let client_secret = match body.client_secret.as_deref().filter(|s| !s.is_empty()) {
                Some(v) => v.to_string(),
                None => {
                    return ApiError::new(400, "validation_error", "client_secret is required")
                        .into_response()
                }
            };
            let account_number = match body.account_number.as_deref().filter(|s| !s.is_empty()) {
                Some(v) => v.to_string(),
                None => {
                    return ApiError::new(400, "validation_error", "account_number is required")
                        .into_response()
                }
            };
            serde_json::json!({
                "client_id": client_id,
                "client_secret": client_secret,
                "account_number": account_number
            })
        }
        "usps" => {
            let user_id = match body.user_id.as_deref().filter(|s| !s.is_empty()) {
                Some(v) => v.to_string(),
                None => {
                    return ApiError::new(400, "validation_error", "user_id is required")
                        .into_response()
                }
            };
            let mut obj = serde_json::json!({ "user_id": user_id });
            if let Some(pw) = &body.password {
                obj["password"] = serde_json::Value::String(pw.clone());
            }
            obj
        }
        _ => unreachable!("validated above"),
    };

    let creds_str = creds.to_string();
    match secret_store::upsert_carrier_creds(
        &state.pool,
        &app_id,
        &carrier_type,
        &creds_str,
        &state.webhooks_key,
    )
    .await
    {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => {
            tracing::error!(error = %e, carrier_type = %carrier_type, "Failed to upsert carrier credentials");
            ApiError::internal("Internal error").into_response()
        }
    }
}

/// GET /api/integrations/carriers/{carrier_type}/credentials/status
pub async fn carrier_credentials_status(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(carrier_type): Path<String>,
) -> Response {
    if let Err(e) = validate_carrier(&carrier_type) {
        return e;
    }

    let app_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };

    let row: Option<(Vec<u8>, chrono::DateTime<chrono::Utc>)> = match sqlx::query_as(
        "SELECT creds_enc, configured_at FROM integrations_carrier_credentials \
         WHERE app_id = $1 AND carrier_type = $2",
    )
    .bind(&app_id)
    .bind(&carrier_type)
    .fetch_optional(&state.pool)
    .await
    {
        Ok(r) => r,
        Err(e) => {
            tracing::error!(error = %e, "DB error checking carrier credentials status");
            return ApiError::internal("Internal database error").into_response();
        }
    };

    match row {
        None => Json(CredentialsStatusResponse {
            configured: false,
            last_set_at: None,
            summary: None,
        })
        .into_response(),
        Some((enc, configured_at)) => {
            let date_str = format!(
                "{:04}-{:02}-{:02}",
                configured_at.year(),
                configured_at.month(),
                configured_at.day()
            );
            let summary = secret_store::decrypt_token(&state.webhooks_key, &enc)
                .ok()
                .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
                .and_then(|v| summary_for(&carrier_type, &v, &date_str));

            Json(CredentialsStatusResponse {
                configured: true,
                last_set_at: Some(configured_at.to_rfc3339()),
                summary,
            })
            .into_response()
        }
    }
}
