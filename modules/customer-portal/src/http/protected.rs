use axum::{extract::State, Json};
use serde::Serialize;
use std::sync::Arc;
use uuid::Uuid;

use crate::auth::PortalClaims;

#[derive(Debug, Serialize)]
pub struct MeResponse {
    pub user_id: String,
    pub tenant_id: String,
    pub party_id: String,
    pub scopes: Vec<String>,
}

pub async fn me(
    State(_state): State<Arc<crate::AppState>>,
    PortalClaims(claims): PortalClaims,
) -> Json<MeResponse> {
    Json(MeResponse {
        user_id: claims.sub,
        tenant_id: claims.tenant_id,
        party_id: claims.party_id,
        scopes: claims.scopes,
    })
}

pub async fn party_guard_probe(
    State(_state): State<Arc<crate::AppState>>,
    axum::extract::Path(party_id): axum::extract::Path<Uuid>,
    PortalClaims(claims): PortalClaims,
) -> Result<Json<serde_json::Value>, (axum::http::StatusCode, Json<serde_json::Value>)> {
    if claims.party_id != party_id.to_string() {
        return Err((
            axum::http::StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "not_found"})),
        ));
    }

    Ok(Json(serde_json::json!({
        "party_id": party_id,
        "tenant_id": claims.tenant_id,
        "status": "ok"
    })))
}
