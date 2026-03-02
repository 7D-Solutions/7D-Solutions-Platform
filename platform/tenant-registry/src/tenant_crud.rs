//! Tenant list and detail read routes
//!
//! Exposes:
//!   GET /api/tenants            — paginated tenant list with optional filters
//!   GET /api/tenants/:tenant_id — tenant detail with derived name and seat_limit

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    routing::get,
    Json, Router,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use std::sync::Arc;
use uuid::Uuid;

/// Shared state for tenant CRUD routes
#[derive(Clone)]
pub struct TenantCrudState {
    pub pool: PgPool,
}

/// Error response body
#[derive(Serialize)]
struct ErrorBody {
    error: String,
}

fn db_err(e: sqlx::Error) -> (StatusCode, Json<ErrorBody>) {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(ErrorBody {
            error: format!("Database error: {e}"),
        }),
    )
}

/// Query parameters for GET /api/tenants
#[derive(Deserialize)]
pub struct TenantListQuery {
    pub search: Option<String>,
    pub status: Option<String>,
    /// Filter by plan_code
    pub plan: Option<String>,
    pub app_id: Option<String>,
    /// 1-based page number (default: 1)
    pub page: Option<i64>,
    /// Tenants per page (default: 25)
    pub page_size: Option<i64>,
}

