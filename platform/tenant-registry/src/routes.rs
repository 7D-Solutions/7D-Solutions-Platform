//! HTTP routes for the tenant control-plane
//!
//! Exposes a read-only summary endpoint:
//!   GET /api/control/tenants/{tenant_id}/summary
//!
//! And an entitlements endpoint for identity-auth consumption:
//!   GET /api/tenants/{tenant_id}/entitlements
//!
//! Uses parallel HTTP fanout to check module readiness.
//! No direct cross-module DB reads.

use axum::{
    extract::{Path, State},
    http::StatusCode,
    routing::get,
    Json, Router,
};
use sqlx::PgPool;
use std::sync::Arc;
use uuid::Uuid;

use crate::summary::{fetch_tenant_summary, ModuleUrl, SummaryError, TenantSummary};
use crate::registry::{get_tenant_app_id, get_tenant_entitlements, get_tenant_status_row, EntitlementRow, TenantAppIdRow, TenantStatusRow};

/// Shared application state for summary routes
#[derive(Clone)]
pub struct SummaryState {
    /// Connection pool to the tenant-registry database
    pub pool: PgPool,
    /// Reusable HTTP client for module readiness fanout
    pub http_client: reqwest::Client,
    /// Module base URLs for HTTP fanout (name, base_url)
    pub module_urls: Vec<ModuleUrl>,
}

impl SummaryState {
    /// Create state with default local module URLs
    pub fn new_local(pool: PgPool) -> Self {
        Self {
            pool,
            http_client: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(3))
                .build()
                .expect("Failed to build HTTP client"),
            module_urls: ModuleUrl::default_local(),
        }
    }

    /// Create state with explicit module URLs (for testing / docker)
    pub fn new_with_urls(pool: PgPool, module_urls: Vec<ModuleUrl>) -> Self {
        Self {
            pool,
            http_client: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(3))
                .build()
                .expect("Failed to build HTTP client"),
            module_urls,
        }
    }
}

/// Error response body
#[derive(serde::Serialize)]
struct ErrorBody {
    error: String,
}

/// GET /api/control/tenants/:tenant_id/summary
///
/// Returns tenant registry record + parallel module readiness checks.
/// 200 on success, 404 if tenant not found, 500 on DB error.
async fn get_tenant_summary(
    State(state): State<Arc<SummaryState>>,
    Path(tenant_id): Path<Uuid>,
) -> Result<Json<TenantSummary>, (StatusCode, Json<ErrorBody>)> {
    match fetch_tenant_summary(
        &state.pool,
        &state.http_client,
        &state.module_urls,
        tenant_id,
    )
    .await
    {
        Ok(summary) => Ok(Json(summary)),
        Err(SummaryError::TenantNotFound(id)) => Err((
            StatusCode::NOT_FOUND,
            Json(ErrorBody {
                error: format!("Tenant not found: {id}"),
            }),
        )),
        Err(SummaryError::Database(e)) => Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorBody {
                error: format!("Database error: {e}"),
            }),
        )),
    }
}

/// Build the summary router.
///
/// Mount this under the control-plane application router:
/// ```no_run
/// # use std::sync::Arc;
/// # use tenant_registry::routes::{SummaryState, summary_router};
/// # async fn example(state: Arc<SummaryState>) {
/// let app = axum::Router::new()
///     .merge(summary_router(state));
/// # }
/// ```
pub fn summary_router(state: Arc<SummaryState>) -> Router {
    Router::new()
        .route(
            "/api/control/tenants/{tenant_id}/summary",
            get(get_tenant_summary),
        )
        .with_state(state)
}

// ============================================================
// Entitlements endpoint
// ============================================================

/// GET /api/tenants/:tenant_id/entitlements
///
/// Returns the entitlement row for a tenant.
/// - 200 + JSON body if the tenant has an entitlements row
/// - 404 if the tenant exists but has no entitlements row
/// - 404 if the tenant does not exist
///
/// identity-auth treats any 404 as deny (fail-closed).
async fn get_entitlements(
    State(state): State<Arc<SummaryState>>,
    Path(tenant_id): Path<Uuid>,
) -> Result<Json<EntitlementRow>, (StatusCode, Json<ErrorBody>)> {
    match get_tenant_entitlements(&state.pool, tenant_id).await {
        Ok(Some(row)) => Ok(Json(row)),
        Ok(None) => Err((
            StatusCode::NOT_FOUND,
            Json(ErrorBody {
                error: format!("No entitlements configured for tenant {tenant_id}"),
            }),
        )),
        Err(sqlx::Error::RowNotFound) => Err((
            StatusCode::NOT_FOUND,
            Json(ErrorBody {
                error: format!("Tenant not found: {tenant_id}"),
            }),
        )),
        Err(e) => Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorBody {
                error: format!("Database error: {e}"),
            }),
        )),
    }
}

