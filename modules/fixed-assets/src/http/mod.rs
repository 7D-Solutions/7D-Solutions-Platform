pub mod admin;
pub mod admin_types;
pub mod assets;
pub mod depreciation;
pub mod disposals;
pub mod helpers;

use axum::{extract::State, http::StatusCode, Json};
use health::{
    build_ready_response, db_check_with_pool, ready_response_to_axum, PoolMetrics, ReadyResponse,
};
use std::sync::Arc;
use std::time::Instant;
use utoipa::OpenApi;

#[derive(OpenApi)]
#[openapi(
    info(
        title = "Fixed Assets Service",
        version = "2.1.0",
        description = "Fixed asset lifecycle: capitalization, depreciation schedules, disposals, and impairments.",
    ),
    paths(
        // Categories
        assets::create_category,
        assets::update_category,
        assets::deactivate_category,
        assets::get_category,
        assets::list_categories,
        // Assets
        assets::create_asset,
        assets::update_asset,
        assets::deactivate_asset,
        assets::get_asset,
        assets::list_assets,
        // Depreciation
        depreciation::generate_schedule,
        depreciation::create_run,
        depreciation::list_runs,
        depreciation::get_run,
        // Disposals
        disposals::dispose_asset,
        disposals::list_disposals,
        disposals::get_disposal,
    ),
    components(schemas(
        // Categories & Assets
        crate::domain::assets::Category,
        crate::domain::assets::Asset,
        crate::domain::assets::DepreciationMethod,
        crate::domain::assets::AssetStatus,
        crate::domain::assets::CreateCategoryRequest,
        crate::domain::assets::UpdateCategoryRequest,
        crate::domain::assets::CreateAssetRequest,
        crate::domain::assets::UpdateAssetRequest,
        // Depreciation
        crate::domain::depreciation::DepreciationSchedule,
        crate::domain::depreciation::DepreciationRun,
        crate::domain::depreciation::GenerateScheduleRequest,
        crate::domain::depreciation::CreateRunRequest,
        // Disposals
        crate::domain::disposals::Disposal,
        crate::domain::disposals::DisposalType,
        crate::domain::disposals::DisposeAssetRequest,
        // Platform
        platform_http_contracts::ApiError,
        platform_http_contracts::PaginatedResponse<crate::domain::assets::Category>,
        platform_http_contracts::PaginatedResponse<crate::domain::assets::Asset>,
        platform_http_contracts::PaginatedResponse<crate::domain::depreciation::DepreciationRun>,
        platform_http_contracts::PaginatedResponse<crate::domain::disposals::Disposal>,
    )),
    security(("bearer" = [])),
    modifiers(&SecurityAddon),
)]
pub struct ApiDoc;

struct SecurityAddon;

impl utoipa::Modify for SecurityAddon {
    fn modify(&self, openapi: &mut utoipa::openapi::OpenApi) {
        let components = openapi.components.get_or_insert_with(Default::default);
        components.add_security_scheme(
            "bearer",
            utoipa::openapi::security::SecurityScheme::Http(
                utoipa::openapi::security::HttpBuilder::new()
                    .scheme(utoipa::openapi::security::HttpAuthScheme::Bearer)
                    .bearer_format("JWT")
                    .build(),
            ),
        );
    }
}

/// GET /api/health — liveness probe (legacy, kept for compat)
pub async fn health() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "status": "healthy",
        "service": "fixed-assets",
        "version": env!("CARGO_PKG_VERSION")
    }))
}

/// GET /api/ready — readiness probe (verifies DB connectivity)
pub async fn ready(
    State(state): State<Arc<crate::AppState>>,
) -> Result<Json<ReadyResponse>, (StatusCode, Json<ReadyResponse>)> {
    let start = Instant::now();
    let db_err = sqlx::query("SELECT 1")
        .execute(&state.pool)
        .await
        .err()
        .map(|e| e.to_string());
    let latency = start.elapsed().as_millis() as u64;

    let pool_metrics = PoolMetrics {
        size: state.pool.size(),
        idle: state.pool.num_idle() as u32,
        active: state
            .pool
            .size()
            .saturating_sub(state.pool.num_idle() as u32),
    };

    let resp = build_ready_response(
        "fixed-assets",
        env!("CARGO_PKG_VERSION"),
        vec![db_check_with_pool(latency, db_err, pool_metrics)],
    );
    ready_response_to_axum(resp)
}

/// GET /api/version — module identity and schema version
pub async fn version() -> Json<serde_json::Value> {
    const SCHEMA_VERSION: &str = "00000000000000";

    Json(serde_json::json!({
        "module_name": "fixed-assets",
        "module_version": env!("CARGO_PKG_VERSION"),
        "schema_version": SCHEMA_VERSION
    }))
}
