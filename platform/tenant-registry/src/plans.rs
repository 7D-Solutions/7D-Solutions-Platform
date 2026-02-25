//! Plan catalog route
//!
//! Exposes:
//!   GET /api/ttp/plans  — list platform billing plans from cp_plans

use axum::{
    extract::{Query, State},
    http::StatusCode,
    routing::get,
    Json, Router,
};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use std::sync::Arc;

/// Shared state for the plan catalog router
#[derive(Clone)]
pub struct PlansState {
    pub pool: PgPool,
}

/// Query parameters for GET /api/ttp/plans
#[derive(Deserialize)]
pub struct PlansQuery {
    /// Optional filter by plan status (e.g. "active", "archived")
    pub status: Option<String>,
    /// 1-based page number (default: 1)
    pub page: Option<i64>,
    /// Number of plans per page (default: 25)
    pub page_size: Option<i64>,
}

/// A single plan as returned to the BFF
#[derive(Serialize)]
pub struct PlanSummary {
    pub id: String,
    pub name: String,
    pub pricing_model: String,
    pub included_seats: i32,
    pub metered_dimensions: Vec<serde_json::Value>,
    pub status: String,
}

/// Response envelope for the plan list
#[derive(Serialize)]
pub struct PlanListResponse {
    pub plans: Vec<PlanSummary>,
    pub total: i64,
    pub page: i64,
    pub page_size: i64,
}

/// Error response body
#[derive(Serialize)]
struct ErrorBody {
    error: String,
}

/// Row returned from cp_plans
#[derive(sqlx::FromRow)]
struct PlanRow {
    plan_code: String,
    name: String,
    pricing_model: Option<String>,
    included_seats: Option<i32>,
    status: Option<String>,
}

fn db_err(e: sqlx::Error) -> (StatusCode, Json<ErrorBody>) {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(ErrorBody { error: format!("Database error: {e}") }),
    )
}

/// GET /api/ttp/plans
///
/// Returns paginated list of platform billing plans.
/// Query params:
///   - status: optional filter (default: returns all)
///   - page: 1-based page number (default: 1)
///   - page_size: plans per page (default: 25)
async fn list_plans(
    State(state): State<Arc<PlansState>>,
    Query(params): Query<PlansQuery>,
) -> Result<Json<PlanListResponse>, (StatusCode, Json<ErrorBody>)> {
    let page = params.page.unwrap_or(1).max(1);
    let page_size = params.page_size.unwrap_or(25).clamp(1, 100);
    let offset = (page - 1) * page_size;

    let (rows, total): (Vec<PlanRow>, i64) = if let Some(ref status_filter) = params.status {
        let rows: Vec<PlanRow> = sqlx::query_as(
            "SELECT plan_code, name, pricing_model, included_seats, status \
             FROM cp_plans WHERE status = $1 ORDER BY plan_code LIMIT $2 OFFSET $3",
        )
        .bind(status_filter)
        .bind(page_size)
        .bind(offset)
        .fetch_all(&state.pool)
        .await
        .map_err(db_err)?;

        let total: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM cp_plans WHERE status = $1",
        )
        .bind(status_filter)
        .fetch_one(&state.pool)
        .await
        .map_err(db_err)?;

        (rows, total)
    } else {
        let rows: Vec<PlanRow> = sqlx::query_as(
            "SELECT plan_code, name, pricing_model, included_seats, status \
             FROM cp_plans ORDER BY plan_code LIMIT $1 OFFSET $2",
        )
        .bind(page_size)
        .bind(offset)
        .fetch_all(&state.pool)
        .await
        .map_err(db_err)?;

        let total: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM cp_plans")
            .fetch_one(&state.pool)
            .await
            .map_err(db_err)?;

        (rows, total)
    };

    let plans = rows
        .into_iter()
        .map(|r| PlanSummary {
            id: r.plan_code,
            name: r.name,
            pricing_model: r.pricing_model.unwrap_or_else(|| "flat_monthly".to_string()),
            included_seats: r.included_seats.unwrap_or(5),
            metered_dimensions: vec![],
            status: r.status.unwrap_or_else(|| "active".to_string()),
        })
        .collect();

    Ok(Json(PlanListResponse {
        plans,
        total,
        page,
        page_size,
    }))
}

