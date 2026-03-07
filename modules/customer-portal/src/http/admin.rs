use axum::{extract::State, Extension, Json};
use base64::Engine as _;
use chrono::Utc;
use rand::rngs::OsRng;
use rand::RngCore;
use security::VerifiedClaims;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use uuid::Uuid;

use crate::outbox::enqueue_portal_event;

#[derive(Debug, Deserialize)]
pub struct InviteUserRequest {
    pub tenant_id: Uuid,
    pub party_id: Uuid,
    pub email: String,
    pub display_name: String,
    pub scopes: Vec<String>,
    pub idempotency_key: String,
}

#[derive(Debug, Serialize)]
pub struct InviteUserResponse {
    pub user_id: Uuid,
    pub tenant_id: Uuid,
    pub party_id: Uuid,
    pub replay: bool,
}

pub async fn invite_user(
    State(state): State<Arc<crate::AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Json(req): Json<InviteUserRequest>,
) -> Result<Json<InviteUserResponse>, (axum::http::StatusCode, Json<serde_json::Value>)> {
    let Extension(actor) = claims.ok_or_else(crate::auth::unauthorized)?;

    if req.idempotency_key.trim().is_empty() {
        return Err((
            axum::http::StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "idempotency_key_required"})),
        ));
    }

    if actor.tenant_id != req.tenant_id {
        return Err((
            axum::http::StatusCode::FORBIDDEN,
            Json(serde_json::json!({"error": "forbidden"})),
        ));
    }

    let existing = sqlx::query_scalar::<_, serde_json::Value>(
        "SELECT response FROM portal_idempotency WHERE tenant_id=$1 AND operation='invite_user' AND idempotency_key=$2",
    )
    .bind(req.tenant_id)
    .bind(&req.idempotency_key)
    .fetch_optional(&state.pool)
    .await
    .map_err(internal_err)?;

    if let Some(response) = existing {
        let user_id = response
            .get("user_id")
            .and_then(|v| v.as_str())
            .and_then(|s| Uuid::parse_str(s).ok())
            .ok_or_else(|| internal_err_text("invalid idempotency record"))?;
        return Ok(Json(InviteUserResponse {
            user_id,
            tenant_id: req.tenant_id,
            party_id: req.party_id,
            replay: true,
        }));
    }

    let mut pw_bytes = [0u8; 24];
    OsRng.fill_bytes(&mut pw_bytes);
    let temp_password = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(pw_bytes);
    let password_hash = crate::hash_password(&temp_password)
        .map_err(|e| internal_err_text(&format!("password hash: {e}")))?;

    let mut tx = state.pool.begin().await.map_err(internal_err)?;

    let user_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO portal_users (id, tenant_id, party_id, email, password_hash, display_name, invited_by, invited_at) \
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8)",
    )
    .bind(user_id)
    .bind(req.tenant_id)
    .bind(req.party_id)
    .bind(req.email.to_lowercase())
    .bind(password_hash)
    .bind(req.display_name)
    .bind(actor.user_id)
    .bind(Utc::now())
    .execute(&mut *tx)
    .await
    .map_err(internal_err)?;

    sqlx::query(
        "INSERT INTO portal_idempotency (tenant_id, operation, idempotency_key, response) VALUES ($1,'invite_user',$2,$3)",
    )
    .bind(req.tenant_id)
    .bind(&req.idempotency_key)
    .bind(serde_json::json!({"user_id": user_id.to_string()}))
    .execute(&mut *tx)
    .await
    .map_err(internal_err)?;

    enqueue_portal_event(
        &mut tx,
        req.tenant_id,
        Some(actor.user_id),
        platform_contracts::portal_identity::events::USER_INVITED,
        serde_json::json!({
            "tenant_id": req.tenant_id,
            "party_id": req.party_id,
            "user_id": user_id,
            "scopes": req.scopes,
        }),
    )
    .await
    .map_err(internal_err)?;

    tx.commit().await.map_err(internal_err)?;

    Ok(Json(InviteUserResponse {
        user_id,
        tenant_id: req.tenant_id,
        party_id: req.party_id,
        replay: false,
    }))
}

fn internal_err(err: sqlx::Error) -> (axum::http::StatusCode, Json<serde_json::Value>) {
    tracing::error!("portal admin db error: {err}");
    (
        axum::http::StatusCode::INTERNAL_SERVER_ERROR,
        Json(serde_json::json!({"error": "internal_error"})),
    )
}

fn internal_err_text(_msg: &str) -> (axum::http::StatusCode, Json<serde_json::Value>) {
    (
        axum::http::StatusCode::INTERNAL_SERVER_ERROR,
        Json(serde_json::json!({"error": "internal_error"})),
    )
}
