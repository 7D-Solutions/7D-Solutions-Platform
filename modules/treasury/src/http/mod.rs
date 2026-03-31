pub mod accounts;
pub mod admin;
pub mod import;
pub mod recon;
pub mod recon_gl;
pub mod reports;
pub mod tenant;

use axum::{extract::State, http::StatusCode, Json};
use health::{
    build_ready_response, db_check_with_pool, ready_response_to_axum, PoolMetrics, ReadyResponse,
};
use std::sync::Arc;
use std::time::Instant;
use utoipa::OpenApi;

pub async fn openapi_json() -> Json<utoipa::openapi::OpenApi> {
    Json(ApiDoc::openapi())
}

#[derive(OpenApi)]
#[openapi(
    info(
        title = "Treasury Service",
        version = "2.1.0",
        description = "Bank account management, transaction import, reconciliation, and cash position reporting.",
    ),
    paths(
        accounts::create_bank_account,
        accounts::create_credit_card_account,
        accounts::list_accounts,
        accounts::get_account,
        accounts::update_account,
        accounts::deactivate_account,
        recon::auto_match,
        recon::manual_match,
        recon::list_matches,
        recon::list_unmatched,
        recon_gl::link_to_gl,
        recon_gl::unmatched_bank_txns,
        recon_gl::unmatched_gl_entries,
        reports::cash_position,
        reports::forecast,
        import::import_statement,
    ),
    components(schemas(
        crate::domain::accounts::TreasuryAccount,
        crate::domain::accounts::AccountStatus,
        crate::domain::accounts::AccountType,
        crate::domain::accounts::CreateBankAccountRequest,
        crate::domain::accounts::CreateCreditCardAccountRequest,
        crate::domain::accounts::UpdateAccountRequest,
        crate::domain::recon::models::ReconMatch,
        crate::domain::recon::models::ReconMatchStatus,
        crate::domain::recon::models::ReconMatchType,
        crate::domain::recon::models::AutoMatchRequest,
        crate::domain::recon::models::AutoMatchResult,
        crate::domain::recon::models::ManualMatchRequest,
        crate::domain::recon::gl_link::LinkToGlRequest,
        crate::domain::recon::gl_link::GlLinkResponse,
        crate::domain::recon::gl_link::UnmatchedBankTxnGl,
        crate::domain::recon::gl_link::UnmatchedGlRequest,
        crate::domain::recon::gl_link::UnmatchedGlResult,
        recon_gl::UnmatchedBankTxnsResponse,
        crate::domain::reports::cash_position::CashPositionResponse,
        crate::domain::reports::cash_position::AccountPosition,
        crate::domain::reports::cash_position::CashPositionSummary,
        crate::domain::reports::forecast::ForecastResponse,
        crate::domain::reports::forecast::CurrencyForecast,
        crate::domain::reports::forecast::ForecastBuckets,
        crate::domain::reports::assumptions::ForecastAssumptions,
        crate::domain::import::ImportResult,
        crate::domain::import::LineError,
        platform_http_contracts::ApiError,
        platform_http_contracts::PaginatedResponse<crate::domain::accounts::TreasuryAccount>,
        platform_http_contracts::PaginationMeta,
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
        "service": "treasury",
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
        "treasury",
        env!("CARGO_PKG_VERSION"),
        vec![db_check_with_pool(latency, db_err, pool_metrics)],
    );
    ready_response_to_axum(resp)
}

/// GET /api/version — module identity and schema version
pub async fn version() -> Json<serde_json::Value> {
    const SCHEMA_VERSION: &str = "20260218000001";

    Json(serde_json::json!({
        "module_name": "treasury",
        "module_version": env!("CARGO_PKG_VERSION"),
        "schema_version": SCHEMA_VERSION
    }))
}
