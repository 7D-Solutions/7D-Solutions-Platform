//! Tax configuration CRUD routes (bd-1m3c)
//!
//! Provides management endpoints for tax jurisdictions and rules backed by
//! the existing `ar_tax_jurisdictions` and `ar_tax_rules` tables.
//!
//! All endpoints are tenant-scoped via `app_id`.

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use uuid::Uuid;

// ============================================================================
// Request / Response types — Jurisdictions
// ============================================================================

#[derive(Debug, Deserialize)]
pub struct CreateJurisdictionRequest {
    pub app_id: String,
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
    pub app_id: String,
    pub country_code: Option<String>,
    pub state_code: Option<String>,
    pub is_active: Option<bool>,
}

// ============================================================================
// Request / Response types — Rules
// ============================================================================

#[derive(Debug, Deserialize)]
pub struct CreateRuleRequest {
    pub app_id: String,
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

#[derive(Debug, Deserialize)]
pub struct ListRulesQuery {
    pub app_id: String,
    pub jurisdiction_id: Option<Uuid>,
    pub effective_on: Option<NaiveDate>,
}

#[derive(Debug, Serialize)]
struct ErrorBody {
    error: String,
}

// ============================================================================
// Jurisdiction handlers
// ============================================================================

/// POST /api/ar/tax/config/jurisdictions
pub async fn create_jurisdiction(
    State(pool): State<PgPool>,
    Json(body): Json<CreateJurisdictionRequest>,
) -> impl IntoResponse {
    if body.app_id.is_empty() || body.country_code.is_empty() || body.jurisdiction_name.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(ErrorBody {
                error: "app_id, country_code, and jurisdiction_name are required".into(),
            }),
        )
            .into_response();
    }

    let tax_type = body.tax_type.as_deref().unwrap_or("sales_tax");

    let id = crate::tax::insert_jurisdiction(
        &pool,
        &body.app_id,
        &body.country_code,
        body.state_code.as_deref(),
        body.postal_pattern.as_deref(),
        &body.jurisdiction_name,
        tax_type,
    )
    .await;

    match id {
        Ok(id) => {
            // Fetch the full row to return
            match get_jurisdiction_by_id(&pool, id).await {
                Ok(Some(j)) => (StatusCode::CREATED, Json(j)).into_response(),
                Ok(None) => (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ErrorBody {
                        error: "Created but could not fetch jurisdiction".into(),
                    }),
                )
                    .into_response(),
                Err(e) => db_error(e),
            }
        }
        Err(e) => db_error(e),
    }
}

/// GET /api/ar/tax/config/jurisdictions
pub async fn list_jurisdictions(
    State(pool): State<PgPool>,
    Query(q): Query<ListJurisdictionsQuery>,
) -> impl IntoResponse {
    if q.app_id.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(ErrorBody {
                error: "app_id is required".into(),
            }),
        )
            .into_response();
    }

    let is_active = q.is_active.unwrap_or(true);

    let rows = sqlx::query_as::<_, (
        Uuid, String, String, Option<String>, Option<String>,
        String, String, bool,
        chrono::DateTime<chrono::Utc>, chrono::DateTime<chrono::Utc>,
    )>(
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
    .bind(&q.app_id)
    .bind(is_active)
    .bind(&q.country_code)
    .bind(&q.state_code)
    .fetch_all(&pool)
    .await;

    match rows {
        Ok(rows) => {
            let jurisdictions: Vec<JurisdictionResponse> = rows.into_iter().map(row_to_jurisdiction).collect();
            (StatusCode::OK, Json(jurisdictions)).into_response()
        }
        Err(e) => db_error(e),
    }
}

