//! HTTP handlers for contact role attributes — 2 endpoints per spec §4.5.

use axum::{
    extract::{Path, State},
    response::IntoResponse,
    Extension, Json,
};
use event_bus::TracingContext;
use platform_http_contracts::ApiError;
use security::VerifiedClaims;
use std::sync::Arc;
use uuid::Uuid;

use crate::domain::contact_role_attributes::{repo, UpsertContactRoleRequest};
use crate::http::tenant::with_request_id;
use crate::AppState;
use platform_sdk::extract_tenant;

#[utoipa::path(
    get, path = "/api/crm-pipeline/contacts/{party_contact_id}/attributes", tag = "ContactRoles",
    params(("party_contact_id" = Uuid, Path, description = "Party Contact ID")),
    responses((status = 200, body = crate::domain::contact_role_attributes::ContactRoleAttributes), (status = 404, body = ApiError)),
    security(("bearer" = [])),
)]
pub async fn get_contact_attributes(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    tracing_ctx: Option<Extension<TracingContext>>,
    Path(party_contact_id): Path<Uuid>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };
    match repo::get_attributes(&state.pool, &tenant_id, party_contact_id).await {
        Ok(Some(attr)) => Json(attr).into_response(),
        Ok(None) => with_request_id(
            ApiError::not_found(format!("No CRM attributes for contact {}", party_contact_id)),
            &tracing_ctx,
        )
        .into_response(),
        Err(e) => with_request_id(ApiError::from(e), &tracing_ctx).into_response(),
    }
}

#[utoipa::path(
    put, path = "/api/crm-pipeline/contacts/{party_contact_id}/attributes", tag = "ContactRoles",
    request_body = UpsertContactRoleRequest,
    responses((status = 200, body = crate::domain::contact_role_attributes::ContactRoleAttributes)),
    security(("bearer" = [])),
)]
pub async fn set_contact_attributes(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    tracing_ctx: Option<Extension<TracingContext>>,
    Path(party_contact_id): Path<Uuid>,
    Json(req): Json<UpsertContactRoleRequest>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };
    let actor = claims.as_ref().map(|c| c.user_id.to_string()).unwrap_or_else(|| "unknown".to_string());
    match repo::upsert_attributes(&state.pool, &tenant_id, party_contact_id, &req, &actor).await {
        Ok(attr) => Json(attr).into_response(),
        Err(e) => with_request_id(ApiError::from(e), &tracing_ctx).into_response(),
    }
}
