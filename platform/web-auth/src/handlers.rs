use std::sync::Arc;

use axum::extract::{Request, State};
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use uuid::Uuid;

use crate::config::{read_cookie, WebAuthConfig};

pub fn build_router(config: Arc<WebAuthConfig>) -> Router {
    Router::new()
        .route("/login", post(login))
        .route("/logout", post(logout))
        .route("/me", get(me))
        .route("/refresh", post(refresh))
        .with_state(config)
}

// ── Request types ─────────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct LoginBody {
    email: String,
    password: String,
    tenant_id: Uuid,
}

/// JWT payload fields needed to extract tenant_id without signature verification.
#[derive(Deserialize)]
struct TenantClaim {
    tenant_id: String,
}

// ── Login ─────────────────────────────────────────────────────────────────────

async fn login(State(cfg): State<Arc<WebAuthConfig>>, Json(body): Json<LoginBody>) -> Response {
    let url = format!("{}/api/auth/login", cfg.auth_base_url);
    let req_body = serde_json::json!({
        "email": body.email,
        "password": body.password,
        "tenant_id": body.tenant_id,
    });

    let upstream = match cfg.http_client.post(&url).json(&req_body).send().await {
        Ok(r) => r,
        Err(e) => {
            tracing::error!(error = %e, "identity-auth /login unreachable");
            return error_503("auth_unavailable", "Authentication service unavailable");
        }
    };

    let status = upstream.status();
    if !status.is_success() {
        return proxy_error_response(status, upstream).await;
    }

    let tokens: serde_json::Value = match upstream.json().await {
        Ok(v) => v,
        Err(e) => {
            tracing::error!(error = %e, "failed to parse identity-auth /login response");
            return error_503("auth_unavailable", "Invalid response from authentication service");
        }
    };

    let Some(access_token) = tokens["access_token"].as_str() else {
        return error_503("auth_unavailable", "Missing access_token in response");
    };
    let Some(refresh_token) = tokens["refresh_token"].as_str() else {
        return error_503("auth_unavailable", "Missing refresh_token in response");
    };

    let mut response = (StatusCode::OK, Json(serde_json::json!({"ok": true}))).into_response();
    let hdrs = response.headers_mut();
    if let Ok(c) = cfg.build_access_set_cookie(access_token).parse() {
        hdrs.append(header::SET_COOKIE, c);
    }
    if let Ok(c) = cfg.build_refresh_set_cookie(refresh_token).parse() {
        hdrs.append(header::SET_COOKIE, c);
    }
    response
}

// ── Logout ────────────────────────────────────────────────────────────────────

async fn logout(State(cfg): State<Arc<WebAuthConfig>>, req: Request) -> Response {
    if let Some(raw_refresh) = read_cookie(req.headers(), &cfg.refresh_cookie_name()) {
        let url = format!("{}/api/auth/logout", cfg.auth_base_url);
        // Forward the refresh token under the identity-auth cookie name so the
        // server-side session is revoked via the cookie path (refresh_sessions table).
        let cookie_header = format!("refresh={}", raw_refresh);
        if let Err(e) = cfg
            .http_client
            .post(&url)
            .header(header::COOKIE, &cookie_header)
            .header(header::CONTENT_TYPE, "application/json")
            .body("{}")
            .send()
            .await
        {
            tracing::warn!(error = %e, "identity-auth /logout call failed — cookies will still be cleared");
        }
    } else {
        tracing::debug!("logout called without refresh cookie — clearing cookies only");
    }

    let mut response = (StatusCode::OK, Json(serde_json::json!({"ok": true}))).into_response();
    let hdrs = response.headers_mut();
    if let Ok(c) = cfg.build_access_clear_cookie().parse() {
        hdrs.append(header::SET_COOKIE, c);
    }
    if let Ok(c) = cfg.build_refresh_clear_cookie().parse() {
        hdrs.append(header::SET_COOKIE, c);
    }
    response
}

// ── Me ────────────────────────────────────────────────────────────────────────

async fn me(State(cfg): State<Arc<WebAuthConfig>>, req: Request) -> Response {
    let Some(access_token) = read_cookie(req.headers(), &cfg.access_cookie_name()) else {
        return error_401("No session cookie");
    };

    let Some(verifier) = cfg.jwt_verifier.as_ref() else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({"error":"config_error","message":"JWT verification not configured"})),
        )
            .into_response();
    };

    match verifier.verify(&access_token) {
        Ok(claims) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "user_id":    claims.user_id,
                "tenant_id":  claims.tenant_id,
                "app_id":     claims.app_id,
                "roles":      claims.roles,
                "perms":      claims.perms,
                "actor_type": claims.actor_type.as_str(),
                "issued_at":  claims.issued_at,
                "expires_at": claims.expires_at,
            })),
        )
            .into_response(),
        Err(e) => {
            tracing::debug!(error = %e, "access cookie JWT invalid on /me");
            error_401("Invalid or expired session")
        }
    }
}

