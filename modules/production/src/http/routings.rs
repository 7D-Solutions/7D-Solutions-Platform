use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    Extension, Json,
};
use chrono::NaiveDate;
use security::VerifiedClaims;
use serde::Deserialize;
use serde_json::json;
use std::sync::Arc;
use uuid::Uuid;

use super::tenant::extract_tenant;
use crate::{
    domain::routings::{
        AddRoutingStepRequest, CreateRoutingRequest, RoutingError, RoutingRepo,
        UpdateRoutingRequest,
    },
    AppState,
};

fn routing_error_response(err: RoutingError) -> impl IntoResponse {
    match err {
        RoutingError::DuplicateRevision(rev, tenant) => (
            StatusCode::CONFLICT,
            Json(json!({
                "error": "duplicate_revision",
                "message": format!(
                    "Routing revision '{}' already exists for item in tenant '{}'",
                    rev, tenant
                )
            })),
        )
            .into_response(),
        RoutingError::DuplicateSequence(seq) => (
            StatusCode::CONFLICT,
            Json(json!({
                "error": "duplicate_sequence",
                "message": format!("Sequence number {} already exists for this routing", seq)
            })),
        )
            .into_response(),
        RoutingError::WorkcenterInvalid(id) => (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({
                "error": "workcenter_invalid",
                "message": format!("Workcenter '{}' not found or inactive", id)
            })),
        )
            .into_response(),
        RoutingError::NotFound => (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "not_found", "message": "Routing not found" })),
        )
            .into_response(),
        RoutingError::InvalidTransition { from, to } => (
            StatusCode::CONFLICT,
            Json(json!({
                "error": "invalid_transition",
                "message": format!("Cannot transition from '{}' to '{}'", from, to)
            })),
        )
            .into_response(),
        RoutingError::ReleasedImmutable => (
            StatusCode::CONFLICT,
            Json(json!({
                "error": "released_immutable",
                "message": "Cannot modify a released routing"
            })),
        )
            .into_response(),
        RoutingError::Validation(msg) => (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({ "error": "validation_error", "message": msg })),
        )
            .into_response(),
        RoutingError::Database(e) => {
            tracing::error!(error = %e, "routing database error");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": "internal_error", "message": "Database error" })),
            )
                .into_response()
        }
    }
}

/// POST /api/production/routings
pub async fn create_routing(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Json(mut req): Json<CreateRoutingRequest>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };
    req.tenant_id = tenant_id;
    let corr = Uuid::new_v4().to_string();
    match RoutingRepo::create(&state.pool, &req, &corr, None).await {
        Ok(rt) => (StatusCode::CREATED, Json(json!(rt))).into_response(),
        Err(e) => routing_error_response(e).into_response(),
    }
}

/// GET /api/production/routings/:id
pub async fn get_routing(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
    claims: Option<Extension<VerifiedClaims>>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };
    match RoutingRepo::find_by_id(&state.pool, id, &tenant_id).await {
        Ok(Some(rt)) => (StatusCode::OK, Json(json!(rt))).into_response(),
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "not_found", "message": "Routing not found" })),
        )
            .into_response(),
        Err(e) => routing_error_response(e).into_response(),
    }
}

/// GET /api/production/routings
pub async fn list_routings(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };
    match RoutingRepo::list(&state.pool, &tenant_id).await {
        Ok(rts) => (StatusCode::OK, Json(json!(rts))).into_response(),
        Err(e) => routing_error_response(e).into_response(),
    }
}

#[derive(Debug, Deserialize)]
pub struct ItemDateQuery {
    pub item_id: Uuid,
    pub effective_date: NaiveDate,
}

/// GET /api/production/routings/by-item?item_id=...&effective_date=...
pub async fn find_routings_by_item(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Query(params): Query<ItemDateQuery>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };
    match RoutingRepo::find_by_item_and_date(
        &state.pool,
        &tenant_id,
        params.item_id,
        params.effective_date,
    )
    .await
    {
        Ok(rts) => (StatusCode::OK, Json(json!(rts))).into_response(),
        Err(e) => routing_error_response(e).into_response(),
    }
}

/// PUT /api/production/routings/:id
pub async fn update_routing(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(id): Path<Uuid>,
    Json(mut req): Json<UpdateRoutingRequest>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };
    req.tenant_id = tenant_id;
    let corr = Uuid::new_v4().to_string();
    match RoutingRepo::update(&state.pool, id, &req, &corr, None).await {
        Ok(rt) => (StatusCode::OK, Json(json!(rt))).into_response(),
        Err(e) => routing_error_response(e).into_response(),
    }
}

/// POST /api/production/routings/:id/release
pub async fn release_routing(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };
    let corr = Uuid::new_v4().to_string();
    match RoutingRepo::release(&state.pool, id, &tenant_id, &corr, None).await {
        Ok(rt) => (StatusCode::OK, Json(json!(rt))).into_response(),
        Err(e) => routing_error_response(e).into_response(),
    }
}

/// POST /api/production/routings/:id/steps
pub async fn add_routing_step(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(id): Path<Uuid>,
    Json(mut req): Json<AddRoutingStepRequest>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };
    req.tenant_id = tenant_id;
    let corr = Uuid::new_v4().to_string();
    match RoutingRepo::add_step(&state.pool, id, &req, &corr, None).await {
        Ok(step) => (StatusCode::CREATED, Json(json!(step))).into_response(),
        Err(e) => routing_error_response(e).into_response(),
    }
}

/// GET /api/production/routings/:id/steps
pub async fn list_routing_steps(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };
    match RoutingRepo::list_steps(&state.pool, id, &tenant_id).await {
        Ok(steps) => (StatusCode::OK, Json(json!(steps))).into_response(),
        Err(e) => routing_error_response(e).into_response(),
    }
}
