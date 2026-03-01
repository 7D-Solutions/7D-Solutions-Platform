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
use chrono::NaiveDate;
use security::VerifiedClaims;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use uuid::Uuid;

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

#[derive(Debug, Serialize)]
pub struct JurisdictionResponse {
    pub id: Uuid,
    pub app_id: String,
    pub country_code: String,
    pub state_code: Option<String>,
    pub postal_pattern: Option<String>,
    pub jurisdiction_name: String,
    pub tax_type: String,
    pub is_active: bool,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Deserialize)]
pub struct ListJurisdictionsQuery {
    pub country_code: Option<String>,
    pub state_code: Option<String>,
    pub is_active: Option<bool>,
}

// ============================================================================
// Shared types (used by tax_config_rules too)
// ============================================================================

#[derive(Debug, Serialize)]
pub struct RuleResponse {
    pub id: Uuid,
    pub jurisdiction_id: Uuid,
    pub app_id: String,
    pub tax_code: Option<String>,
    pub rate: f64,
    pub flat_amount_minor: i64,
    pub is_exempt: bool,
    pub effective_from: NaiveDate,
    pub effective_to: Option<NaiveDate>,
    pub priority: i32,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Serialize)]
pub(crate) struct ErrorBody {
    pub error: String,
}

// ============================================================================
// Jurisdiction handlers
// ============================================================================

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

    let rows = sqlx::query_as::<
        _,
        (
            Uuid,
            String,
            String,
            Option<String>,
            Option<String>,
            String,
            String,
            bool,
            chrono::DateTime<chrono::Utc>,
            chrono::DateTime<chrono::Utc>,
        ),
    >(
        r#"
        SELECT id, app_id, country_code, state_code, postal_pattern,
               jurisdiction_name, tax_type, is_active, created_at, updated_at
        FROM ar_tax_jurisdictions
        WHERE app_id = $1
          AND is_active = $2
          AND ($3::VARCHAR IS NULL OR country_code = $3)
          AND ($4::VARCHAR IS NULL OR state_code = $4)
        ORDER BY country_code, state_code, postal_pattern
        "#,
    )
    .bind(&app_id)
    .bind(is_active)
    .bind(&q.country_code)
    .bind(&q.state_code)
    .fetch_all(&pool)
    .await;

    match rows {
        Ok(rows) => {
            let jurisdictions: Vec<JurisdictionResponse> =
                rows.into_iter().map(row_to_jurisdiction).collect();
            (StatusCode::OK, Json(jurisdictions)).into_response()
        }
        Err(e) => db_error(e),
    }
}

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

    match get_jurisdiction_by_id_and_tenant(&pool, id, &app_id).await {
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

    let result = sqlx::query(
        r#"
        UPDATE ar_tax_jurisdictions SET
            jurisdiction_name = COALESCE($2, jurisdiction_name),
            tax_type = COALESCE($3, tax_type),
            is_active = COALESCE($4, is_active),
            updated_at = NOW()
        WHERE id = $1 AND app_id = $5
        "#,
    )
    .bind(id)
    .bind(&body.jurisdiction_name)
    .bind(&body.tax_type)
    .bind(body.is_active)
    .bind(&app_id)
    .execute(&pool)
    .await;

    match result {
        Ok(r) if r.rows_affected() == 0 => (
            StatusCode::NOT_FOUND,
            Json(ErrorBody {
                error: "Jurisdiction not found".into(),
            }),
        )
            .into_response(),
        Ok(_) => match get_jurisdiction_by_id_and_tenant(&pool, id, &app_id).await {
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
    let row = sqlx::query_as::<
        _,
        (
            Uuid,
            String,
            String,
            Option<String>,
            Option<String>,
            String,
            String,
            bool,
            chrono::DateTime<chrono::Utc>,
            chrono::DateTime<chrono::Utc>,
        ),
    >(
        r#"
        SELECT id, app_id, country_code, state_code, postal_pattern,
               jurisdiction_name, tax_type, is_active, created_at, updated_at
        FROM ar_tax_jurisdictions
        WHERE id = $1 AND app_id = $2
        "#,
    )
    .bind(id)
    .bind(app_id)
    .fetch_optional(pool)
    .await?;

    Ok(row.map(row_to_jurisdiction))
}

pub(crate) async fn get_rule_by_id_and_tenant(
    pool: &PgPool,
    id: Uuid,
    app_id: &str,
) -> Result<Option<RuleResponse>, sqlx::Error> {
    let row = sqlx::query_as::<
        _,
        (
            Uuid,
            Uuid,
            String,
            Option<String>,
            f64,
            i64,
            bool,
            NaiveDate,
            Option<NaiveDate>,
            i32,
            chrono::DateTime<chrono::Utc>,
            chrono::DateTime<chrono::Utc>,
        ),
    >(
        r#"
        SELECT id, jurisdiction_id, app_id, tax_code, rate::FLOAT8,
               flat_amount_minor, is_exempt, effective_from, effective_to, priority,
               created_at, updated_at
        FROM ar_tax_rules
        WHERE id = $1 AND app_id = $2
        "#,
    )
    .bind(id)
    .bind(app_id)
    .fetch_optional(pool)
    .await?;

    Ok(row.map(row_to_rule))
}

fn row_to_jurisdiction(
    r: (
        Uuid,
        String,
        String,
        Option<String>,
        Option<String>,
        String,
        String,
        bool,
        chrono::DateTime<chrono::Utc>,
        chrono::DateTime<chrono::Utc>,
    ),
) -> JurisdictionResponse {
    JurisdictionResponse {
        id: r.0,
        app_id: r.1,
        country_code: r.2,
        state_code: r.3,
        postal_pattern: r.4,
        jurisdiction_name: r.5,
        tax_type: r.6,
        is_active: r.7,
        created_at: r.8,
        updated_at: r.9,
    }
}

pub(crate) fn row_to_rule(
    r: (
        Uuid,
        Uuid,
        String,
        Option<String>,
        f64,
        i64,
        bool,
        NaiveDate,
        Option<NaiveDate>,
        i32,
        chrono::DateTime<chrono::Utc>,
        chrono::DateTime<chrono::Utc>,
    ),
) -> RuleResponse {
    RuleResponse {
        id: r.0,
        jurisdiction_id: r.1,
        app_id: r.2,
        tax_code: r.3,
        rate: r.4,
        flat_amount_minor: r.5,
        is_exempt: r.6,
        effective_from: r.7,
        effective_to: r.8,
        priority: r.9,
        created_at: r.10,
        updated_at: r.11,
    }
}

pub(crate) fn db_error(e: sqlx::Error) -> axum::response::Response {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(ErrorBody {
            error: format!("Database error: {}", e),
        }),
    )
        .into_response()
}