// ── Refresh ───────────────────────────────────────────────────────────────────

async fn refresh(State(cfg): State<Arc<WebAuthConfig>>, req: Request) -> Response {
    let Some(raw_refresh) = read_cookie(req.headers(), &cfg.refresh_cookie_name()) else {
        return error_401("No refresh cookie");
    };

    // TODO(bd-knkyq): Remove this decode-without-verify once bd-knkyq provides
    // tenant_id from the refresh session server-side without requiring it in the request.
    let tenant_id =
        decode_tenant_from_access_cookie(req.headers(), &cfg.access_cookie_name());

    let url = format!("{}/api/auth/refresh", cfg.auth_base_url);

    // Forward the refresh token as the `refresh` cookie so identity-auth routes to
    // the cookie-aware path (refresh_sessions table with session rotation).
    let cookie_header = format!("refresh={}", raw_refresh);
    let mut req_builder = cfg
        .http_client
        .post(&url)
        .header(header::COOKIE, &cookie_header);

    // Also include body fields as a belt-and-suspenders fallback.
    // The cookie path takes priority server-side; the body is ignored when the
    // cookie is present. tenant_id comes from decode-without-verify (see TODO above).
    if let Some(tid) = tenant_id {
        req_builder = req_builder.json(&serde_json::json!({
            "refresh_token": raw_refresh,
            "tenant_id": tid,
        }));
    }

    let upstream = match req_builder.send().await {
        Ok(r) => r,
        Err(e) => {
            tracing::error!(error = %e, "identity-auth /refresh unreachable");
            return error_503("auth_unavailable", "Authentication service unavailable");
        }
    };

    let status = upstream.status();
    if !status.is_success() {
        return proxy_error_response(status, upstream).await;
    }

    let tokens: serde_json::Value = match upstream.json().await {
        Ok(v) => v,
        Err(e) => {
            tracing::error!(error = %e, "failed to parse identity-auth /refresh response");
            return error_503("auth_unavailable", "Invalid response from authentication service");
        }
    };

    let Some(access_token) = tokens["access_token"].as_str() else {
        return error_503("auth_unavailable", "Missing access_token in response");
    };
    let Some(refresh_token) = tokens["refresh_token"].as_str() else {
        return error_503("auth_unavailable", "Missing refresh_token in response");
    };

    let mut response = (
        StatusCode::OK,
        Json(serde_json::json!({"access_token": access_token, "ok": true})),
    )
        .into_response();
    let hdrs = response.headers_mut();
    if let Ok(c) = cfg.build_access_set_cookie(access_token).parse() {
        hdrs.append(header::SET_COOKIE, c);
    }
    if let Ok(c) = cfg.build_refresh_set_cookie(refresh_token).parse() {
        hdrs.append(header::SET_COOKIE, c);
    }
    response
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Decode the access cookie payload without signature verification to extract tenant_id.
///
/// Used only in the refresh path as an interim bridge — see TODO(bd-knkyq).
/// We never trust the extracted value for auth decisions; it is only used as a
/// routing hint to pass tenant_id to identity-auth's legacy body path.
fn decode_tenant_from_access_cookie(
    headers: &axum::http::HeaderMap,
    access_cookie_name: &str,
) -> Option<Uuid> {
    let raw = read_cookie(headers, access_cookie_name)?;
    let mut validation = jsonwebtoken::Validation::new(jsonwebtoken::Algorithm::RS256);
    validation.insecure_disable_signature_validation();
    validation.validate_exp = false;
    validation.validate_aud = false;
    validation.set_required_spec_claims(&[] as &[&str]);
    let key = jsonwebtoken::DecodingKey::from_secret(b"");
    let data = jsonwebtoken::decode::<TenantClaim>(&raw, &key, &validation).ok()?;
    Uuid::parse_str(&data.claims.tenant_id).ok()
}

fn error_401(message: &str) -> Response {
    (
        StatusCode::UNAUTHORIZED,
        Json(serde_json::json!({"error": "unauthorized", "message": message})),
    )
        .into_response()
}

fn error_503(error: &str, message: &str) -> Response {
    (
        StatusCode::BAD_GATEWAY,
        Json(serde_json::json!({"error": error, "message": message})),
    )
        .into_response()
}

async fn proxy_error_response(status: reqwest::StatusCode, resp: reqwest::Response) -> Response {
    let axum_status =
        StatusCode::from_u16(status.as_u16()).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    let body_text = resp.text().await.unwrap_or_default();
    let json_body: serde_json::Value = serde_json::from_str(&body_text).unwrap_or_else(|_| {
        serde_json::json!({"error": "upstream_error", "message": body_text})
    });
    (axum_status, Json(json_body)).into_response()
}
