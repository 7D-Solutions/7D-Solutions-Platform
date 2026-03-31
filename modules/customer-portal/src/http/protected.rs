use axum::{extract::State, Extension, Json};
use event_bus::TracingContext;
use platform_http_contracts::ApiError;
use serde::Serialize;
use std::sync::Arc;
use utoipa::ToSchema;
use uuid::Uuid;

use super::tenant::with_request_id;
use crate::auth::PortalClaims;

#[derive(Debug, Serialize, ToSchema)]
pub struct MeResponse {
    pub user_id: String,
    pub tenant_id: String,
    pub party_id: String,
    pub scopes: Vec<String>,
}

#[utoipa::path(
    get, path = "/portal/me", tag = "Portal",
    responses(
        (status = 200, description = "Current user info", body = MeResponse),
        (status = 401, body = ApiError),
    ),
    security(("bearer" = [])),
)]
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

#[utoipa::path(
    get, path = "/portal/party/{party_id}/probe", tag = "Portal",
    params(("party_id" = Uuid, Path, description = "Party ID to probe")),
    responses(
        (status = 200, description = "Party guard check passed"),
        (status = 401, body = ApiError), (status = 404, body = ApiError),
    ),
    security(("bearer" = [])),
)]
pub async fn party_guard_probe(
    State(_state): State<Arc<crate::AppState>>,
    axum::extract::Path(party_id): axum::extract::Path<Uuid>,
    PortalClaims(claims): PortalClaims,
    ctx: Option<Extension<TracingContext>>,
) -> Result<Json<serde_json::Value>, ApiError> {
    if claims.party_id != party_id.to_string() {
        return Err(with_request_id(ApiError::not_found("not_found"), &ctx));
    }

    Ok(Json(serde_json::json!({
        "party_id": party_id,
        "tenant_id": claims.tenant_id,
        "status": "ok"
    })))
}
