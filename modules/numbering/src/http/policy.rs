//! Policy management HTTP handlers.
//!
//! PUT  /policies/:entity — upsert a numbering policy (Guard → Mutation → Outbox)
//! GET  /policies/:entity — read a policy

use axum::{
    extract::{Path, State},
    http::StatusCode,
    Extension, Json,
};
use security::VerifiedClaims;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use uuid::Uuid;

use crate::{outbox, policy};

use super::allocate::ErrorResponse;

// ── Request / Response types ────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct UpsertPolicyRequest {
    pub pattern: String,
    #[serde(default)]
    pub prefix: String,
    #[serde(default)]
    pub padding: i32,
}

#[derive(Debug, Serialize)]
pub struct PolicyResponse {
    pub tenant_id: String,
    pub entity: String,
    pub pattern: String,
    pub prefix: String,
    pub padding: i32,
    pub version: i32,
}

// ── Event payload ───────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
struct PolicyUpdatedPayload {
    pub tenant_id: String,
    pub entity: String,
    pub pattern: String,
    pub prefix: String,
    pub padding: i32,
    pub version: i32,
}

// ── Handlers ────────────────────────────────────────────────────────────

/// PUT /policies/:entity
///
/// Guard → Mutation → Outbox in a single transaction.
pub async fn upsert_policy(
    State(state): State<Arc<crate::AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(entity): Path<String>,
    Json(req): Json<UpsertPolicyRequest>,
) -> Result<(StatusCode, Json<PolicyResponse>), (StatusCode, Json<ErrorResponse>)> {
    let tenant_id = extract_tenant(&claims)?;

    // Guard: validate inputs
    validate_entity(&entity)?;
    validate_pattern(&req.pattern)?;
    validate_prefix(&req.prefix)?;
    validate_padding(req.padding)?;

    // Begin transaction: Guard → Mutation → Outbox
    let mut tx = state.pool.begin().await.map_err(db_error)?;

    // Mutation: upsert the policy
    let row = policy::upsert_policy_tx(
        &mut tx,
        tenant_id,
        &entity,
        &req.pattern,
        &req.prefix,
        req.padding,
    )
    .await
    .map_err(db_error)?;

    // Outbox: emit policy.updated event
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
    .map_err(db_error)?;

    tx.commit().await.map_err(db_error)?;

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

/// GET /policies/:entity
pub async fn get_policy(
    State(state): State<Arc<crate::AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(entity): Path<String>,
) -> Result<Json<PolicyResponse>, (StatusCode, Json<ErrorResponse>)> {
    let tenant_id = extract_tenant(&claims)?;

    let row = policy::get_policy(&state.pool, tenant_id, &entity)
        .await
        .map_err(db_error)?
        .ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                Json(ErrorResponse {
                    error: "not_found".to_string(),
                    message: format!("No policy for entity '{}'", entity),
                }),
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

// ── Helpers ─────────────────────────────────────────────────────────────

fn extract_tenant(
    claims: &Option<Extension<VerifiedClaims>>,
) -> Result<Uuid, (StatusCode, Json<ErrorResponse>)> {
    match claims {
        Some(Extension(c)) => Ok(c.tenant_id),
        None => Err((
            StatusCode::UNAUTHORIZED,
            Json(ErrorResponse {
                error: "unauthorized".to_string(),
                message: "Missing or invalid authentication".to_string(),
            }),
        )),
    }
}

fn validate_entity(entity: &str) -> Result<(), (StatusCode, Json<ErrorResponse>)> {
    if entity.is_empty() || entity.len() > 100 {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: "invalid_entity".to_string(),
                message: "entity must be 1-100 characters".to_string(),
            }),
        ));
    }
    Ok(())
}

fn validate_pattern(pattern: &str) -> Result<(), (StatusCode, Json<ErrorResponse>)> {
    if pattern.is_empty() || pattern.len() > 255 {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: "invalid_pattern".to_string(),
                message: "pattern must be 1-255 characters".to_string(),
            }),
        ));
    }
    if !pattern.contains("{number}") {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: "invalid_pattern".to_string(),
                message: "pattern must contain {number} token".to_string(),
            }),
        ));
    }
    Ok(())
}

fn validate_prefix(prefix: &str) -> Result<(), (StatusCode, Json<ErrorResponse>)> {
    if prefix.len() > 50 {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: "invalid_prefix".to_string(),
                message: "prefix must be at most 50 characters".to_string(),
            }),
        ));
    }
    Ok(())
}

fn validate_padding(padding: i32) -> Result<(), (StatusCode, Json<ErrorResponse>)> {
    if padding < 0 || padding > 20 {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: "invalid_padding".to_string(),
                message: "padding must be between 0 and 20".to_string(),
            }),
        ));
    }
    Ok(())
}

fn db_error(e: sqlx::Error) -> (StatusCode, Json<ErrorResponse>) {
    tracing::error!("Numbering: database error: {}", e);
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(ErrorResponse {
            error: "database_error".to_string(),
            message: "Internal database error".to_string(),
        }),
    )
}
