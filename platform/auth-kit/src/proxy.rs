//! Proxy handlers that forward auth requests to identity-auth.
//!
//! Verticals mount these on their own router so end users hit the vertical's
//! domain, not identity-auth directly.

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use uuid::Uuid;

use crate::AuthKitState;

/// Login request — forwarded to identity-auth `/api/auth/login`.
#[derive(Debug, Deserialize, Serialize)]
pub struct LoginRequest {
    pub tenant_id: Uuid,
    pub email: String,
    pub password: String,
}

/// Refresh request — forwarded to identity-auth `/api/auth/refresh`.
#[derive(Debug, Deserialize, Serialize)]
pub struct RefreshRequest {
    pub tenant_id: Uuid,
    pub refresh_token: String,
}

/// Logout request — forwarded to identity-auth `/api/auth/logout`.
#[derive(Debug, Deserialize, Serialize)]
pub struct LogoutRequest {
    pub tenant_id: Uuid,
    pub refresh_token: String,
}

/// Token response returned by identity-auth.
#[derive(Debug, Deserialize, Serialize)]
pub struct TokenResponse {
    pub token_type: String,
    pub access_token: String,
    pub expires_in_seconds: i64,
    pub refresh_token: String,
}

/// Simple OK response.
#[derive(Debug, Deserialize, Serialize)]
pub struct OkResponse {
    pub ok: bool,
}

/// Error response from identity-auth.
#[derive(Debug, Deserialize, Serialize)]
pub struct ErrorBody {
    #[serde(default)]
    pub error: String,
    #[serde(default)]
    pub message: String,
}

/// Proxy POST /auth/login → identity-auth /api/auth/login.
pub(crate) async fn login(
    State(state): State<Arc<AuthKitState>>,
    Json(body): Json<LoginRequest>,
) -> impl IntoResponse {
    forward_post(&state, "/api/auth/login", &body).await
}

/// Proxy POST /auth/refresh → identity-auth /api/auth/refresh.
pub(crate) async fn refresh(
    State(state): State<Arc<AuthKitState>>,
    Json(body): Json<RefreshRequest>,
) -> impl IntoResponse {
    forward_post(&state, "/api/auth/refresh", &body).await
}

/// Proxy POST /auth/logout → identity-auth /api/auth/logout.
pub(crate) async fn logout(
    State(state): State<Arc<AuthKitState>>,
    Json(body): Json<LogoutRequest>,
) -> impl IntoResponse {
    forward_post(&state, "/api/auth/logout", &body).await
}

/// Forward a JSON POST to identity-auth, returning the raw status + body.
async fn forward_post<T: Serialize>(
    state: &AuthKitState,
    path: &str,
    body: &T,
) -> impl IntoResponse {
    let url = format!("{}{}", state.identity_url, path);
    let result = state.http.post(&url).json(body).send().await;

    match result {
        Ok(resp) => {
            let status = StatusCode::from_u16(resp.status().as_u16())
                .unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
            match resp.bytes().await {
                Ok(bytes) => (
                    status,
                    [(
                        axum::http::header::CONTENT_TYPE,
                        "application/json".to_string(),
                    )],
                    bytes,
                )
                    .into_response(),
                Err(e) => {
                    tracing::error!(path, error = %e, "failed to read identity-auth response body");
                    StatusCode::BAD_GATEWAY.into_response()
                }
            }
        }
        Err(e) => {
            tracing::error!(path, error = %e, "identity-auth request failed");
            StatusCode::BAD_GATEWAY.into_response()
        }
    }
}