/// Summary DTO for a tenant in the list response
#[derive(Serialize)]
pub struct TenantSummaryDto {
    pub id: String,
    pub name: String,
    pub status: String,
    pub plan: String,
    pub app_id: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

/// Response envelope for the tenant list
#[derive(Serialize)]
pub struct TenantListResponse {
    pub tenants: Vec<TenantSummaryDto>,
    pub total: i64,
    pub page: i64,
    pub page_size: i64,
}

/// Detail DTO for a single tenant
#[derive(Serialize)]
pub struct TenantDetailDto {
    pub id: String,
    pub name: String,
    pub status: String,
    pub plan: String,
    pub app_id: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub seat_limit: Option<i32>,
}

/// DB row for tenant list query
#[derive(sqlx::FromRow)]
struct TenantRow {
    tenant_id: Uuid,
    status: String,
    plan_code: Option<String>,
    app_id: Option<String>,
    product_code: Option<String>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

/// DB row for tenant detail (includes optional seat limit from cp_entitlements)
#[derive(sqlx::FromRow)]
struct TenantDetailRow {
    tenant_id: Uuid,
    status: String,
    plan_code: Option<String>,
    app_id: Option<String>,
    product_code: Option<String>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
    concurrent_user_limit: Option<i32>,
}

/// Derive a display name from available fields.
///
/// Priority: product_code (non-empty) > app_id (non-empty) > first 8 chars of tenant_id.
pub fn derive_name(product_code: Option<&str>, app_id: Option<&str>, tenant_id: Uuid) -> String {
    if let Some(pc) = product_code {
        if !pc.is_empty() {
            return pc.to_string();
        }
    }
    if let Some(ai) = app_id {
        if !ai.is_empty() {
            return ai.to_string();
        }
    }
    tenant_id.to_string()[..8].to_string()
}

impl From<TenantRow> for TenantSummaryDto {
    fn from(r: TenantRow) -> Self {
        let name = derive_name(r.product_code.as_deref(), r.app_id.as_deref(), r.tenant_id);
        TenantSummaryDto {
            id: r.tenant_id.to_string(),
            name,
            status: r.status,
            plan: r.plan_code.unwrap_or_else(|| "None".to_string()),
            app_id: r.app_id,
            created_at: r.created_at.to_rfc3339(),
            updated_at: r.updated_at.to_rfc3339(),
        }
    }
}

impl From<TenantDetailRow> for TenantDetailDto {
    fn from(r: TenantDetailRow) -> Self {
        let name = derive_name(r.product_code.as_deref(), r.app_id.as_deref(), r.tenant_id);
        TenantDetailDto {
            id: r.tenant_id.to_string(),
            name,
            status: r.status,
            plan: r.plan_code.unwrap_or_else(|| "None".to_string()),
            app_id: r.app_id,
            created_at: r.created_at.to_rfc3339(),
            updated_at: r.updated_at.to_rfc3339(),
            seat_limit: r.concurrent_user_limit,
        }
    }
}

/// GET /api/tenants
///
/// Returns a paginated list of non-deleted tenants with optional filters.
async fn list_tenants(
    State(state): State<Arc<TenantCrudState>>,
    Query(params): Query<TenantListQuery>,
) -> Result<Json<TenantListResponse>, (StatusCode, Json<ErrorBody>)> {
    let page = params.page.unwrap_or(1).max(1);
    let page_size = params.page_size.unwrap_or(25).clamp(1, 100);
    let offset = (page - 1) * page_size;

    // Build ILIKE pattern once for search
    let search_pattern = params.search.as_deref().map(|s| format!("%{s}%"));

    let rows: Vec<TenantRow> = sqlx::query_as(
        r#"
        SELECT tenant_id, status, plan_code, app_id, product_code, created_at, updated_at
        FROM tenants
        WHERE deleted_at IS NULL
          AND ($1::text IS NULL OR status = $1)
          AND ($2::text IS NULL OR plan_code = $2)
          AND ($3::text IS NULL OR app_id = $3)
          AND ($4::text IS NULL
               OR product_code ILIKE $4
               OR app_id ILIKE $4
               OR tenant_id::text ILIKE $4)
        ORDER BY created_at DESC
        LIMIT $5 OFFSET $6
        "#,
    )
    .bind(&params.status)
    .bind(&params.plan)
    .bind(&params.app_id)
    .bind(&search_pattern)
    .bind(page_size)
    .bind(offset)
    .fetch_all(&state.pool)
    .await
    .map_err(db_err)?;

    let total: i64 = sqlx::query_scalar(
        r#"
        SELECT COUNT(*)
        FROM tenants
        WHERE deleted_at IS NULL
          AND ($1::text IS NULL OR status = $1)
          AND ($2::text IS NULL OR plan_code = $2)
          AND ($3::text IS NULL OR app_id = $3)
          AND ($4::text IS NULL
               OR product_code ILIKE $4
               OR app_id ILIKE $4
               OR tenant_id::text ILIKE $4)
        "#,
    )
    .bind(&params.status)
    .bind(&params.plan)
    .bind(&params.app_id)
    .bind(&search_pattern)
    .fetch_one(&state.pool)
    .await
    .map_err(db_err)?;

    Ok(Json(TenantListResponse {
        tenants: rows.into_iter().map(Into::into).collect(),
        total,
        page,
        page_size,
    }))
}

/// GET /api/tenants/:tenant_id
///
/// Returns tenant detail including seat_limit from cp_entitlements (if present).
async fn get_tenant_detail(
    State(state): State<Arc<TenantCrudState>>,
    Path(tenant_id): Path<Uuid>,
) -> Result<Json<TenantDetailDto>, (StatusCode, Json<ErrorBody>)> {
    let row: Option<TenantDetailRow> = sqlx::query_as(
        r#"
        SELECT t.tenant_id, t.status, t.plan_code, t.app_id, t.product_code,
               t.created_at, t.updated_at,
               e.concurrent_user_limit
        FROM tenants t
        LEFT JOIN cp_entitlements e ON e.tenant_id = t.tenant_id
        WHERE t.tenant_id = $1 AND t.deleted_at IS NULL
        "#,
    )
    .bind(tenant_id)
    .fetch_optional(&state.pool)
    .await
    .map_err(db_err)?;

    match row {
        Some(r) => Ok(Json(r.into())),
        None => Err((
            StatusCode::NOT_FOUND,
            Json(ErrorBody {
                error: format!("Tenant not found: {tenant_id}"),
            }),
        )),
    }
}

/// Build the tenant list router.
///
/// Exposes: GET /api/tenants
pub fn tenant_list_router(pool: PgPool) -> Router {
    let state = Arc::new(TenantCrudState { pool });
    Router::new()
        .route("/api/tenants", get(list_tenants))
        .with_state(state)
}

/// Build the tenant detail router.
///
/// Exposes: GET /api/tenants/{tenant_id}
pub fn tenant_detail_router(pool: PgPool) -> Router {
    let state = Arc::new(TenantCrudState { pool });
    Router::new()
        .route("/api/tenants/{tenant_id}", get(get_tenant_detail))
        .with_state(state)
}

#[cfg(test)]
mod derive_name_tests {
    use super::*;