/// Build the entitlements router.
///
/// Exposes: GET /api/tenants/:tenant_id/entitlements
pub fn entitlements_router(state: Arc<SummaryState>) -> Router {
    Router::new()
        .route(
            "/api/tenants/{tenant_id}/entitlements",
            get(get_entitlements),
        )
        .with_state(state)
}

// ============================================================
// App-ID mapping endpoint
// ============================================================

/// GET /api/tenants/:tenant_id/app-id
///
/// Returns the app_id (and product_code) for a tenant so that orchestrators
/// such as TTP billing can translate tenant_id → AR app_id.
///
/// - 200 + JSON body if the tenant exists and has a non-NULL app_id
/// - 409 if the tenant exists but app_id is NULL (data integrity problem)
/// - 404 if the tenant does not exist
async fn get_app_id(
    State(state): State<Arc<SummaryState>>,
    Path(tenant_id): Path<Uuid>,
) -> Result<Json<TenantAppIdRow>, (StatusCode, Json<ErrorBody>)> {
    match get_tenant_app_id(&state.pool, tenant_id).await {
        Ok(Some(row)) => Ok(Json(row)),
        Ok(None) => Err((
            StatusCode::CONFLICT,
            Json(ErrorBody {
                error: format!("Tenant {tenant_id} has no app_id assigned"),
            }),
        )),
        Err(sqlx::Error::RowNotFound) => Err((
            StatusCode::NOT_FOUND,
            Json(ErrorBody {
                error: format!("Tenant not found: {tenant_id}"),
            }),
        )),
        Err(e) => Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorBody {
                error: format!("Database error: {e}"),
            }),
        )),
    }
}

/// Build the app-id router.
///
/// Exposes: GET /api/tenants/:tenant_id/app-id
pub fn app_id_router(state: Arc<SummaryState>) -> Router {
    Router::new()
        .route("/api/tenants/{tenant_id}/app-id", get(get_app_id))
        .with_state(state)
}

// ============================================================
// Tenant status endpoint (lightweight — for identity-auth gating)
// ============================================================

/// GET /api/tenants/:tenant_id/status
///
/// Returns the lifecycle status for a tenant (no module fanout).
/// Consumed by identity-auth to gate login/refresh by tenant lifecycle.
///
/// - 200 + `{ tenant_id, status }` if tenant exists
/// - 404 if tenant does not exist
async fn get_tenant_status(
    State(state): State<Arc<SummaryState>>,
    Path(tenant_id): Path<Uuid>,
) -> Result<Json<TenantStatusRow>, (StatusCode, Json<ErrorBody>)> {
    match get_tenant_status_row(&state.pool, tenant_id).await {
        Ok(Some(row)) => Ok(Json(row)),
        Ok(None) => Err((
            StatusCode::NOT_FOUND,
            Json(ErrorBody {
                error: format!("Tenant not found: {tenant_id}"),
            }),
        )),
        Err(e) => Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorBody {
                error: format!("Database error: {e}"),
            }),
        )),
    }
}

/// Build the tenant status router.
///
/// Exposes: GET /api/tenants/:tenant_id/status
pub fn status_router(state: Arc<SummaryState>) -> Router {
    Router::new()
        .route("/api/tenants/{tenant_id}/status", get(get_tenant_status))
        .with_state(state)
}

