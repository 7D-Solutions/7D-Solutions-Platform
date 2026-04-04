use axum::{
    routing::{get, post},
    Json, Router,
};
use security::{permissions, RequirePermissionsLayer};
use std::sync::Arc;
use utoipa::OpenApi;

use reporting::{http, metrics, AppState};

// ── OpenAPI spec ─────────────────────────────────────────────────────────────

use platform_http_contracts::ApiError;
use reporting::domain::{
    aging::{ap_aging, ar_aging},
    forecast::types::{AtRiskItem, CashForecastResponse, CurrencyForecast, ForecastHorizon},
    jobs::snapshot_runner::SnapshotRunResult,
    kpis::KpiSnapshot,
    statements::{
        balance_sheet::{BalanceSheet, BsAccountLine, BsSection},
        cashflow::{CashflowLine, CashflowSection, CashflowStatement},
        pl::{PlAccountLine, PlSection, PlStatement},
    },
};
use platform_http_contracts::{PaginatedResponse, PaginationMeta};
use reporting::http::{
    admin::{
        RebuildRequest, CursorStatusSchema, ProjectionStatusSchema,
        ConsistencyCheckSchema, ProjectionSummarySchema,
    },
    aging::ArAgingResponse,
};

#[derive(OpenApi)]
#[openapi(
    info(
        title = "Reporting Service",
        version = "3.0.0",
        description = "Financial reporting: aging, KPIs, statements, cash flow forecasts, \
                        and probabilistic collection forecasting.",
    ),
    paths(
        reporting::http::statements::get_pl,
        reporting::http::statements::get_balance_sheet,
        reporting::http::cashflow::get_cashflow,
        reporting::http::aging::get_ar_aging,
        reporting::http::aging::get_ap_aging,
        reporting::http::kpis::get_kpis,
        reporting::http::forecast::get_forecast,
        reporting::http::admin::rebuild,
        reporting::http::admin::projection_status,
        reporting::http::admin::consistency_check,
        reporting::http::admin::list_projections,
        reporting::http::health,
        reporting::http::ready,
        reporting::http::version,
    ),
    components(schemas(
        PlStatement, PlSection, PlAccountLine,
        BalanceSheet, BsSection, BsAccountLine,
        CashflowStatement, CashflowSection, CashflowLine,
        ar_aging::ArAgingSummary, ar_aging::ArAgingRow,
        ArAgingResponse,
        ap_aging::ApAgingReport, ap_aging::VendorAgingRow, ap_aging::CurrencySummary,
        KpiSnapshot,
        CashForecastResponse, CurrencyForecast, ForecastHorizon, AtRiskItem,
        SnapshotRunResult,
        RebuildRequest,
        CursorStatusSchema, ProjectionStatusSchema,
        ConsistencyCheckSchema, ProjectionSummarySchema,
        PaginatedResponse<ProjectionSummarySchema>, PaginationMeta,
        ApiError,
    )),
    security(
        ("bearer" = [])
    ),
    modifiers(&SecurityAddon),
)]
struct ApiDoc;

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

async fn openapi_json() -> Json<utoipa::openapi::OpenApi> {
    Json(ApiDoc::openapi())
}

// ── main ─────────────────────────────────────────────────────────────────────

use platform_sdk::ModuleBuilder;

static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./db/migrations");

#[tokio::main]
async fn main() {
    ModuleBuilder::from_manifest("module.toml")
        .migrator(&MIGRATOR)
        .routes(|ctx| {
            let reporting_metrics = Arc::new(
                metrics::ReportingMetrics::new().expect("Reporting: failed to create metrics"),
            );
            let app_state = Arc::new(AppState {
                pool: ctx.pool().clone(),
                metrics: reporting_metrics,
            });

            let reporting_mutations = Router::new()
                .route("/api/reporting/rebuild", post(http::admin::rebuild))
                .route_layer(RequirePermissionsLayer::new(&[
                    permissions::REPORTING_MUTATE,
                ]))
                .with_state(app_state.clone());

            let reporting_reads = Router::new()
                .route("/api/reporting/pl", get(http::statements::get_pl))
                .route(
                    "/api/reporting/balance-sheet",
                    get(http::statements::get_balance_sheet),
                )
                .route("/api/reporting/cashflow", get(http::cashflow::get_cashflow))
                .route("/api/reporting/ar-aging", get(http::aging::get_ar_aging))
                .route("/api/reporting/ap-aging", get(http::aging::get_ap_aging))
                .route("/api/reporting/kpis", get(http::kpis::get_kpis))
                .route("/api/reporting/forecast", get(http::forecast::get_forecast))
                .route_layer(RequirePermissionsLayer::new(&[
                    permissions::REPORTING_READ,
                ]))
                .with_state(app_state);

            Router::new()
                .merge(reporting_reads)
                .merge(reporting_mutations)
                .merge(http::admin::admin_router(ctx.pool().clone()))
                .route("/api/openapi.json", get(openapi_json))
        })
        .run()
        .await
        .expect("reporting module failed");
}