/// Build the plan catalog router.
///
/// Mount this in the control-plane application router:
///   GET /api/ttp/plans
pub fn plans_router(pool: PgPool) -> Router {
    let state = Arc::new(PlansState { pool });
    Router::new()
        .route("/api/ttp/plans", get(list_plans))
        .with_state(state)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt;

    async fn test_pool() -> PgPool {
        let url = std::env::var("TENANT_REGISTRY_DATABASE_URL").unwrap_or_else(|_| {
            "postgresql://tenant_registry_user:tenant_registry_pass@localhost:5441/tenant_registry_db"
                .to_string()
        });
        PgPool::connect(&url).await.expect("connect to tenant-registry DB")
    }

    fn build_app(pool: PgPool) -> axum::Router {
        plans_router(pool)
    }

    #[tokio::test]
    async fn plans_returns_200_with_all_plans() {
        let pool = test_pool().await;
        let app = build_app(pool);

        let req = Request::builder()
            .uri("/api/ttp/plans?page=1&page_size=50")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.expect("call plans endpoint");
        assert_eq!(resp.status(), StatusCode::OK);

        let body = axum::body::to_bytes(resp.into_body(), 64 * 1024)
            .await
            .expect("read body");
        let json: serde_json::Value = serde_json::from_slice(&body).expect("parse JSON");

        let plans = json["plans"].as_array().expect("plans array");
        assert!(!plans.is_empty(), "should return at least one plan");

        for plan in plans {
            assert!(plan["pricing_model"].is_string(), "pricing_model must be string");
            assert!(plan["metered_dimensions"].is_array(), "metered_dimensions must be array");
            assert_eq!(
                plan["metered_dimensions"].as_array().unwrap().len(),
                0,
                "metered_dimensions always empty for now"
            );
            assert!(plan["status"].is_string(), "status must be string");
            assert!(plan["included_seats"].is_number(), "included_seats must be number");
        }
    }

    #[tokio::test]
    async fn plans_includes_starter_professional_enterprise() {
        let pool = test_pool().await;
        let app = build_app(pool);

        let req = Request::builder()
            .uri("/api/ttp/plans?page=1&page_size=50")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.expect("call plans endpoint");
        assert_eq!(resp.status(), StatusCode::OK);

        let body = axum::body::to_bytes(resp.into_body(), 64 * 1024)
            .await
            .expect("read body");
        let json: serde_json::Value = serde_json::from_slice(&body).expect("parse JSON");

        let ids: Vec<&str> = json["plans"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|p| p["id"].as_str())
            .collect();

        assert!(ids.contains(&"starter"), "starter plan must exist");
        assert!(ids.contains(&"professional"), "professional plan must exist");
        assert!(ids.contains(&"enterprise"), "enterprise plan must exist");
    }

    #[tokio::test]
    async fn plans_status_filter_works() {
        let pool = test_pool().await;
        let app = build_app(pool);

        let req = Request::builder()
            .uri("/api/ttp/plans?status=active")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.expect("call plans endpoint");
        assert_eq!(resp.status(), StatusCode::OK);

        let body = axum::body::to_bytes(resp.into_body(), 64 * 1024)
            .await
            .expect("read body");
        let json: serde_json::Value = serde_json::from_slice(&body).expect("parse JSON");

        for plan in json["plans"].as_array().unwrap() {
            assert_eq!(plan["status"], "active");
        }
    }

    #[tokio::test]
    async fn plans_response_has_pagination_fields() {
        let pool = test_pool().await;
        let app = build_app(pool);

        let req = Request::builder()
            .uri("/api/ttp/plans?page=1&page_size=2")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.expect("call plans endpoint");
        assert_eq!(resp.status(), StatusCode::OK);

        let body = axum::body::to_bytes(resp.into_body(), 64 * 1024)
            .await
            .expect("read body");
        let json: serde_json::Value = serde_json::from_slice(&body).expect("parse JSON");

        assert!(json["total"].is_number(), "total must be present");
        assert_eq!(json["page"], 1, "page must match requested");
        assert_eq!(json["page_size"], 2, "page_size must match requested");
    }
}