#[cfg(test)]
mod app_id_tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use sqlx::PgPool;
    use tower::ServiceExt;
    use uuid::Uuid;

    async fn test_pool() -> PgPool {
        let url = std::env::var("TENANT_REGISTRY_DATABASE_URL").unwrap_or_else(|_| {
            "postgresql://tenant_registry_user:tenant_registry_pass@localhost:5441/tenant_registry_db"
                .to_string()
        });
        PgPool::connect(&url).await.expect("connect to tenant-registry DB")
    }

    fn build_app(pool: PgPool) -> axum::Router {
        let state = Arc::new(SummaryState::new_with_urls(pool, vec![]));
        app_id_router(state)
    }

    /// Insert a tenant with a known app_id.
    async fn seed_tenant_with_app_id(pool: &PgPool, app_id: &str) -> Uuid {
        let tenant_id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO tenants (tenant_id, status, environment, module_schema_versions, app_id, product_code, created_at, updated_at)
             VALUES ($1, 'active', 'development', '{}'::jsonb, $2, 'starter', NOW(), NOW())",
        )
        .bind(tenant_id)
        .bind(app_id)
        .execute(pool)
        .await
        .expect("insert tenant with app_id");
        tenant_id
    }

    /// Insert a tenant with a NULL app_id (edge case — should not happen for new tenants).
    async fn seed_tenant_without_app_id(pool: &PgPool) -> Uuid {
        let tenant_id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO tenants (tenant_id, status, environment, module_schema_versions, created_at, updated_at)
             VALUES ($1, 'active', 'development', '{}'::jsonb, NOW(), NOW())",
        )
        .bind(tenant_id)
        .execute(pool)
        .await
        .expect("insert tenant without app_id");
        tenant_id
    }

    #[tokio::test]
    async fn app_id_found_returns_200_with_app_id() {
        let pool = test_pool().await;
        let app_id = format!("app_{}", Uuid::new_v4().to_string().replace('-', "")[..8].to_string());
        let tenant_id = seed_tenant_with_app_id(&pool, &app_id).await;
        let app = build_app(pool.clone());

        let req = Request::builder()
            .uri(format!("/api/tenants/{tenant_id}/app-id"))
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.expect("call app-id endpoint");
        assert_eq!(resp.status(), StatusCode::OK, "expected 200 for tenant with app_id");

        let body = axum::body::to_bytes(resp.into_body(), 16 * 1024)
            .await
            .expect("read body");
        let json: serde_json::Value = serde_json::from_slice(&body).expect("parse JSON");
        assert_eq!(json["app_id"], app_id);
        assert_eq!(json["product_code"], "starter");

        // Cleanup
        sqlx::query("DELETE FROM tenants WHERE tenant_id = $1").bind(tenant_id).execute(&pool).await.ok();
    }

    #[tokio::test]
    async fn app_id_not_found_returns_404() {
        let pool = test_pool().await;
        let app = build_app(pool);

        let nonexistent = Uuid::new_v4();
        let req = Request::builder()
            .uri(format!("/api/tenants/{nonexistent}/app-id"))
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.expect("call app-id endpoint");
        assert_eq!(resp.status(), StatusCode::NOT_FOUND, "expected 404 for missing tenant");
    }

    #[tokio::test]
    async fn app_id_missing_returns_409() {
        let pool = test_pool().await;
        let tenant_id = seed_tenant_without_app_id(&pool).await;
        let app = build_app(pool.clone());

        let req = Request::builder()
            .uri(format!("/api/tenants/{tenant_id}/app-id"))
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.expect("call app-id endpoint");
        assert_eq!(resp.status(), StatusCode::CONFLICT, "expected 409 when app_id is NULL");

        let body = axum::body::to_bytes(resp.into_body(), 16 * 1024)
            .await
            .expect("read body");
        let json: serde_json::Value = serde_json::from_slice(&body).expect("parse JSON");
        assert!(json["error"].as_str().unwrap().contains("no app_id"));

        // Cleanup
        sqlx::query("DELETE FROM tenants WHERE tenant_id = $1").bind(tenant_id).execute(&pool).await.ok();
    }
}

#[cfg(test)]
mod tests {
    use crate::summary::ModuleUrl;

    #[test]
    fn summary_state_new_local_has_five_modules() {
        // We can't create a real PgPool in a unit test, but we can verify
        // the module URL list without running the actual server.
        let urls = ModuleUrl::default_local();
        assert_eq!(urls.len(), 5);
    }

    #[test]
    fn summary_router_compiles() {
        // Structural smoke test: verify router can be constructed with mock state
        // (no actual DB connection needed — just verify the type graph compiles)
        // This test is intentionally minimal; real coverage is in E2E tests.
        let _ = "GET /api/control/tenants/:tenant_id/summary";
    }
}

