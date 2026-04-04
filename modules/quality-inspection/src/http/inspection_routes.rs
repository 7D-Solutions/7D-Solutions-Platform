use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    Extension, Json,
};
use event_bus::TracingContext;
use platform_http_contracts::{ApiError, PaginatedResponse};
use security::VerifiedClaims;
use serde::Deserialize;
use std::sync::Arc;
use utoipa::IntoParams;
use uuid::Uuid;

use platform_sdk::extract_tenant;
use super::tenant::with_request_id;
use crate::domain::models::*;
use crate::domain::service;
use crate::AppState;

fn correlation_id() -> String {
    Uuid::new_v4().to_string()
}

// ============================================================================
// Inspection Plans
// ============================================================================

#[utoipa::path(
    post,
    path = "/api/quality-inspection/plans",
    tag = "Inspection Plans",
    request_body = CreateInspectionPlanRequest,
    responses(
        (status = 201, description = "Plan created", body = InspectionPlan),
        (status = 401, description = "Unauthorized"),
        (status = 422, description = "Validation error", body = ApiError),
        (status = 500, description = "Internal error", body = ApiError),
    ),
    security(("bearer" = ["QUALITY_INSPECTION_MUTATE"]))
)]
pub async fn post_inspection_plan(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    tracing_ctx: Option<Extension<TracingContext>>,
    Json(req): Json<CreateInspectionPlanRequest>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };
    match service::create_inspection_plan(
        &state.pool,
        &tenant_id,
        &req,
        &correlation_id(),
        None,
    )
    .await
    {
        Ok(plan) => (StatusCode::CREATED, Json(plan)).into_response(),
        Err(e) => with_request_id(ApiError::from(e), &tracing_ctx).into_response(),
    }
}

#[utoipa::path(
    get,
    path = "/api/quality-inspection/plans/{plan_id}",
    tag = "Inspection Plans",
    params(("plan_id" = Uuid, Path, description = "Plan ID")),
    responses(
        (status = 200, description = "Plan details", body = InspectionPlan),
        (status = 401, description = "Unauthorized"),
        (status = 404, description = "Not found", body = ApiError),
        (status = 500, description = "Internal error", body = ApiError),
    ),
    security(("bearer" = ["QUALITY_INSPECTION_READ"]))
)]
pub async fn get_inspection_plan(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    tracing_ctx: Option<Extension<TracingContext>>,
    Path(plan_id): Path<Uuid>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };
    match service::get_inspection_plan(&state.pool, &tenant_id, plan_id).await {
        Ok(plan) => Json(plan).into_response(),
        Err(e) => with_request_id(ApiError::from(e), &tracing_ctx).into_response(),
    }
}

#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct ListPlansPaginatedQuery {
    pub page: Option<i64>,
    pub page_size: Option<i64>,
}

#[utoipa::path(
    get,
    path = "/api/quality-inspection/plans",
    tag = "Inspection Plans",
    params(ListPlansPaginatedQuery),
    responses(
        (status = 200, description = "Paginated inspection plans", body = PaginatedResponse<InspectionPlan>),
        (status = 401, description = "Unauthorized"),
        (status = 500, description = "Internal error", body = ApiError),
    ),
    security(("bearer" = ["QUALITY_INSPECTION_READ"]))
)]
pub async fn get_inspection_plans(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    tracing_ctx: Option<Extension<TracingContext>>,
    Query(q): Query<ListPlansPaginatedQuery>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };
    let page = q.page.unwrap_or(1).max(1);
    let page_size = q.page_size.unwrap_or(50).clamp(1, 200);

    match service::list_inspection_plans(&state.pool, &tenant_id, page, page_size).await {
        Ok((rows, total)) => {
            Json(PaginatedResponse::new(rows, page, page_size, total)).into_response()
        }
        Err(e) => with_request_id(ApiError::from(e), &tracing_ctx).into_response(),
    }
}

