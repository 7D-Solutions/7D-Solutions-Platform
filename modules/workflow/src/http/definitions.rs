//! Workflow definition HTTP handlers.

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    Extension, Json,
};
use security::VerifiedClaims;
use serde::Deserialize;
use serde_json::json;
use std::sync::Arc;
use uuid::Uuid;

use crate::domain::definitions::{
    CreateDefinitionRequest, DefError, DefinitionRepo, ListDefinitionsQuery,
};
use crate::routes::ErrorBody;
use crate::AppState;

fn def_error_response(err: DefError) -> (StatusCode, Json<ErrorBody>) {
    match err {
        DefError::NotFound => (
            StatusCode::NOT_FOUND,
            Json(ErrorBody::new("not_found", "Workflow definition not found")),
        ),
        DefError::Validation(msg) => (
            StatusCode::BAD_REQUEST,
            Json(ErrorBody::new("validation_error", &msg)),
        ),
        DefError::Duplicate => (
            StatusCode::CONFLICT,
            Json(ErrorBody::new(
                "duplicate",
                "Definition with this name+version already exists",
            )),
        ),
        DefError::Database(e) => {
            tracing::error!(error = %e, "workflow definition database error");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorBody::new("internal_error", "Database error")),
            )
        }
    }
}

pub async fn create_definition(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Json(mut req): Json<CreateDefinitionRequest>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(t) => t,
        Err(resp) => return resp.into_response(),
    };
    req.tenant_id = tenant_id;
    match DefinitionRepo::create(&state.pool, &req).await {
        Ok(def) => (StatusCode::CREATED, Json(json!(def))).into_response(),
        Err(e) => def_error_response(e).into_response(),
    }
}

#[derive(Debug, Deserialize)]
pub struct ListDefinitionsParams {
    pub active_only: Option<bool>,
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

pub async fn list_definitions(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Query(params): Query<ListDefinitionsParams>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(t) => t,
        Err(resp) => return resp.into_response(),
    };
    let q = ListDefinitionsQuery {
        tenant_id,
        active_only: params.active_only,
        limit: params.limit,
        offset: params.offset,
    };
    match DefinitionRepo::list(&state.pool, &q).await {
        Ok(defs) => Json(json!(defs)).into_response(),
        Err(e) => def_error_response(e).into_response(),
    }
}

pub async fn get_definition(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(def_id): Path<Uuid>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(t) => t,
        Err(resp) => return resp.into_response(),
    };
    match DefinitionRepo::get(&state.pool, &tenant_id, def_id).await {
        Ok(def) => Json(json!(def)).into_response(),
        Err(e) => def_error_response(e).into_response(),
    }
}

fn extract_tenant(
    claims: &Option<Extension<VerifiedClaims>>,
) -> Result<String, (StatusCode, Json<ErrorBody>)> {
    match claims {
        Some(Extension(vc)) => Ok(vc.tenant_id.to_string()),
        None => {
            // Dev mode: fall back to header or default
            Ok("dev-tenant".to_string())
        }
    }
}