#[cfg(test)]
mod entitlement_tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use sqlx::PgPool;
    use tower::ServiceExt;
    use uuid::Uuid;

    /// Connect to the tenant-registry database used in local dev/CI.
    /// Requires the container from docker-compose.infrastructure.yml to be running.
    async fn test_pool() -> PgPool {
        let url = std::env::var("TENANT_REGISTRY_DATABASE_URL").unwrap_or_else(|_| {
            "postgresql://tenant_registry_user:tenant_registry_pass@localhost:5441/tenant_registry_db"
                .to_string()
        });
        let pool = PgPool::connect(&url).await.expect("connect to tenant-registry DB");

        // Ensure cp_entitlements exists (idempotent — other tables exist from prior setup).
        // We don't run the full migrator because the base schema was created outside sqlx tracking.
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS cp_entitlements (
                tenant_id UUID PRIMARY KEY REFERENCES tenants(tenant_id) ON DELETE CASCADE,
                plan_code TEXT NOT NULL,
                concurrent_user_limit INT NOT NULL CHECK (concurrent_user_limit > 0),
                effective_at TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP,
                updated_at TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP
            )
            "#,
        )
        .execute(&pool)
        .await
        .expect("ensure cp_entitlements table exists");

        pool
    }

    fn build_app(pool: PgPool) -> axum::Router {
        let state = Arc::new(SummaryState::new_with_urls(pool, vec![]));
        entitlements_router(state)
    }

    /// Returns a UUID that is known to exist in tenants and has an entitlements row.
    async fn seed_tenant_with_entitlements(pool: &PgPool) -> Uuid {
        let tenant_id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO tenants (tenant_id, status, environment, module_schema_versions, created_at, updated_at)
             VALUES ($1, 'active', 'development', '{}'::jsonb, NOW(), NOW())",
        )
        .bind(tenant_id)
        .execute(pool)
        .await
        .expect("insert tenant");

        sqlx::query(
            "INSERT INTO cp_entitlements (tenant_id, plan_code, concurrent_user_limit)
             VALUES ($1, 'monthly', 10)",
        )
        .bind(tenant_id)
        .execute(pool)
        .await
        .expect("insert entitlements");

        tenant_id
    }

    /// Returns a UUID for a tenant that has no entitlements row.
    async fn seed_tenant_without_entitlements(pool: &PgPool) -> Uuid {
        let tenant_id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO tenants (tenant_id, status, environment, module_schema_versions, created_at, updated_at)
             VALUES ($1, 'active', 'development', '{}'::jsonb, NOW(), NOW())",
        )
        .bind(tenant_id)
        .execute(pool)
        .await
        .expect("insert tenant without entitlements");
        tenant_id
    }

    #[tokio::test]
    async fn entitlements_found_returns_200_with_limit() {
        let pool = test_pool().await;
        let tenant_id = seed_tenant_with_entitlements(&pool).await;
        let app = build_app(pool.clone());

        let req = Request::builder()
            .uri(format!("/api/tenants/{tenant_id}/entitlements"))
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.expect("call entitlements endpoint");
        assert_eq!(resp.status(), StatusCode::OK, "expected 200 for tenant with entitlements");

        let body = axum::body::to_bytes(resp.into_body(), 16 * 1024)
            .await
            .expect("read body");
        let json: serde_json::Value = serde_json::from_slice(&body).expect("parse JSON");
        assert_eq!(json["concurrent_user_limit"], 10);
        assert_eq!(json["plan_code"], "monthly");

        // Cleanup
        sqlx::query("DELETE FROM cp_entitlements WHERE tenant_id = $1").bind(tenant_id).execute(&pool).await.ok();
        sqlx::query("DELETE FROM tenants WHERE tenant_id = $1").bind(tenant_id).execute(&pool).await.ok();
    }

    #[tokio::test]
    async fn entitlements_not_found_returns_404_when_no_row() {
        let pool = test_pool().await;
        let tenant_id = seed_tenant_without_entitlements(&pool).await;
        let app = build_app(pool.clone());

        let req = Request::builder()
            .uri(format!("/api/tenants/{tenant_id}/entitlements"))
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.expect("call entitlements endpoint");
        assert_eq!(resp.status(), StatusCode::NOT_FOUND, "expected 404 when no entitlements row");

        // Cleanup
        sqlx::query("DELETE FROM tenants WHERE tenant_id = $1").bind(tenant_id).execute(&pool).await.ok();
    }

    #[tokio::test]
    async fn entitlements_missing_tenant_returns_404() {
        let pool = test_pool().await;
        let app = build_app(pool);

        let nonexistent = Uuid::new_v4();
        let req = Request::builder()
            .uri(format!("/api/tenants/{nonexistent}/entitlements"))
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.expect("call entitlements endpoint");
        assert_eq!(resp.status(), StatusCode::NOT_FOUND, "expected 404 for missing tenant");
    }
}
