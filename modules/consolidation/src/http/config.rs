//! HTTP handlers for consolidation config CRUD.
//!
//! Tenant identity derived from JWT `VerifiedClaims` (maps to tenant_id).

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Extension, Json,
};
use security::VerifiedClaims;
use serde::Serialize;
use std::sync::Arc;
use uuid::Uuid;

use crate::domain::config::{self, models::*, service, service_rules, ConfigError};
use crate::AppState;

// ============================================================================
// Helpers
// ============================================================================

fn extract_tenant(
    claims: &Option<Extension<VerifiedClaims>>,
) -> Result<String, (StatusCode, Json<ErrorBody>)> {
    match claims {
        Some(Extension(c)) => Ok(c.tenant_id.to_string()),
        None => Err((
            StatusCode::UNAUTHORIZED,
            Json(ErrorBody::new(
                "unauthorized",
                "Missing or invalid authentication",
            )),
        )),
    }
}

fn config_error_response(e: ConfigError) -> (StatusCode, Json<ErrorBody>) {
    match e {
        ConfigError::GroupNotFound(id) => (
            StatusCode::NOT_FOUND,
            Json(ErrorBody::new("group_not_found", &format!("Group {} not found", id))),
        ),
        ConfigError::EntityNotFound(id) => (
            StatusCode::NOT_FOUND,
            Json(ErrorBody::new("entity_not_found", &format!("Entity {} not found", id))),
        ),
        ConfigError::RuleNotFound(id) => (
            StatusCode::NOT_FOUND,
            Json(ErrorBody::new("rule_not_found", &format!("Rule {} not found", id))),
        ),
        ConfigError::PolicyNotFound(id) => (
            StatusCode::NOT_FOUND,
            Json(ErrorBody::new("policy_not_found", &format!("Policy {} not found", id))),
        ),
        ConfigError::MappingNotFound(id) => (
            StatusCode::NOT_FOUND,
            Json(ErrorBody::new("mapping_not_found", &format!("Mapping {} not found", id))),
        ),
        ConfigError::Validation(msg) => (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(ErrorBody::new("validation_error", &msg)),
        ),
        ConfigError::Conflict(msg) => (
            StatusCode::CONFLICT,
            Json(ErrorBody::new("conflict", &msg)),
        ),
        ConfigError::Database(e) => {
            tracing::error!("Consolidation config DB error: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorBody::new("database_error", "Internal database error")),
            )
        }
    }
}

#[derive(Debug, Serialize)]
pub struct ErrorBody {
    pub error: String,
    pub message: String,
}

impl ErrorBody {
    pub fn new(error: &str, message: &str) -> Self {
        Self { error: error.to_string(), message: message.to_string() }
    }
}

// ============================================================================
// Groups
// ============================================================================

pub async fn create_group(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Json(req): Json<CreateGroupRequest>,
) -> Result<(StatusCode, Json<config::Group>), (StatusCode, Json<ErrorBody>)> {
    let tid = extract_tenant(&claims)?;
    let group = service::create_group(&state.pool, &tid, &req)
        .await
        .map_err(config_error_response)?;
    Ok((StatusCode::CREATED, Json(group)))
}

pub async fn list_groups(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Query(q): Query<ListGroupsQuery>,
) -> Result<Json<Vec<config::Group>>, (StatusCode, Json<ErrorBody>)> {
    let tid = extract_tenant(&claims)?;
    let rows = service::list_groups(&state.pool, &tid, q.include_inactive)
        .await
        .map_err(config_error_response)?;
    Ok(Json(rows))
}

pub async fn get_group(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(id): Path<Uuid>,
) -> Result<Json<config::Group>, (StatusCode, Json<ErrorBody>)> {
    let tid = extract_tenant(&claims)?;
    let group = service::get_group(&state.pool, &tid, id)
        .await
        .map_err(config_error_response)?;
    Ok(Json(group))
}

