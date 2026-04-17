//! Barcode resolution HTTP handlers.
//!
//! Endpoints:
//!   POST /api/inventory/barcode/resolve              — resolve a single barcode
//!   POST /api/inventory/barcode/resolve/batch        — batch resolve (max 100, 256 chars each)
//!   POST /api/inventory/barcode/rules/test           — test barcode against rules (no side effects)
//!   GET  /api/inventory/barcode/rules                — list all tenant rules
//!   POST /api/inventory/barcode/rules                — create a rule
//!   PUT  /api/inventory/barcode/rules/:id            — update a rule
//!   POST /api/inventory/barcode/rules/:id/deactivate — soft deactivate a rule

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Extension, Json,
};
use event_bus::TracingContext;
use platform_http_contracts::ApiError;
use security::VerifiedClaims;
use serde::Deserialize;
use std::sync::Arc;
use uuid::Uuid;

use super::tenant::with_request_id;
use crate::{
    domain::barcode_resolver::{
        self, BatchResolveRequest, CreateRuleRequest, ResolveRequest, TestRuleRequest,
        UpdateRuleRequest, BARCODE_MAX_LEN, BATCH_MAX_BARCODES,
    },
    AppState,
};
use platform_sdk::extract_tenant;

// ============================================================================
// Deactivate body
// ============================================================================

#[derive(Debug, Deserialize)]
pub struct DeactivateRuleBody {
    pub updated_by: Option<String>,
}

// ============================================================================
// Rule handlers
// ============================================================================

#[utoipa::path(
    get,
    path = "/api/inventory/barcode/rules",
    tag = "Barcode",
    responses(
        (status = 200, description = "List of barcode format rules"),
        (status = 401, description = "Unauthorized"),
    ),
    security(("bearer" = [])),
)]
pub async fn get_list_barcode_rules(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    tracing_ctx: Option<Extension<TracingContext>>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };

    match barcode_resolver::list_rules(&state.pool, &tenant_id).await {
        Ok(rules) => (StatusCode::OK, Json(rules)).into_response(),
        Err(e) => {
            let api_err: ApiError = e.into();
            with_request_id(api_err, &tracing_ctx).into_response()
        }
    }
}

#[utoipa::path(
    post,
    path = "/api/inventory/barcode/rules",
    tag = "Barcode",
    request_body = CreateRuleRequest,
    responses(
        (status = 201, description = "Rule created"),
        (status = 422, description = "Validation error or invalid regex", body = ApiError),
    ),
    security(("bearer" = [])),
)]
pub async fn post_create_barcode_rule(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    tracing_ctx: Option<Extension<TracingContext>>,
    Json(mut req): Json<CreateRuleRequest>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };
    req.tenant_id = tenant_id;

    match barcode_resolver::create_rule(&state.pool, &req).await {
        Ok(rule) => (StatusCode::CREATED, Json(rule)).into_response(),
        Err(e) => {
            let api_err: ApiError = e.into();
            with_request_id(api_err, &tracing_ctx).into_response()
        }
    }
}

#[utoipa::path(
    put,
    path = "/api/inventory/barcode/rules/{id}",
    tag = "Barcode",
    params(("id" = Uuid, Path, description = "Rule ID")),
    request_body = UpdateRuleRequest,
    responses(
        (status = 200, description = "Rule updated"),
        (status = 404, description = "Rule not found", body = ApiError),
        (status = 422, description = "Validation error or invalid regex", body = ApiError),
    ),
    security(("bearer" = [])),
)]
pub async fn put_update_barcode_rule(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(rule_id): Path<Uuid>,
    tracing_ctx: Option<Extension<TracingContext>>,
    Json(mut req): Json<UpdateRuleRequest>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };
    req.tenant_id = tenant_id.clone();

    match barcode_resolver::update_rule(&state.pool, &tenant_id, rule_id, &req).await {
        Ok(rule) => (StatusCode::OK, Json(rule)).into_response(),
        Err(e) => {
            let api_err: ApiError = e.into();
            with_request_id(api_err, &tracing_ctx).into_response()
        }
    }
}

