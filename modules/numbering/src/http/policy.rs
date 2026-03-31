//! Policy management HTTP handlers.
//!
//! PUT  /policies/:entity — upsert a numbering policy (Guard → Mutation → Outbox)
//! GET  /policies/:entity — read a policy

use axum::{
    extract::{Path, State},
    http::StatusCode,
    Extension, Json,
};
use event_bus::TracingContext;
use platform_http_contracts::ApiError;
use security::VerifiedClaims;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use utoipa::ToSchema;
use uuid::Uuid;

use super::tenant::{extract_tenant, with_request_id};
use crate::{outbox, policy};

#[derive(Debug, Deserialize, ToSchema)]
pub struct UpsertPolicyRequest {
    pub pattern: String,
    #[serde(default)]
    pub prefix: String,
    #[serde(default)]
    pub padding: i32,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct PolicyResponse {
    pub tenant_id: String,
    pub entity: String,
    pub pattern: String,
    pub prefix: String,
    pub padding: i32,
    pub version: i32,
}

#[derive(Debug, Serialize)]
struct PolicyUpdatedPayload {
    pub tenant_id: String,
    pub entity: String,
    pub pattern: String,
    pub prefix: String,
    pub padding: i32,
    pub version: i32,
}

#[utoipa::path(
    put, path = "/policies/{entity}", tag = "Policies",
    params(("entity" = String, Path)),
    request_body = UpsertPolicyRequest,
    responses((status = 200, body = PolicyResponse), (status = 400, body = ApiError)),
    security(("bearer" = [])),
)]
pub async fn upsert_policy(
    State(state): State<Arc<crate::AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    ctx: Option<Extension<TracingContext>>,
    Path(entity): Path<String>,
    Json(req): Json<UpsertPolicyRequest>,
) -> Result<(StatusCode, Json<PolicyResponse>), ApiError> {
    let tenant_id = extract_tenant(&claims)
        .map_err(|e| with_request_id(e, &ctx))?;

    validate_entity(&entity, &ctx)?;
    validate_pattern(&req.pattern, &ctx)?;
    validate_prefix(&req.prefix, &ctx)?;
    validate_padding(req.padding, &ctx)?;

    let mut tx = state.pool.begin().await.map_err(|e| {
        tracing::error!("Numbering: policy begin tx failed: {}", e);
        with_request_id(ApiError::internal("Database error"), &ctx)
    })?;

    let row = policy::upsert_policy_tx(
        &mut tx,
        tenant_id,
        &entity,
        &req.pattern,
        &req.prefix,
        req.padding,
    )
    .await
    .map_err(|e| {
        tracing::error!("Numbering: policy upsert failed: {}", e);
        with_request_id(ApiError::internal("Database error"), &ctx)
    })?;

    let event_payload = PolicyUpdatedPayload {
        tenant_id: tenant_id.to_string(),
        entity: entity.clone(),
        pattern: row.pattern.clone(),
        prefix: row.prefix.clone(),
        padding: row.padding,
        version: row.version,
    };

    outbox::enqueue_event_tx(
        &mut tx,
        Uuid::new_v4(),
        "numbering.events.policy.updated",
        "policy",
        &format!("{}:{}", tenant_id, entity),
        &event_payload,
    )
    .await
    .map_err(|e| {
        tracing::error!("Numbering: policy outbox failed: {}", e);
        with_request_id(ApiError::internal("Database error"), &ctx)
    })?;

    tx.commit().await.map_err(|e| {
        tracing::error!("Numbering: policy commit failed: {}", e);
        with_request_id(ApiError::internal("Database error"), &ctx)
    })?;

    tracing::info!(
        tenant_id = %tenant_id,
        entity = %entity,
        version = row.version,
        "Numbering: policy updated"
    );

    Ok((
        StatusCode::OK,
        Json(PolicyResponse {
            tenant_id: tenant_id.to_string(),
            entity,
            pattern: row.pattern,
            prefix: row.prefix,
            padding: row.padding,
            version: row.version,
        }),
    ))
}

#[utoipa::path(
    get, path = "/policies/{entity}", tag = "Policies",
    params(("entity" = String, Path)),
    responses((status = 200, body = PolicyResponse), (status = 404, body = ApiError)),
    security(("bearer" = [])),
)]
pub async fn get_policy(
    State(state): State<Arc<crate::AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    ctx: Option<Extension<TracingContext>>,
    Path(entity): Path<String>,
) -> Result<Json<PolicyResponse>, ApiError> {
    let tenant_id = extract_tenant(&claims)
        .map_err(|e| with_request_id(e, &ctx))?;

    let row = policy::get_policy(&state.pool, tenant_id, &entity)
        .await
        .map_err(|e| {
            tracing::error!("Numbering: get policy failed: {}", e);
            with_request_id(ApiError::internal("Database error"), &ctx)
        })?
        .ok_or_else(|| {
            with_request_id(
                ApiError::not_found(format!("No policy for entity '{}'", entity)),
                &ctx,
            )
        })?;

    Ok(Json(PolicyResponse {
        tenant_id: tenant_id.to_string(),
        entity: row.entity,
        pattern: row.pattern,
        prefix: row.prefix,
        padding: row.padding,
        version: row.version,
    }))
}

fn validate_entity(entity: &str, ctx: &Option<Extension<TracingContext>>) -> Result<(), ApiError> {
    if entity.is_empty() || entity.len() > 100 {
        return Err(with_request_id(
            ApiError::bad_request("entity must be 1-100 characters"),
            ctx,
        ));
    }
    Ok(())
}

fn validate_pattern(pattern: &str, ctx: &Option<Extension<TracingContext>>) -> Result<(), ApiError> {
    if pattern.is_empty() || pattern.len() > 255 {
        return Err(with_request_id(
            ApiError::bad_request("pattern must be 1-255 characters"),
            ctx,
        ));
    }
    if !pattern.contains("{number}") {
        return Err(with_request_id(
            ApiError::bad_request("pattern must contain {number} token"),
            ctx,
        ));
    }
    Ok(())
}

fn validate_prefix(prefix: &str, ctx: &Option<Extension<TracingContext>>) -> Result<(), ApiError> {
    if prefix.len() > 50 {
        return Err(with_request_id(
            ApiError::bad_request("prefix must be at most 50 characters"),
            ctx,
        ));
    }
    Ok(())
}

fn validate_padding(
    padding: i32,
    ctx: &Option<Extension<TracingContext>>,
) -> Result<(), ApiError> {
    if !(0..=20).contains(&padding) {
        return Err(with_request_id(
            ApiError::bad_request("padding must be between 0 and 20"),
            ctx,
        ));
    }
    Ok(())
}
