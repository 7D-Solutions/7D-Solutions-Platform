//! Tax rule CRUD routes — effective-dated rules within jurisdictions (bd-1m3c)
//!
//! Provides management endpoints for tax rules backed by the existing
//! `ar_tax_rules` table.  Shared types (RuleResponse, db_error, etc.)
//! are imported from [`tax_config`].

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    Extension, Json,
};
use chrono::NaiveDate;
use security::VerifiedClaims;
use serde::Deserialize;
use sqlx::PgPool;
use uuid::Uuid;

use super::tax_config::{db_error, get_rule_by_id_and_tenant, ErrorBody};
use crate::domain::tax_config as tax_config_repo;

// ============================================================================
// Request / Response types — Rules
// ============================================================================

#[derive(Debug, Deserialize)]
pub struct CreateRuleRequest {
    pub jurisdiction_id: Uuid,
    pub tax_code: Option<String>,
    pub rate: f64,
    pub flat_amount_minor: Option<i64>,
    pub is_exempt: Option<bool>,
    pub effective_from: NaiveDate,
    pub effective_to: Option<NaiveDate>,
    pub priority: Option<i32>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateRuleRequest {
    pub rate: Option<f64>,
    pub flat_amount_minor: Option<i64>,
    pub is_exempt: Option<bool>,
    pub effective_to: Option<NaiveDate>,
    pub priority: Option<i32>,
}

#[derive(Debug, Deserialize)]
pub struct ListRulesQuery {
    pub jurisdiction_id: Option<Uuid>,
    /// If set, only return rules effective on this date.
    pub as_of: Option<NaiveDate>,
}

// ============================================================================
// Handlers
// ============================================================================

#[utoipa::path(post, path = "/api/ar/tax/config/rules", tag = "Tax Config",
    request_body = serde_json::Value,
    responses(
        (status = 201, description = "Rule created", body = serde_json::Value),
    ),
    security(("bearer" = [])))]
/// POST /api/ar/tax/config/rules
pub async fn create_rule(
    State(pool): State<PgPool>,
    claims: Option<Extension<VerifiedClaims>>,
    Json(body): Json<CreateRuleRequest>,
) -> impl IntoResponse {
    let app_id = match super::tenant::extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };

    let flat = body.flat_amount_minor.unwrap_or(0);
    let is_exempt = body.is_exempt.unwrap_or(false);
    let priority = body.priority.unwrap_or(0);

    let id = crate::tax::insert_tax_rule(
        &pool,
        body.jurisdiction_id,
        &app_id,
        body.tax_code.as_deref(),
        body.rate,
        flat,
        is_exempt,
        body.effective_from,
        body.effective_to,
        priority,
    )
    .await;

    match id {
        Ok(id) => match get_rule_by_id_and_tenant(&pool, id, &app_id).await {
            Ok(Some(r)) => (StatusCode::CREATED, Json(r)).into_response(),
            Ok(None) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorBody {
                    error: "Created but could not fetch rule".into(),
                }),
            )
                .into_response(),
            Err(e) => db_error(e),
        },
        Err(e) => db_error(e),
    }
}

#[utoipa::path(get, path = "/api/ar/tax/config/rules", tag = "Tax Config",
    params(
        ("jurisdiction_id" = Option<uuid::Uuid>, Query, description = "Filter by jurisdiction"),
        ("as_of" = Option<chrono::NaiveDate>, Query, description = "Filter rules effective on this date"),
    ),
    responses((status = 200, description = "List of tax rules", body = serde_json::Value)),
    security(("bearer" = [])))]
/// GET /api/ar/tax/config/rules
pub async fn list_rules(
    State(pool): State<PgPool>,
    claims: Option<Extension<VerifiedClaims>>,
    Query(q): Query<ListRulesQuery>,
) -> impl IntoResponse {
    let app_id = match super::tenant::extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };

    match tax_config_repo::list_rules(&pool, &app_id, q.jurisdiction_id, q.as_of).await {
        Ok(rules) => (StatusCode::OK, Json(rules)).into_response(),
        Err(e) => db_error(e),
    }
}

#[utoipa::path(get, path = "/api/ar/tax/config/rules/{id}", tag = "Tax Config",
    params(("id" = uuid::Uuid, Path, description = "Rule ID")),
    responses(
        (status = 200, description = "Rule found", body = serde_json::Value),
        (status = 404, description = "Not found", body = serde_json::Value),
    ),
    security(("bearer" = [])))]
/// GET /api/ar/tax/config/rules/:id
pub async fn get_rule(
    State(pool): State<PgPool>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    let app_id = match super::tenant::extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };

    match get_rule_by_id_and_tenant(&pool, id, &app_id).await {
        Ok(Some(r)) => (StatusCode::OK, Json(r)).into_response(),
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(ErrorBody {
                error: "Rule not found".into(),
            }),
        )
            .into_response(),
        Err(e) => db_error(e),
    }
}

#[utoipa::path(put, path = "/api/ar/tax/config/rules/{id}", tag = "Tax Config",
    params(("id" = uuid::Uuid, Path, description = "Rule ID")),
    request_body = serde_json::Value,
    responses(
        (status = 200, description = "Rule updated", body = serde_json::Value),
        (status = 404, description = "Not found", body = serde_json::Value),
    ),
    security(("bearer" = [])))]
/// PUT /api/ar/tax/config/rules/:id
pub async fn update_rule(
    State(pool): State<PgPool>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(id): Path<Uuid>,
    Json(body): Json<UpdateRuleRequest>,
) -> impl IntoResponse {
    let app_id = match super::tenant::extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };

    let result = tax_config_repo::update_rule(
        &pool,
        id,
        body.rate,
        body.flat_amount_minor,
        body.is_exempt,
        body.effective_to,
        body.priority,
        &app_id,
    )
    .await;

    match result {
        Ok(rows) if rows == 0 => (
            StatusCode::NOT_FOUND,
            Json(ErrorBody {
                error: "Rule not found".into(),
            }),
        )
            .into_response(),
        Ok(_) => match get_rule_by_id_and_tenant(&pool, id, &app_id).await {
            Ok(Some(r)) => (StatusCode::OK, Json(r)).into_response(),
            Ok(None) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorBody {
                    error: "Updated but could not fetch rule".into(),
                }),
            )
                .into_response(),
            Err(e) => db_error(e),
        },
        Err(e) => db_error(e),
    }
}

// end of module