    #[test]
    fn product_code_wins_over_app_id() {
        let id = Uuid::new_v4();
        assert_eq!(derive_name(Some("acme"), Some("app_abc"), id), "acme");
    }

    #[test]
    fn app_id_wins_when_product_code_empty() {
        let id = Uuid::new_v4();
        assert_eq!(derive_name(Some(""), Some("app_abc"), id), "app_abc");
    }

    #[test]
    fn app_id_wins_when_product_code_none() {
        let id = Uuid::new_v4();
        assert_eq!(derive_name(None, Some("app_abc"), id), "app_abc");
    }

    #[test]
    fn tenant_id_prefix_fallback() {
        let id = Uuid::parse_str("550e8400-e29b-41d4-a716-446655440000").unwrap();
        let name = derive_name(None, None, id);
        assert_eq!(name, "550e8400");
        assert_eq!(name.len(), 8);
    }

    #[test]
    fn tenant_id_prefix_fallback_when_both_empty() {
        let id = Uuid::parse_str("550e8400-e29b-41d4-a716-446655440000").unwrap();
        assert_eq!(derive_name(Some(""), Some(""), id), "550e8400");
    }
}

#[cfg(test)]
mod tenant_list_tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt;

    async fn test_pool() -> PgPool {
        let url = std::env::var("TENANT_REGISTRY_DATABASE_URL").unwrap_or_else(|_| {
            "postgresql://tenant_registry_user:tenant_registry_pass@localhost:5441/tenant_registry_db"
                .to_string()
        });
        PgPool::connect(&url)
            .await
            .expect("connect to tenant-registry DB")
    }

    fn build_list_app(pool: PgPool) -> axum::Router {
        tenant_list_router(pool)
    }

    fn build_detail_app(pool: PgPool) -> axum::Router {
        tenant_detail_router(pool)
    }

    async fn seed_tenant(pool: &PgPool, product_code: &str, app_id: Option<&str>) -> Uuid {
        let tenant_id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO tenants (tenant_id, status, environment, module_schema_versions, product_code, app_id, created_at, updated_at)
             VALUES ($1, 'active', 'development', '{}'::jsonb, $2, $3, NOW(), NOW())",
        )
        .bind(tenant_id)
        .bind(product_code)
        .bind(app_id)
        .execute(pool)
        .await
        .expect("insert tenant");
        tenant_id
    }

    async fn cleanup(pool: &PgPool, tenant_id: Uuid) {
        sqlx::query("DELETE FROM cp_entitlements WHERE tenant_id = $1")
            .bind(tenant_id)
            .execute(pool)
            .await
            .ok();
        sqlx::query("DELETE FROM tenants WHERE tenant_id = $1")
            .bind(tenant_id)
            .execute(pool)
            .await
            .ok();
    }

    #[tokio::test]
    async fn list_returns_200_with_pagination_fields() {
        let pool = test_pool().await;
        let app = build_list_app(pool);

        let req = Request::builder()
            .uri("/api/tenants?page=1&page_size=10")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.expect("call list endpoint");
        assert_eq!(resp.status(), StatusCode::OK);

        let body = axum::body::to_bytes(resp.into_body(), 64 * 1024)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();

        assert!(json["tenants"].is_array(), "tenants must be array");
        assert!(json["total"].is_number(), "total must be present");
        assert_eq!(json["page"], 1);
        assert_eq!(json["page_size"], 10);
    }

