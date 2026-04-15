//! Workflow instance HTTP handlers.

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    Extension, Json,
};
use event_bus::TracingContext;
use platform_http_contracts::{ApiError, PaginatedResponse};
use security::VerifiedClaims;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use uuid::Uuid;

use crate::domain::instances::{
    AdvanceInstanceRequest, InstanceRepo, ListInstancesQuery, StartInstanceRequest,
    WorkflowInstance, WorkflowTransition,
};
use crate::AppState;

use super::tenant::with_request_id;
use platform_sdk::extract_tenant;

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct AdvanceResponse {
    pub instance: WorkflowInstance,
    pub transition: WorkflowTransition,
}

#[utoipa::path(
    post, path = "/api/workflow/instances", tag = "Instances",
    request_body = StartInstanceRequest,
    responses(
        (status = 201, description = "Instance started", body = WorkflowInstance),
        (status = 404, description = "Definition not found", body = ApiError),
        (status = 422, description = "Validation error", body = ApiError),
    ),
    security(("bearer" = ["WORKFLOW_MUTATE"])),
)]
pub async fn start_instance(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    ctx: Option<Extension<TracingContext>>,
    Json(mut req): Json<StartInstanceRequest>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(t) => t,
        Err(e) => return with_request_id(e, &ctx).into_response(),
    };
    req.tenant_id = tenant_id;
    match InstanceRepo::start(&state.pool, &req).await {
        Ok(inst) => (StatusCode::CREATED, Json(inst)).into_response(),
        Err(e) => with_request_id(ApiError::from(e), &ctx).into_response(),
    }
}

#[utoipa::path(
    patch, path = "/api/workflow/instances/{instance_id}/advance", tag = "Instances",
    params(("instance_id" = Uuid, Path, description = "Instance ID")),
    request_body = AdvanceInstanceRequest,
    responses(
        (status = 200, description = "Instance advanced", body = AdvanceResponse),
        (status = 404, description = "Not found", body = ApiError),
        (status = 422, description = "Invalid transition", body = ApiError),
    ),
    security(("bearer" = ["WORKFLOW_MUTATE"])),
)]
pub async fn advance_instance(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    ctx: Option<Extension<TracingContext>>,
    Path(instance_id): Path<Uuid>,
    Json(mut req): Json<AdvanceInstanceRequest>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(t) => t,
        Err(e) => return with_request_id(e, &ctx).into_response(),
    };
    req.tenant_id = tenant_id;
    match InstanceRepo::advance(&state.pool, instance_id, &req).await {
        Ok((inst, transition)) => Json(AdvanceResponse {
            instance: inst,
            transition,
        })
        .into_response(),
        Err(e) => with_request_id(ApiError::from(e), &ctx).into_response(),
    }
}

#[utoipa::path(
    get, path = "/api/workflow/instances/{instance_id}", tag = "Instances",
    params(("instance_id" = Uuid, Path, description = "Instance ID")),
    responses(
        (status = 200, description = "Instance found", body = WorkflowInstance),
        (status = 404, description = "Not found", body = ApiError),
    ),
    security(("bearer" = ["WORKFLOW_MUTATE"])),
)]
pub async fn get_instance(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    ctx: Option<Extension<TracingContext>>,
    Path(instance_id): Path<Uuid>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(t) => t,
        Err(e) => return with_request_id(e, &ctx).into_response(),
    };
    match InstanceRepo::get(&state.pool, &tenant_id, instance_id).await {
        Ok(inst) => Json(inst).into_response(),
        Err(e) => with_request_id(ApiError::from(e), &ctx).into_response(),
    }
}

#[derive(Debug, Deserialize, utoipa::IntoParams)]
#[into_params(parameter_in = Query)]
pub struct ListInstancesParams {
    pub entity_type: Option<String>,
    pub entity_id: Option<String>,
    pub status: Option<String>,
    pub definition_id: Option<Uuid>,
    pub page: Option<i64>,
    pub page_size: Option<i64>,
}

#[utoipa::path(
    get, path = "/api/workflow/instances", tag = "Instances",
    params(ListInstancesParams),
    responses(
        (status = 200, description = "Paginated instances", body = PaginatedResponse<WorkflowInstance>),
    ),
    security(("bearer" = ["WORKFLOW_MUTATE"])),
)]
pub async fn list_instances(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    ctx: Option<Extension<TracingContext>>,
    Query(params): Query<ListInstancesParams>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(t) => t,
        Err(e) => return with_request_id(e, &ctx).into_response(),
    };
    let page = params.page.unwrap_or(1).max(1);
    let page_size = params.page_size.unwrap_or(50).clamp(1, 200);
    let q = ListInstancesQuery {
        tenant_id,
        entity_type: params.entity_type,
        entity_id: params.entity_id,
        status: params.status,
        definition_id: params.definition_id,
        limit: Some(page_size),
        offset: Some((page - 1) * page_size),
    };
    let total = match InstanceRepo::count(&state.pool, &q).await {
        Ok(t) => t,
        Err(e) => return with_request_id(ApiError::from(e), &ctx).into_response(),
    };
    match InstanceRepo::list(&state.pool, &q).await {
        Ok(instances) => {
            Json(PaginatedResponse::new(instances, page, page_size, total)).into_response()
        }
        Err(e) => with_request_id(ApiError::from(e), &ctx).into_response(),
    }
}

#[utoipa::path(
    get, path = "/api/workflow/instances/{instance_id}/transitions", tag = "Instances",
    params(("instance_id" = Uuid, Path, description = "Instance ID")),
    responses(
        (status = 200, description = "Transitions for instance", body = Vec<WorkflowTransition>),
        (status = 404, description = "Not found", body = ApiError),
    ),
    security(("bearer" = ["WORKFLOW_MUTATE"])),
)]
pub async fn list_transitions(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    ctx: Option<Extension<TracingContext>>,
    Path(instance_id): Path<Uuid>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(t) => t,
        Err(e) => return with_request_id(e, &ctx).into_response(),
    };
    match InstanceRepo::list_transitions(&state.pool, &tenant_id, instance_id).await {
        Ok(transitions) => Json(transitions).into_response(),
        Err(e) => with_request_id(ApiError::from(e), &ctx).into_response(),
    }
}
