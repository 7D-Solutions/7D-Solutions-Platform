use axum::{
    routing::{get, post, put},
    Json, Router,
};
use std::sync::Arc;
use utoipa::OpenApi;

use ap::{http, metrics, AppState};
use platform_sdk::ModuleBuilder;
use security::{permissions, RequirePermissionsLayer};

async fn openapi_json() -> Json<utoipa::openapi::OpenApi> {
    Json(http::ApiDoc::openapi())
}

static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./db/migrations");

#[tokio::main]
async fn main() {
    ModuleBuilder::from_manifest("module.toml")
        .migrator(&MIGRATOR)
        .routes(|ctx| {
            let ap_metrics =
                Arc::new(metrics::ApMetrics::new().expect("AP: failed to create metrics"));

            // Optional GL pool for period pre-validation.
            // Connects lazily — no startup failure if GL_DATABASE_URL is absent.
            let gl_pool = std::env::var("GL_DATABASE_URL")
                .ok()
                .and_then(|url| sqlx::PgPool::connect_lazy(&url).ok());

            let app_state = Arc::new(AppState {
                pool: ctx.pool().clone(),
                metrics: ap_metrics,
                gl_pool,
            });

            if let Ok(bus) = ctx.bus_arc() {
                ap::consumers::attachment_linked::start_attachment_linked_consumer(
                    bus,
                    ctx.pool().clone(),
                );
                tracing::info!("AP: attachment_linked consumer started");
            }

            let ap_mutations = Router::new()
                .route("/api/ap/vendors", post(http::vendors::create_vendor))
                .route(
                    "/api/ap/import/vendors",
                    post(http::imports::import_vendors),
                )
                .route(
                    "/api/ap/vendors/{vendor_id}",
                    put(http::vendors::update_vendor),
                )
                .route(
                    "/api/ap/vendors/{vendor_id}/deactivate",
                    post(http::vendors::deactivate_vendor),
                )
                .route(
                    "/api/ap/vendors/{vendor_id}/qualify",
                    post(http::vendors::qualify_vendor),
                )
                .route(
                    "/api/ap/vendors/{vendor_id}/prefer",
                    post(http::vendors::mark_vendor_preferred),
                )
                .route(
                    "/api/ap/vendors/{vendor_id}/unprefer",
                    post(http::vendors::unmark_vendor_preferred),
                )
                .route("/api/ap/pos", post(http::purchase_orders::create_po))
                .route(
                    "/api/ap/pos/{po_id}/lines",
                    put(http::purchase_orders::update_po_lines),
                )
                .route(
                    "/api/ap/pos/{po_id}/approve",
                    post(http::purchase_orders::approve_po),
                )
                .route("/api/ap/bills", post(http::bills::create_bill))
                .route(
                    "/api/ap/bills/{bill_id}/match",
                    post(http::bills::match_bill),
                )
                .route(
                    "/api/ap/bills/{bill_id}/approve",
                    post(http::bills::approve_bill),
                )
                .route("/api/ap/bills/{bill_id}/void", post(http::bills::void_bill))
                .route(
                    "/api/ap/bills/{bill_id}/tax-quote",
                    post(http::bills::quote_bill_tax),
                )
                .route(
                    "/api/ap/bills/{bill_id}/allocations",
                    post(http::allocations::create_allocation),
                )
                .route(
                    "/api/ap/payment-terms",
                    post(http::payment_terms::create_terms),
                )
                .route(
                    "/api/ap/payment-terms/{term_id}",
                    put(http::payment_terms::update_terms),
                )
                .route(
                    "/api/ap/bills/{bill_id}/assign-terms",
                    post(http::payment_terms::assign_terms),
                )
                .route("/api/ap/payment-runs", post(http::payment_runs::create_run))
                .route(
                    "/api/ap/payment-runs/{run_id}/execute",
                    post(http::payment_runs::execute_run),
                )
                .route_layer(RequirePermissionsLayer::new(&[permissions::AP_MUTATE]))
                .with_state(app_state.clone());

            let ap_reads = Router::new()
                .route("/api/ap/vendors", get(http::vendors::list_vendors))
                .route(
                    "/api/ap/vendors/{vendor_id}",
                    get(http::vendors::get_vendor),
                )
                .route(
                    "/api/ap/vendors/{vendor_id}/qualification-history",
                    get(http::vendors::get_vendor_qualification_history),
                )
                .route("/api/ap/pos", get(http::purchase_orders::list_pos))
                .route("/api/ap/pos/{po_id}", get(http::purchase_orders::get_po))
                .route("/api/ap/bills", get(http::bills::list_bills))
                .route("/api/ap/bills/{bill_id}", get(http::bills::get_bill))
                .route(
                    "/api/ap/bills/{bill_id}/allocations",
                    get(http::allocations::list_allocations),
                )
                .route(
                    "/api/ap/bills/{bill_id}/balance",
                    get(http::allocations::get_balance),
                )
                .route(
                    "/api/ap/payment-terms",
                    get(http::payment_terms::list_terms),
                )
                .route(
                    "/api/ap/payment-terms/{term_id}",
                    get(http::payment_terms::get_terms),
                )
                .route(
                    "/api/ap/payment-runs/{run_id}",
                    get(http::payment_runs::get_run),
                )
                .route("/api/ap/aging", get(http::reports::aging_report))
                .route(
                    "/api/ap/tax/reports/summary",
                    get(http::tax_reports::tax_report_summary),
                )
                .route(
                    "/api/ap/tax/reports/export",
                    get(http::tax_reports::tax_report_export),
                )
                .route_layer(RequirePermissionsLayer::new(&[permissions::AP_READ]))
                .with_state(app_state);

            Router::new()
                .route("/api/openapi.json", get(openapi_json))
                .merge(ap_reads)
                .merge(ap_mutations)
                .merge(http::admin::admin_router(ctx.pool().clone()))
        })
        .run()
        .await
        .expect("ap module failed");
}
