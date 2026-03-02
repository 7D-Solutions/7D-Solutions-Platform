//! Workflow instance HTTP handlers.

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

use crate::domain::instances::{
    AdvanceInstanceRequest, InstanceError, InstanceRepo, ListInstancesQuery, StartInstanceRequest,
};
use crate::routes::ErrorBody;
use crate::AppState;

fn instance_error_response(err: InstanceError) -> (StatusCode, Json<ErrorBody>) {
    match err {
        InstanceError::NotFound => (
            StatusCode::NOT_FOUND,
            Json(ErrorBody::new("not_found", "Workflow instance not found")),
        ),
        InstanceError::DefinitionNotFound => (
            StatusCode::NOT_FOUND,
            Json(ErrorBody::new("not_found", "Workflow definition not found")),
        ),
        InstanceError::Validation(msg) => (
            StatusCode::BAD_REQUEST,
            Json(ErrorBody::new("validation_error", &msg)),
        ),
        InstanceError::InvalidTransition(msg) => (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(ErrorBody::new("invalid_transition", &msg)),
        ),
        InstanceError::NotActive(status) => (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(ErrorBody::new(
                "not_active",
                &format!("Instance is not active (status: {})", status),
            )),
        ),
        InstanceError::Database(e) => {
            tracing::error!(error = %e, "workflow instance database error");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorBody::new("internal_error", "Database error")),
            )
        }
    }
}

pub async fn start_instance(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Json(mut req): Json<StartInstanceRequest>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(t) => t,
        Err(resp) => return resp.into_response(),
    };
    req.tenant_id = tenant_id;
    match InstanceRepo::start(&state.pool, &req).await {
        Ok(inst) => (StatusCode::CREATED, Json(json!(inst))).into_response(),
        Err(e) => instance_error_response(e).into_response(),
    }
}

pub async fn advance_instance(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(instance_id): Path<Uuid>,
    Json(mut req): Json<AdvanceInstanceRequest>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(t) => t,
        Err(resp) => return resp.into_response(),
    };
    req.tenant_id = tenant_id;
    match InstanceRepo::advance(&state.pool, instance_id, &req).await {
        Ok((inst, transition)) => Json(json!({
            "instance": inst,
            "transition": transition
        }))
        .into_response(),
        Err(e) => instance_error_response(e).into_response(),
    }
}

pub async fn get_instance(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(instance_id): Path<Uuid>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(t) => t,
        Err(resp) => return resp.into_response(),
    };
    match InstanceRepo::get(&state.pool, &tenant_id, instance_id).await {
        Ok(inst) => Json(json!(inst)).into_response(),
        Err(e) => instance_error_response(e).into_response(),
    }
}

#[derive(Debug, Deserialize)]
pub struct ListInstancesParams {
    pub entity_type: Option<String>,
    pub entity_id: Option<String>,
    pub status: Option<String>,
    pub definition_id: Option<Uuid>,
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

pub async fn list_instances(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Query(params): Query<ListInstancesParams>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(t) => t,
        Err(resp) => return resp.into_response(),
    };
    let q = ListInstancesQuery {
        tenant_id,
        entity_type: params.entity_type,
        entity_id: params.entity_id,
        status: params.status,
        definition_id: params.definition_id,
        limit: params.limit,
        offset: params.offset,
    };
    match InstanceRepo::list(&state.pool, &q).await {
        Ok(instances) => Json(json!(instances)).into_response(),
        Err(e) => instance_error_response(e).into_response(),
    }
}

pub async fn list_transitions(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(instance_id): Path<Uuid>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(t) => t,
        Err(resp) => return resp.into_response(),
    };
    match InstanceRepo::list_transitions(&state.pool, &tenant_id, instance_id).await {
        Ok(transitions) => Json(json!(transitions)).into_response(),
        Err(e) => instance_error_response(e).into_response(),
    }
}

fn extract_tenant(
    claims: &Option<Extension<VerifiedClaims>>,
) -> Result<String, (StatusCode, Json<ErrorBody>)> {
    match claims {
        Some(Extension(vc)) => Ok(vc.tenant_id.to_string()),
        None => Ok("dev-tenant".to_string()),
    }
}
