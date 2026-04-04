//! Tax configuration CRUD routes — jurisdictions (bd-1m3c)
//!
//! Provides management endpoints for tax jurisdictions backed by
//! the existing `ar_tax_jurisdictions` table.
//!
//! Rule endpoints live in [`tax_config_rules`].

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    Extension, Json,
};
use security::VerifiedClaims;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use uuid::Uuid;

use crate::domain::tax_config as tax_config_repo;

// Re-export domain types for use by handlers and other modules.
pub use crate::domain::tax_config::{JurisdictionResponse, RuleResponse, row_to_jurisdiction, row_to_rule};

// ============================================================================
// Request / Response types — Jurisdictions
// ============================================================================

#[derive(Debug, Deserialize)]
pub struct CreateJurisdictionRequest {
    pub country_code: String,
    pub state_code: Option<String>,
    pub postal_pattern: Option<String>,
    pub jurisdiction_name: String,
    pub tax_type: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateJurisdictionRequest {
    pub jurisdiction_name: Option<String>,
    pub tax_type: Option<String>,
    pub is_active: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct ListJurisdictionsQuery {
    pub country_code: Option<String>,
    pub state_code: Option<String>,
    pub is_active: Option<bool>,
}

#[derive(Debug, Serialize)]
pub(crate) struct ErrorBody {
    pub error: String,
}

// ============================================================================
// Jurisdiction handlers
// ============================================================================

#[utoipa::path(post, path = "/api/ar/tax/config/jurisdictions", tag = "Tax Config",
    request_body = serde_json::Value,
    responses(
        (status = 201, description = "Jurisdiction created", body = serde_json::Value),
        (status = 400, description = "Validation error", body = serde_json::Value),
    ),
    security(("bearer" = [])))]
/// POST /api/ar/tax/config/jurisdictions
pub async fn create_jurisdiction(
    State(pool): State<PgPool>,
    claims: Option<Extension<VerifiedClaims>>,
    Json(body): Json<CreateJurisdictionRequest>,
) -> impl IntoResponse {
    let app_id = match super::tenant::extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };

    if body.country_code.is_empty() || body.jurisdiction_name.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(ErrorBody {
                error: "country_code and jurisdiction_name are required".into(),
            }),
        )
            .into_response();
    }

    let tax_type = body.tax_type.as_deref().unwrap_or("sales_tax");

    let id = crate::tax::insert_jurisdiction(
        &pool,
        &app_id,
        &body.country_code,
        body.state_code.as_deref(),
        body.postal_pattern.as_deref(),
        &body.jurisdiction_name,
        tax_type,
    )
    .await;

    match id {
        Ok(id) => match get_jurisdiction_by_id_and_tenant(&pool, id, &app_id).await {
            Ok(Some(j)) => (StatusCode::CREATED, Json(j)).into_response(),
            Ok(None) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorBody {
                    error: "Created but could not fetch jurisdiction".into(),
                }),
            )
                .into_response(),
            Err(e) => db_error(e),
        },
        Err(e) => db_error(e),
    }
}

#[utoipa::path(get, path = "/api/ar/tax/config/jurisdictions", tag = "Tax Config",
    params(
        ("country_code" = Option<String>, Query, description = "Filter by country"),
        ("state_code" = Option<String>, Query, description = "Filter by state"),
        ("is_active" = Option<bool>, Query, description = "Filter by active status"),
    ),
    responses((status = 200, description = "List of jurisdictions", body = serde_json::Value)),
    security(("bearer" = [])))]
/// GET /api/ar/tax/config/jurisdictions
pub async fn list_jurisdictions(
    State(pool): State<PgPool>,
    claims: Option<Extension<VerifiedClaims>>,
    Query(q): Query<ListJurisdictionsQuery>,
) -> impl IntoResponse {
    let app_id = match super::tenant::extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };

    let is_active = q.is_active.unwrap_or(true);

    match tax_config_repo::list_jurisdictions(
        &pool,
        &app_id,
        is_active,
        q.country_code.as_deref(),
        q.state_code.as_deref(),
    )
    .await
    {
        Ok(jurisdictions) => (StatusCode::OK, Json(jurisdictions)).into_response(),
        Err(e) => db_error(e),
    }
}

#[utoipa::path(get, path = "/api/ar/tax/config/jurisdictions/{id}", tag = "Tax Config",
    params(("id" = uuid::Uuid, Path, description = "Jurisdiction ID")),
    responses(
        (status = 200, description = "Jurisdiction found", body = serde_json::Value),
        (status = 404, description = "Not found", body = serde_json::Value),
    ),
    security(("bearer" = [])))]
/// GET /api/ar/tax/config/jurisdictions/:id
pub async fn get_jurisdiction(
    State(pool): State<PgPool>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    let app_id = match super::tenant::extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };

    match tax_config_repo::get_jurisdiction_by_id_and_tenant(&pool, id, &app_id).await {
        Ok(Some(j)) => (StatusCode::OK, Json(j)).into_response(),
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(ErrorBody {
                error: "Jurisdiction not found".into(),
            }),
        )
            .into_response(),
        Err(e) => db_error(e),
    }
}

#[utoipa::path(put, path = "/api/ar/tax/config/jurisdictions/{id}", tag = "Tax Config",
    params(("id" = uuid::Uuid, Path, description = "Jurisdiction ID")),
    request_body = serde_json::Value,
    responses(
        (status = 200, description = "Jurisdiction updated", body = serde_json::Value),
        (status = 404, description = "Not found", body = serde_json::Value),
    ),
    security(("bearer" = [])))]
/// PUT /api/ar/tax/config/jurisdictions/:id
pub async fn update_jurisdiction(
    State(pool): State<PgPool>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(id): Path<Uuid>,
    Json(body): Json<UpdateJurisdictionRequest>,
) -> impl IntoResponse {
    let app_id = match super::tenant::extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };

    let result = tax_config_repo::update_jurisdiction(
        &pool,
        id,
        body.jurisdiction_name.as_deref(),
        body.tax_type.as_deref(),
        body.is_active,
        &app_id,
    )
    .await;

    match result {
        Ok(rows) if rows == 0 => (
            StatusCode::NOT_FOUND,
            Json(ErrorBody {
                error: "Jurisdiction not found".into(),
            }),
        )
            .into_response(),
        Ok(_) => match tax_config_repo::get_jurisdiction_by_id_and_tenant(&pool, id, &app_id).await {
            Ok(Some(j)) => (StatusCode::OK, Json(j)).into_response(),
            Ok(None) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorBody {
                    error: "Updated but could not fetch jurisdiction".into(),
                }),
            )
                .into_response(),
            Err(e) => db_error(e),
        },
        Err(e) => db_error(e),
    }
}

// ============================================================================
// Shared helpers
// ============================================================================

pub(crate) async fn get_jurisdiction_by_id_and_tenant(
    pool: &PgPool,
    id: Uuid,
    app_id: &str,
) -> Result<Option<JurisdictionResponse>, sqlx::Error> {
    tax_config_repo::get_jurisdiction_by_id_and_tenant(pool, id, app_id).await
}

pub(crate) async fn get_rule_by_id_and_tenant(
    pool: &PgPool,
    id: Uuid,
    app_id: &str,
) -> Result<Option<RuleResponse>, sqlx::Error> {
    tax_config_repo::get_rule_by_id_and_tenant(pool, id, app_id).await
}

pub(crate) fn db_error(e: sqlx::Error) -> axum::response::Response {
    tracing::error!("Database error: {}", e);
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(ErrorBody {
            error: "Internal database error".to_string(),
        }),
    )
        .into_response()
}