pub async fn update_group(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(id): Path<Uuid>,
    Json(req): Json<UpdateGroupRequest>,
) -> Result<Json<config::Group>, (StatusCode, Json<ErrorBody>)> {
    let tid = extract_tenant(&claims)?;
    let group = service::update_group(&state.pool, &tid, id, &req)
        .await
        .map_err(config_error_response)?;
    Ok(Json(group))
}

pub async fn delete_group(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, (StatusCode, Json<ErrorBody>)> {
    let tid = extract_tenant(&claims)?;
    service::delete_group(&state.pool, &tid, id)
        .await
        .map_err(config_error_response)?;
    Ok(StatusCode::NO_CONTENT)
}

// ============================================================================
// Entities
// ============================================================================

pub async fn create_entity(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(group_id): Path<Uuid>,
    Json(req): Json<CreateEntityRequest>,
) -> Result<(StatusCode, Json<config::GroupEntity>), (StatusCode, Json<ErrorBody>)> {
    let tid = extract_tenant(&claims)?;
    let entity = service::create_entity(&state.pool, &tid, group_id, &req)
        .await
        .map_err(config_error_response)?;
    Ok((StatusCode::CREATED, Json(entity)))
}

pub async fn list_entities(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(group_id): Path<Uuid>,
    Query(q): Query<ListEntitiesQuery>,
) -> Result<Json<Vec<config::GroupEntity>>, (StatusCode, Json<ErrorBody>)> {
    let tid = extract_tenant(&claims)?;
    let rows = service::list_entities(&state.pool, &tid, group_id, q.include_inactive)
        .await
        .map_err(config_error_response)?;
    Ok(Json(rows))
}

pub async fn get_entity(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
) -> Result<Json<config::GroupEntity>, (StatusCode, Json<ErrorBody>)> {
    let entity = service::get_entity(&state.pool, id)
        .await
        .map_err(config_error_response)?;
    Ok(Json(entity))
}

pub async fn update_entity(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(id): Path<Uuid>,
    Json(req): Json<UpdateEntityRequest>,
) -> Result<Json<config::GroupEntity>, (StatusCode, Json<ErrorBody>)> {
    let tid = extract_tenant(&claims)?;
    let entity = service::update_entity(&state.pool, &tid, id, &req)
        .await
        .map_err(config_error_response)?;
    Ok(Json(entity))
}

pub async fn delete_entity(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, (StatusCode, Json<ErrorBody>)> {
    let tid = extract_tenant(&claims)?;
    service::delete_entity(&state.pool, &tid, id)
        .await
        .map_err(config_error_response)?;
    Ok(StatusCode::NO_CONTENT)
}

// ============================================================================
// COA mappings
// ============================================================================

pub async fn create_coa_mapping(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(group_id): Path<Uuid>,
    Json(req): Json<CreateCoaMappingRequest>,
) -> Result<(StatusCode, Json<config::CoaMapping>), (StatusCode, Json<ErrorBody>)> {
    let tid = extract_tenant(&claims)?;
    let mapping = service::create_coa_mapping(&state.pool, &tid, group_id, &req)
        .await
        .map_err(config_error_response)?;
    Ok((StatusCode::CREATED, Json(mapping)))
}

pub async fn list_coa_mappings(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(group_id): Path<Uuid>,
    Query(q): Query<ListCoaMappingsQuery>,
) -> Result<Json<Vec<config::CoaMapping>>, (StatusCode, Json<ErrorBody>)> {
    let tid = extract_tenant(&claims)?;
    let rows = service::list_coa_mappings(
        &state.pool,
        &tid,
        group_id,
        q.entity_tenant_id.as_deref(),
    )
    .await
    .map_err(config_error_response)?;
    Ok(Json(rows))
}

pub async fn delete_coa_mapping(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, (StatusCode, Json<ErrorBody>)> {
    let tid = extract_tenant(&claims)?;
    service::delete_coa_mapping(&state.pool, &tid, id)
        .await
        .map_err(config_error_response)?;
    Ok(StatusCode::NO_CONTENT)
}

// ============================================================================
// Elimination rules
// ============================================================================

pub async fn create_elimination_rule(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(group_id): Path<Uuid>,
    Json(req): Json<CreateEliminationRuleRequest>,
) -> Result<(StatusCode, Json<config::EliminationRule>), (StatusCode, Json<ErrorBody>)> {
    let tid = extract_tenant(&claims)?;
    let rule = service_rules::create_elimination_rule(&state.pool, &tid, group_id, &req)
        .await
        .map_err(config_error_response)?;
    Ok((StatusCode::CREATED, Json(rule)))
}

pub async fn list_elimination_rules(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(group_id): Path<Uuid>,
    Query(q): Query<ListEliminationRulesQuery>,
) -> Result<Json<Vec<config::EliminationRule>>, (StatusCode, Json<ErrorBody>)> {
    let tid = extract_tenant(&claims)?;
    let rows =
        service_rules::list_elimination_rules(&state.pool, &tid, group_id, q.include_inactive)
            .await
            .map_err(config_error_response)?;
    Ok(Json(rows))
}

pub async fn get_elimination_rule(
    State(state): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
) -> Result<Json<config::EliminationRule>, (StatusCode, Json<ErrorBody>)> {
    let rule = service_rules::get_elimination_rule(&state.pool, id)
        .await
        .map_err(config_error_response)?;
    Ok(Json(rule))
}

pub async fn update_elimination_rule(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(id): Path<Uuid>,
    Json(req): Json<UpdateEliminationRuleRequest>,
) -> Result<Json<config::EliminationRule>, (StatusCode, Json<ErrorBody>)> {
    let tid = extract_tenant(&claims)?;
    let rule = service_rules::update_elimination_rule(&state.pool, &tid, id, &req)
        .await
        .map_err(config_error_response)?;
    Ok(Json(rule))
}

pub async fn delete_elimination_rule(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, (StatusCode, Json<ErrorBody>)> {
    let tid = extract_tenant(&claims)?;
    service_rules::delete_elimination_rule(&state.pool, &tid, id)
        .await
        .map_err(config_error_response)?;
    Ok(StatusCode::NO_CONTENT)
}

// ============================================================================
// FX policies
// ============================================================================

pub async fn upsert_fx_policy(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(group_id): Path<Uuid>,
    Json(req): Json<UpsertFxPolicyRequest>,
) -> Result<Json<config::FxPolicy>, (StatusCode, Json<ErrorBody>)> {
    let tid = extract_tenant(&claims)?;
    let policy = service_rules::upsert_fx_policy(&state.pool, &tid, group_id, &req)
        .await
        .map_err(config_error_response)?;
    Ok(Json(policy))
}

pub async fn list_fx_policies(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(group_id): Path<Uuid>,
) -> Result<Json<Vec<config::FxPolicy>>, (StatusCode, Json<ErrorBody>)> {
    let tid = extract_tenant(&claims)?;
    let rows = service_rules::list_fx_policies(&state.pool, &tid, group_id)
        .await
        .map_err(config_error_response)?;
    Ok(Json(rows))
}

pub async fn delete_fx_policy(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, (StatusCode, Json<ErrorBody>)> {
    let tid = extract_tenant(&claims)?;
    service_rules::delete_fx_policy(&state.pool, &tid, id)
        .await
        .map_err(config_error_response)?;
    Ok(StatusCode::NO_CONTENT)
}

// ============================================================================
// Validation
// ============================================================================

pub async fn validate_group(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(group_id): Path<Uuid>,
) -> Result<Json<ValidationResult>, (StatusCode, Json<ErrorBody>)> {
    let tid = extract_tenant(&claims)?;
    let result = service::validate_group_completeness(&state.pool, &tid, group_id)
        .await
        .map_err(config_error_response)?;
    Ok(Json(result))
}