#[utoipa::path(
    post,
    path = "/api/quality-inspection/plans/{plan_id}/activate",
    tag = "Inspection Plans",
    params(("plan_id" = Uuid, Path, description = "Plan ID")),
    responses(
        (status = 200, description = "Plan activated", body = InspectionPlan),
        (status = 401, description = "Unauthorized"),
        (status = 404, description = "Not found", body = ApiError),
        (status = 422, description = "Validation error", body = ApiError),
        (status = 500, description = "Internal error", body = ApiError),
    ),
    security(("bearer" = ["QUALITY_INSPECTION_MUTATE"]))
)]
pub async fn post_activate_plan(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    tracing_ctx: Option<Extension<TracingContext>>,
    Path(plan_id): Path<Uuid>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };
    match service::activate_plan(&state.pool, &tenant_id, plan_id).await {
        Ok(plan) => Json(plan).into_response(),
        Err(e) => with_request_id(ApiError::from(e), &tracing_ctx).into_response(),
    }
}

// ============================================================================
// Receiving Inspections
// ============================================================================

#[utoipa::path(
    post,
    path = "/api/quality-inspection/inspections",
    tag = "Inspections",
    request_body = CreateReceivingInspectionRequest,
    responses(
        (status = 201, description = "Receiving inspection created", body = Inspection),
        (status = 401, description = "Unauthorized"),
        (status = 422, description = "Validation error", body = ApiError),
        (status = 500, description = "Internal error", body = ApiError),
    ),
    security(("bearer" = ["QUALITY_INSPECTION_MUTATE"]))
)]
pub async fn post_receiving_inspection(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    tracing_ctx: Option<Extension<TracingContext>>,
    Json(req): Json<CreateReceivingInspectionRequest>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };
    match service::create_receiving_inspection(
        &state.pool,
        &tenant_id,
        &req,
        &correlation_id(),
        None,
    )
    .await
    {
        Ok(inspection) => (StatusCode::CREATED, Json(inspection)).into_response(),
        Err(e) => with_request_id(ApiError::from(e), &tracing_ctx).into_response(),
    }
}

#[utoipa::path(
    get,
    path = "/api/quality-inspection/inspections/{inspection_id}",
    tag = "Inspections",
    params(("inspection_id" = Uuid, Path, description = "Inspection ID")),
    responses(
        (status = 200, description = "Inspection details", body = Inspection),
        (status = 401, description = "Unauthorized"),
        (status = 404, description = "Not found", body = ApiError),
        (status = 500, description = "Internal error", body = ApiError),
    ),
    security(("bearer" = ["QUALITY_INSPECTION_READ"]))
)]
pub async fn get_inspection(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    tracing_ctx: Option<Extension<TracingContext>>,
    Path(inspection_id): Path<Uuid>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };
    match service::get_inspection(&state.pool, &tenant_id, inspection_id).await {
        Ok(i) => Json(i).into_response(),
        Err(e) => with_request_id(ApiError::from(e), &tracing_ctx).into_response(),
    }
}

// ============================================================================
// Disposition transitions
// ============================================================================

#[utoipa::path(
    post,
    path = "/api/quality-inspection/inspections/{inspection_id}/hold",
    tag = "Disposition",
    params(("inspection_id" = Uuid, Path, description = "Inspection ID")),
    request_body = DispositionTransitionRequest,
    responses(
        (status = 200, description = "Inspection held", body = Inspection),
        (status = 401, description = "Unauthorized"),
        (status = 404, description = "Not found", body = ApiError),
        (status = 422, description = "Validation error", body = ApiError),
        (status = 500, description = "Internal error", body = ApiError),
    ),
    security(("bearer" = ["QUALITY_INSPECTION_MUTATE"]))
)]
pub async fn post_hold_inspection(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    tracing_ctx: Option<Extension<TracingContext>>,
    Path(inspection_id): Path<Uuid>,
    Json(req): Json<DispositionTransitionRequest>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };
    match service::hold_inspection(
        &state.pool,
        &state.wc_client,
        &tenant_id,
        inspection_id,
        req.inspector_id,
        req.reason.as_deref(),
        &correlation_id(),
        None,
    )
    .await
    {
        Ok(i) => Json(i).into_response(),
        Err(e) => with_request_id(ApiError::from(e), &tracing_ctx).into_response(),
    }
}