    #[tokio::test]
    async fn list_returns_seeded_tenant() {
        let pool = test_pool().await;
        let tenant_id = seed_tenant(&pool, "acme-corp", Some("app_test123")).await;
        let app = build_list_app(pool.clone());

        let req = Request::builder()
            .uri("/api/tenants?page=1&page_size=100")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.expect("call list endpoint");
        assert_eq!(resp.status(), StatusCode::OK);

        let body = axum::body::to_bytes(resp.into_body(), 64 * 1024)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();

        let tenants = json["tenants"].as_array().unwrap();
        let found = tenants
            .iter()
            .any(|t| t["id"].as_str() == Some(&tenant_id.to_string()));
        assert!(found, "seeded tenant should appear in list");

        cleanup(&pool, tenant_id).await;
    }

    #[tokio::test]
    async fn list_tenant_name_uses_product_code() {
        let pool = test_pool().await;
        let tenant_id = seed_tenant(&pool, "my-product", Some("app_xyz")).await;
        let app = build_list_app(pool.clone());

        let req = Request::builder()
            .uri(format!("/api/tenants?search={tenant_id}"))
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.expect("call list endpoint");
        let body = axum::body::to_bytes(resp.into_body(), 64 * 1024)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();

        let tenants = json["tenants"].as_array().unwrap();
        let t = tenants
            .iter()
            .find(|t| t["id"].as_str() == Some(&tenant_id.to_string()));
        assert!(
            t.is_some(),
            "seeded tenant should be searchable by tenant_id"
        );
        assert_eq!(t.unwrap()["name"], "my-product");

        cleanup(&pool, tenant_id).await;
    }

    #[tokio::test]
    async fn detail_returns_tenant_with_derived_name() {
        let pool = test_pool().await;
        let tenant_id = seed_tenant(&pool, "test-company", Some("app_detail")).await;
        let app = build_detail_app(pool.clone());

        let req = Request::builder()
            .uri(format!("/api/tenants/{tenant_id}"))
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.expect("call detail endpoint");
        assert_eq!(resp.status(), StatusCode::OK);

        let body = axum::body::to_bytes(resp.into_body(), 64 * 1024)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();

        assert_eq!(json["id"], tenant_id.to_string());
        assert_eq!(json["name"], "test-company");
        assert_eq!(json["status"], "active");
        assert!(json["created_at"].is_string());
        assert!(json["updated_at"].is_string());

        cleanup(&pool, tenant_id).await;
    }

    #[tokio::test]
    async fn detail_returns_404_for_missing_tenant() {
        let pool = test_pool().await;
        let app = build_detail_app(pool);

        let nonexistent = Uuid::new_v4();
        let req = Request::builder()
            .uri(format!("/api/tenants/{nonexistent}"))
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.expect("call detail endpoint");
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn detail_includes_seat_limit_from_entitlements() {
        let pool = test_pool().await;
        let tenant_id = seed_tenant(&pool, "seat-test", None).await;

        sqlx::query(
            r#"CREATE TABLE IF NOT EXISTS cp_entitlements (
                tenant_id UUID PRIMARY KEY REFERENCES tenants(tenant_id) ON DELETE CASCADE,
                plan_code TEXT NOT NULL,
                concurrent_user_limit INT NOT NULL CHECK (concurrent_user_limit > 0),
                effective_at TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP,
                updated_at TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP
            )"#,
        )
        .execute(&pool)
        .await
        .expect("ensure cp_entitlements");

        sqlx::query(
            "INSERT INTO cp_entitlements (tenant_id, plan_code, concurrent_user_limit) VALUES ($1, 'starter', 20)",
        )
        .bind(tenant_id)
        .execute(&pool)
        .await
        .expect("insert entitlements");

        let app = build_detail_app(pool.clone());

        let req = Request::builder()
            .uri(format!("/api/tenants/{tenant_id}"))
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.expect("call detail endpoint");
        assert_eq!(resp.status(), StatusCode::OK);

        let body = axum::body::to_bytes(resp.into_body(), 64 * 1024)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();

        assert_eq!(json["seat_limit"], 20);

        cleanup(&pool, tenant_id).await;
    }
}