#[utoipa::path(
    post,
    path = "/api/inventory/barcode/rules/{id}/deactivate",
    tag = "Barcode",
    params(("id" = Uuid, Path, description = "Rule ID")),
    responses(
        (status = 200, description = "Rule deactivated"),
        (status = 404, description = "Rule not found", body = ApiError),
    ),
    security(("bearer" = [])),
)]
pub async fn post_deactivate_barcode_rule(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(rule_id): Path<Uuid>,
    tracing_ctx: Option<Extension<TracingContext>>,
    Json(body): Json<DeactivateRuleBody>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };

    match barcode_resolver::deactivate_rule(
        &state.pool,
        &tenant_id,
        rule_id,
        body.updated_by.as_deref(),
    )
    .await
    {
        Ok(rule) => (StatusCode::OK, Json(rule)).into_response(),
        Err(e) => {
            let api_err: ApiError = e.into();
            with_request_id(api_err, &tracing_ctx).into_response()
        }
    }
}

// ============================================================================
// Resolution handlers
// ============================================================================

#[utoipa::path(
    post,
    path = "/api/inventory/barcode/resolve",
    tag = "Barcode",
    request_body = ResolveRequest,
    responses(
        (status = 200, description = "Resolution result"),
    ),
    security(("bearer" = [])),
)]
pub async fn post_resolve_barcode(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    tracing_ctx: Option<Extension<TracingContext>>,
    Json(mut req): Json<ResolveRequest>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };
    req.tenant_id = tenant_id;

    if req.barcode_raw.len() > BARCODE_MAX_LEN {
        return with_request_id(
            ApiError::new(413, "payload_too_large", format!(
                "barcode_raw exceeds maximum length of {} characters",
                BARCODE_MAX_LEN
            )),
            &tracing_ctx,
        )
        .into_response();
    }

    match barcode_resolver::resolve(&state.pool, &req).await {
        Ok(result) => (StatusCode::OK, Json(result)).into_response(),
        Err(e) => {
            let api_err: ApiError = e.into();
            with_request_id(api_err, &tracing_ctx).into_response()
        }
    }
}

#[utoipa::path(
    post,
    path = "/api/inventory/barcode/resolve/batch",
    tag = "Barcode",
    request_body = BatchResolveRequest,
    responses(
        (status = 200, description = "Array of resolution results"),
        (status = 413, description = "Batch too large (>100) or barcode too long (>256)", body = ApiError),
    ),
    security(("bearer" = [])),
)]
pub async fn post_resolve_barcode_batch(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    tracing_ctx: Option<Extension<TracingContext>>,
    Json(mut req): Json<BatchResolveRequest>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };
    req.tenant_id = tenant_id;

    // Enforce batch cap before any allocation
    if req.barcodes.len() > BATCH_MAX_BARCODES {
        return with_request_id(
            ApiError::new(413, "payload_too_large", format!(
                "Batch exceeds maximum of {} barcodes",
                BATCH_MAX_BARCODES
            )),
            &tracing_ctx,
        )
        .into_response();
    }

    // Enforce per-barcode length cap
    for barcode in &req.barcodes {
        if barcode.len() > BARCODE_MAX_LEN {
            return with_request_id(
                ApiError::new(413, "payload_too_large", format!(
                    "A barcode in the batch exceeds maximum length of {} characters",
                    BARCODE_MAX_LEN
                )),
                &tracing_ctx,
            )
            .into_response();
        }
    }

    match barcode_resolver::resolve_batch(&state.pool, &req).await {
        Ok(results) => (StatusCode::OK, Json(results)).into_response(),
        Err(e) => {
            let api_err: ApiError = e.into();
            with_request_id(api_err, &tracing_ctx).into_response()
        }
    }
}

#[utoipa::path(
    post,
    path = "/api/inventory/barcode/rules/test",
    tag = "Barcode",
    request_body = TestRuleRequest,
    responses(
        (status = 200, description = "Test result — which rule matched, if any (no side effects)"),
    ),
    security(("bearer" = [])),
)]
pub async fn post_test_barcode_rule(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    tracing_ctx: Option<Extension<TracingContext>>,
    Json(mut req): Json<TestRuleRequest>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return with_request_id(e, &tracing_ctx).into_response(),
    };
    req.tenant_id = tenant_id;

    match barcode_resolver::test_barcode(&state.pool, &req).await {
        Ok(result) => (StatusCode::OK, Json(result)).into_response(),
        Err(e) => {
            let api_err: ApiError = e.into();
            with_request_id(api_err, &tracing_ctx).into_response()
        }
    }
}