#[utoipa::path(
    post,
    path = "/api/quality-inspection/inspections/{inspection_id}/release",
    tag = "Disposition",
    params(("inspection_id" = Uuid, Path, description = "Inspection ID")),
    request_body = DispositionTransitionRequest,
    responses(
        (status = 200, description = "Inspection released", body = Inspection),
        (status = 401, description = "Unauthorized"),
        (status = 404, description = "Not found", body = ApiError),
        (status = 422, description = "Validation error", body = ApiError),
        (status = 500, description = "Internal error", body = ApiError),
    ),
    security(("bearer" = ["QUALITY_INSPECTION_MUTATE"]))
)]
pub async fn post_release_inspection(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    tracing_ctx: Option<Extension<TracingContext>>,
    Path(inspection_id): Path<Uuid>,
    Json(req): Json<DispositionTransitionRequest>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };
    match service::release_inspection(
        &state.pool,
        &state.wc_client,
        &tenant_id,
        inspection_id,
        req.inspector_id,
        req.reason.as_deref(),
        &correlation_id(),
        None,
    )
    .await
    {
        Ok(i) => Json(i).into_response(),
        Err(e) => with_request_id(ApiError::from(e), &tracing_ctx).into_response(),
    }
}

#[utoipa::path(
    post,
    path = "/api/quality-inspection/inspections/{inspection_id}/accept",
    tag = "Disposition",
    params(("inspection_id" = Uuid, Path, description = "Inspection ID")),
    request_body = DispositionTransitionRequest,
    responses(
        (status = 200, description = "Inspection accepted", body = Inspection),
        (status = 401, description = "Unauthorized"),
        (status = 404, description = "Not found", body = ApiError),
        (status = 422, description = "Validation error", body = ApiError),
        (status = 500, description = "Internal error", body = ApiError),
    ),
    security(("bearer" = ["QUALITY_INSPECTION_MUTATE"]))
)]
pub async fn post_accept_inspection(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    tracing_ctx: Option<Extension<TracingContext>>,
    Path(inspection_id): Path<Uuid>,
    Json(req): Json<DispositionTransitionRequest>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };
    match service::accept_inspection(
        &state.pool,
        &state.wc_client,
        &tenant_id,
        inspection_id,
        req.inspector_id,
        req.reason.as_deref(),
        &correlation_id(),
        None,
    )
    .await
    {
        Ok(i) => Json(i).into_response(),
        Err(e) => with_request_id(ApiError::from(e), &tracing_ctx).into_response(),
    }
}

#[utoipa::path(
    post,
    path = "/api/quality-inspection/inspections/{inspection_id}/reject",
    tag = "Disposition",
    params(("inspection_id" = Uuid, Path, description = "Inspection ID")),
    request_body = DispositionTransitionRequest,
    responses(
        (status = 200, description = "Inspection rejected", body = Inspection),
        (status = 401, description = "Unauthorized"),
        (status = 404, description = "Not found", body = ApiError),
        (status = 422, description = "Validation error", body = ApiError),
        (status = 500, description = "Internal error", body = ApiError),
    ),
    security(("bearer" = ["QUALITY_INSPECTION_MUTATE"]))
)]
pub async fn post_reject_inspection(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    tracing_ctx: Option<Extension<TracingContext>>,
    Path(inspection_id): Path<Uuid>,
    Json(req): Json<DispositionTransitionRequest>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };
    match service::reject_inspection(
        &state.pool,
        &state.wc_client,
        &tenant_id,
        inspection_id,
        req.inspector_id,
        req.reason.as_deref(),
        &correlation_id(),
        None,
    )
    .await
    {
        Ok(i) => Json(i).into_response(),
        Err(e) => with_request_id(ApiError::from(e), &tracing_ctx).into_response(),
    }
}