/// GET /api/ar/tax/config/jurisdictions/:id
pub async fn get_jurisdiction(
    State(pool): State<PgPool>,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    match get_jurisdiction_by_id(&pool, id).await {
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
    Path(id): Path<Uuid>,
    Json(body): Json<UpdateJurisdictionRequest>,
) -> impl IntoResponse {
    let result = sqlx::query(
        r#"
        UPDATE ar_tax_jurisdictions SET
            jurisdiction_name = COALESCE($2, jurisdiction_name),
            tax_type = COALESCE($3, tax_type),
            is_active = COALESCE($4, is_active),
            updated_at = NOW()
        WHERE id = $1
        "#,
    )
    .bind(id)
    .bind(&body.jurisdiction_name)
    .bind(&body.tax_type)
    .bind(body.is_active)
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
        Ok(_) => match get_jurisdiction_by_id(&pool, id).await {
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
// Rule handlers
// ============================================================================

/// POST /api/ar/tax/config/rules
pub async fn create_rule(
    State(pool): State<PgPool>,
    Json(body): Json<CreateRuleRequest>,
) -> impl IntoResponse {
    if body.app_id.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(ErrorBody {
                error: "app_id is required".into(),
            }),
        )
            .into_response();
    }

    if body.rate < 0.0 || body.rate > 1.0 {
        return (
            StatusCode::BAD_REQUEST,
            Json(ErrorBody {
                error: "rate must be between 0.0 and 1.0".into(),
            }),
        )
            .into_response();
    }

    // Validate effective_to > effective_from if set
    if let Some(to) = body.effective_to {
        if to <= body.effective_from {
            return (
                StatusCode::BAD_REQUEST,
                Json(ErrorBody {
                    error: "effective_to must be after effective_from".into(),
                }),
            )
                .into_response();
        }
    }

    // Verify jurisdiction exists and belongs to the same app_id
    let owner = sqlx::query_scalar::<_, String>(
        "SELECT app_id FROM ar_tax_jurisdictions WHERE id = $1",
    )
    .bind(body.jurisdiction_id)
    .fetch_optional(&pool)
    .await;

    match owner {
        Ok(None) => {
            return (
                StatusCode::NOT_FOUND,
                Json(ErrorBody {
                    error: "Jurisdiction not found".into(),
                }),
            )
                .into_response();
        }
        Ok(Some(owner_app)) if owner_app != body.app_id => {
            return (
                StatusCode::FORBIDDEN,
                Json(ErrorBody {
                    error: "Jurisdiction belongs to a different tenant".into(),
                }),
            )
                .into_response();
        }
        Err(e) => return db_error(e),
        _ => {}
    }

    let id = crate::tax::insert_tax_rule(
        &pool,
        body.jurisdiction_id,
        &body.app_id,
        body.tax_code.as_deref(),
        body.rate,
        body.flat_amount_minor.unwrap_or(0),
        body.is_exempt.unwrap_or(false),
        body.effective_from,
        body.effective_to,
        body.priority.unwrap_or(0),
    )
    .await;

    match id {
        Ok(id) => match get_rule_by_id(&pool, id).await {
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

/// GET /api/ar/tax/config/rules
pub async fn list_rules(
    State(pool): State<PgPool>,
    Query(q): Query<ListRulesQuery>,
) -> impl IntoResponse {
    if q.app_id.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(ErrorBody {
                error: "app_id is required".into(),
            }),
        )
            .into_response();
    }

    let rows = sqlx::query_as::<_, (
        Uuid, Uuid, String, Option<String>, f64,
        i64, bool, NaiveDate, Option<NaiveDate>, i32,
        chrono::DateTime<chrono::Utc>, chrono::DateTime<chrono::Utc>,
    )>(
        r#"
        SELECT id, jurisdiction_id, app_id, tax_code, rate::FLOAT8,
               flat_amount_minor, is_exempt, effective_from, effective_to, priority,
               created_at, updated_at
        FROM ar_tax_rules
        WHERE app_id = $1
          AND ($2::UUID IS NULL OR jurisdiction_id = $2)
          AND ($3::DATE IS NULL OR (effective_from <= $3 AND (effective_to IS NULL OR effective_to > $3)))
        ORDER BY jurisdiction_id, priority DESC, effective_from
        "#,
    )
    .bind(&q.app_id)
    .bind(q.jurisdiction_id)
    .bind(q.effective_on)
    .fetch_all(&pool)
    .await;

    match rows {
        Ok(rows) => {
            let rules: Vec<RuleResponse> = rows.into_iter().map(row_to_rule).collect();
            (StatusCode::OK, Json(rules)).into_response()
        }
        Err(e) => db_error(e),
    }
}

/// GET /api/ar/tax/config/rules/:id
pub async fn get_rule(
    State(pool): State<PgPool>,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    match get_rule_by_id(&pool, id).await {
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

/// PUT /api/ar/tax/config/rules/:id
pub async fn update_rule(
    State(pool): State<PgPool>,
    Path(id): Path<Uuid>,
    Json(body): Json<UpdateRuleRequest>,
) -> impl IntoResponse {
    // Validate rate if provided
    if let Some(rate) = body.rate {
        if rate < 0.0 || rate > 1.0 {
            return (
                StatusCode::BAD_REQUEST,
                Json(ErrorBody {
                    error: "rate must be between 0.0 and 1.0".into(),
                }),
            )
                .into_response();
        }
    }

    let result = sqlx::query(
        r#"
        UPDATE ar_tax_rules SET
            rate = COALESCE($2, rate),
            flat_amount_minor = COALESCE($3, flat_amount_minor),
            is_exempt = COALESCE($4, is_exempt),
            effective_to = COALESCE($5, effective_to),
            priority = COALESCE($6, priority),
            updated_at = NOW()
        WHERE id = $1
        "#,
    )
    .bind(id)
    .bind(body.rate)
    .bind(body.flat_amount_minor)
    .bind(body.is_exempt)
    .bind(body.effective_to)
    .bind(body.priority)
    .execute(&pool)
    .await;

    match result {
        Ok(r) if r.rows_affected() == 0 => (
            StatusCode::NOT_FOUND,
            Json(ErrorBody {
                error: "Rule not found".into(),
            }),
        )
            .into_response(),
        Ok(_) => match get_rule_by_id(&pool, id).await {
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

// ============================================================================
// Helpers
// ============================================================================

async fn get_jurisdiction_by_id(
    pool: &PgPool,
    id: Uuid,
) -> Result<Option<JurisdictionResponse>, sqlx::Error> {
    let row = sqlx::query_as::<_, (
        Uuid, String, String, Option<String>, Option<String>,
        String, String, bool,
        chrono::DateTime<chrono::Utc>, chrono::DateTime<chrono::Utc>,
    )>(
        r#"
        SELECT id, app_id, country_code, state_code, postal_pattern,
               jurisdiction_name, tax_type, is_active, created_at, updated_at
        FROM ar_tax_jurisdictions
        WHERE id = $1
        "#,
    )
    .bind(id)
    .fetch_optional(pool)
    .await?;

    Ok(row.map(row_to_jurisdiction))
}

async fn get_rule_by_id(
    pool: &PgPool,
    id: Uuid,
) -> Result<Option<RuleResponse>, sqlx::Error> {
    let row = sqlx::query_as::<_, (
        Uuid, Uuid, String, Option<String>, f64,
        i64, bool, NaiveDate, Option<NaiveDate>, i32,
        chrono::DateTime<chrono::Utc>, chrono::DateTime<chrono::Utc>,
    )>(
        r#"
        SELECT id, jurisdiction_id, app_id, tax_code, rate::FLOAT8,
               flat_amount_minor, is_exempt, effective_from, effective_to, priority,
               created_at, updated_at
        FROM ar_tax_rules
        WHERE id = $1
        "#,
    )
    .bind(id)
    .fetch_optional(pool)
    .await?;

    Ok(row.map(row_to_rule))
}

fn row_to_jurisdiction(
    r: (Uuid, String, String, Option<String>, Option<String>,
        String, String, bool,
        chrono::DateTime<chrono::Utc>, chrono::DateTime<chrono::Utc>),
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

fn row_to_rule(
    r: (Uuid, Uuid, String, Option<String>, f64,
        i64, bool, NaiveDate, Option<NaiveDate>, i32,
        chrono::DateTime<chrono::Utc>, chrono::DateTime<chrono::Utc>),
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

fn db_error(e: sqlx::Error) -> axum::response::Response {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(ErrorBody {
            error: format!("Database error: {}", e),
        }),
    )
        .into_response()
}