// ============================================================================
// In-Process Inspections
// ============================================================================

#[utoipa::path(
    post,
    path = "/api/quality-inspection/inspections/in-process",
    tag = "Inspections",
    request_body = CreateInProcessInspectionRequest,
    responses(
        (status = 201, description = "In-process inspection created", body = Inspection),
        (status = 401, description = "Unauthorized"),
        (status = 422, description = "Validation error", body = ApiError),
        (status = 500, description = "Internal error", body = ApiError),
    ),
    security(("bearer" = ["QUALITY_INSPECTION_MUTATE"]))
)]
pub async fn post_in_process_inspection(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    tracing_ctx: Option<Extension<TracingContext>>,
    Json(req): Json<CreateInProcessInspectionRequest>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };
    match service::create_in_process_inspection(
        &state.pool,
        &tenant_id,
        &req,
        &correlation_id(),
        None,
    )
    .await
    {
        Ok(inspection) => (StatusCode::CREATED, Json(inspection)).into_response(),
        Err(e) => with_request_id(ApiError::from(e), &tracing_ctx).into_response(),
    }
}

// ============================================================================
// Final Inspections
// ============================================================================

#[utoipa::path(
    post,
    path = "/api/quality-inspection/inspections/final",
    tag = "Inspections",
    request_body = CreateFinalInspectionRequest,
    responses(
        (status = 201, description = "Final inspection created", body = Inspection),
        (status = 401, description = "Unauthorized"),
        (status = 422, description = "Validation error", body = ApiError),
        (status = 500, description = "Internal error", body = ApiError),
    ),
    security(("bearer" = ["QUALITY_INSPECTION_MUTATE"]))
)]
pub async fn post_final_inspection(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    tracing_ctx: Option<Extension<TracingContext>>,
    Json(req): Json<CreateFinalInspectionRequest>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };
    match service::create_final_inspection(
        &state.pool,
        &tenant_id,
        &req,
        &correlation_id(),
        None,
    )
    .await
    {
        Ok(inspection) => (StatusCode::CREATED, Json(inspection)).into_response(),
        Err(e) => with_request_id(ApiError::from(e), &tracing_ctx).into_response(),
    }
}

// ============================================================================
// Queries (paginated)
// ============================================================================

#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct InspectionsByPartRevPaginatedQuery {
    pub part_id: Uuid,
    pub part_revision: Option<String>,
    pub page: Option<i64>,
    pub page_size: Option<i64>,
}

#[utoipa::path(
    get,
    path = "/api/quality-inspection/inspections/by-part-rev",
    tag = "Queries",
    params(InspectionsByPartRevPaginatedQuery),
    responses(
        (status = 200, description = "Paginated inspections", body = PaginatedResponse<Inspection>),
        (status = 401, description = "Unauthorized"),
        (status = 500, description = "Internal error", body = ApiError),
    ),
    security(("bearer" = ["QUALITY_INSPECTION_READ"]))
)]
pub async fn get_inspections_by_part_rev(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    tracing_ctx: Option<Extension<TracingContext>>,
    Query(q): Query<InspectionsByPartRevPaginatedQuery>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };
    let page = q.page.unwrap_or(1).max(1);
    let page_size = q.page_size.unwrap_or(50).clamp(1, 200);
    let offset = (page - 1) * page_size;

    match service::list_inspections_by_part_rev_paginated(
        &state.pool,
        &tenant_id,
        q.part_id,
        q.part_revision.as_deref(),
        page_size,
        offset,
    )
    .await
    {
        Ok((rows, total)) => {
            Json(PaginatedResponse::new(rows, page, page_size, total)).into_response()
        }
        Err(e) => with_request_id(ApiError::from(e), &tracing_ctx).into_response(),
    }
}

#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct InspectionsByReceiptPaginatedQuery {
    pub receipt_id: Uuid,
    pub page: Option<i64>,
    pub page_size: Option<i64>,
}

#[utoipa::path(
    get,
    path = "/api/quality-inspection/inspections/by-receipt",
    tag = "Queries",
    params(InspectionsByReceiptPaginatedQuery),
    responses(
        (status = 200, description = "Paginated inspections", body = PaginatedResponse<Inspection>),
        (status = 401, description = "Unauthorized"),
        (status = 500, description = "Internal error", body = ApiError),
    ),
    security(("bearer" = ["QUALITY_INSPECTION_READ"]))
)]
pub async fn get_inspections_by_receipt(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    tracing_ctx: Option<Extension<TracingContext>>,
    Query(q): Query<InspectionsByReceiptPaginatedQuery>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };
    let page = q.page.unwrap_or(1).max(1);
    let page_size = q.page_size.unwrap_or(50).clamp(1, 200);
    let offset = (page - 1) * page_size;

    match service::list_inspections_by_receipt_paginated(
        &state.pool,
        &tenant_id,
        q.receipt_id,
        page_size,
        offset,
    )
    .await
    {
        Ok((rows, total)) => {
            Json(PaginatedResponse::new(rows, page, page_size, total)).into_response()
        }
        Err(e) => with_request_id(ApiError::from(e), &tracing_ctx).into_response(),
    }
}

#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct InspectionsByWoPaginatedQuery {
    pub wo_id: Uuid,
    pub inspection_type: Option<String>,
    pub page: Option<i64>,
    pub page_size: Option<i64>,
}

#[utoipa::path(
    get,
    path = "/api/quality-inspection/inspections/by-wo",
    tag = "Queries",
    params(InspectionsByWoPaginatedQuery),
    responses(
        (status = 200, description = "Paginated inspections", body = PaginatedResponse<Inspection>),
        (status = 401, description = "Unauthorized"),
        (status = 500, description = "Internal error", body = ApiError),
    ),
    security(("bearer" = ["QUALITY_INSPECTION_READ"]))
)]
pub async fn get_inspections_by_wo(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    tracing_ctx: Option<Extension<TracingContext>>,
    Query(q): Query<InspectionsByWoPaginatedQuery>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };
    let page = q.page.unwrap_or(1).max(1);
    let page_size = q.page_size.unwrap_or(50).clamp(1, 200);
    let offset = (page - 1) * page_size;

    match service::list_inspections_by_wo_paginated(
        &state.pool,
        &tenant_id,
        q.wo_id,
        q.inspection_type.as_deref(),
        page_size,
        offset,
    )
    .await
    {
        Ok((rows, total)) => {
            Json(PaginatedResponse::new(rows, page, page_size, total)).into_response()
        }
        Err(e) => with_request_id(ApiError::from(e), &tracing_ctx).into_response(),
    }
}

#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct InspectionsByLotPaginatedQuery {
    pub lot_id: Uuid,
    pub page: Option<i64>,
    pub page_size: Option<i64>,
}

#[utoipa::path(
    get,
    path = "/api/quality-inspection/inspections/by-lot",
    tag = "Queries",
    params(InspectionsByLotPaginatedQuery),
    responses(
        (status = 200, description = "Paginated inspections", body = PaginatedResponse<Inspection>),
        (status = 401, description = "Unauthorized"),
        (status = 500, description = "Internal error", body = ApiError),
    ),
    security(("bearer" = ["QUALITY_INSPECTION_READ"]))
)]
pub async fn get_inspections_by_lot(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    tracing_ctx: Option<Extension<TracingContext>>,
    Query(q): Query<InspectionsByLotPaginatedQuery>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };
    let page = q.page.unwrap_or(1).max(1);
    let page_size = q.page_size.unwrap_or(50).clamp(1, 200);
    let offset = (page - 1) * page_size;

    match service::list_inspections_by_lot_paginated(
        &state.pool,
        &tenant_id,
        q.lot_id,
        page_size,
        offset,
    )
    .await
    {
        Ok((rows, total)) => {
            Json(PaginatedResponse::new(rows, page, page_size, total)).into_response()
        }
        Err(e) => with_request_id(ApiError::from(e), &tracing_ctx).into_response(),
    }
}
