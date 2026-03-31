use axum::{
    routing::{get, post},
    Json, Router,
};
use security::{permissions, RequirePermissionsLayer};
use std::sync::Arc;
use utoipa::OpenApi;

use gl_rs::{
    config::Config,
    consumers::ap_vendor_bill_approved_consumer::start_ap_vendor_bill_approved_consumer,
    consumers::ar_tax_liability::{start_ar_tax_committed_consumer, start_ar_tax_voided_consumer},
    consumers::fixed_assets_depreciation::start_fixed_assets_depreciation_consumer,
    consumers::gl_credit_note_consumer::start_gl_credit_note_consumer,
    consumers::gl_fx_realized_consumer::start_gl_fx_realized_consumer,
    consumers::gl_inventory_consumer::start_gl_inventory_consumer,
    consumers::gl_writeoff_consumer::start_gl_writeoff_consumer,
    consumers::timekeeping_labor_cost::start_gl_labor_cost_consumer,
    http::account_activity::get_account_activity,
    http::accounts::create_account,
    http::accruals::{
        create_accrual_handler, create_template_handler, execute_reversals_handler,
    },
    http::balance_sheet::get_balance_sheet,
    http::cashflow::get_cash_flow,
    http::close_checklist::{
        complete_checklist_item, create_approval, create_checklist_item, get_approvals,
        get_checklist_status, waive_checklist_item,
    },
    http::exports::create_export,
    http::fx_rates::{create_fx_rate, get_latest_rate as get_latest_fx_rate},
    http::gl_detail::get_gl_detail,
    http::income_statement::get_income_statement,
    http::period_close::{
        approve_reopen, close_period_handler, get_close_status, list_reopen_requests,
        reject_reopen, request_reopen, validate_close,
    },
    http::period_summary::get_period_summary,
    http::reporting_currency::{
        get_reporting_balance_sheet, get_reporting_income_statement, get_reporting_trial_balance,
    },
    http::revrec::{
        amend_contract, create_contract, generate_schedule_handler, run_recognition_handler,
    },
    http::trial_balance::get_trial_balance,
    start_gl_posting_consumer, start_gl_reversal_consumer, AppState,
};
use platform_sdk::ModuleBuilder;

static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./db/migrations");

#[tokio::main]
async fn main() {
    ModuleBuilder::from_manifest("module.toml")
        .migrator(&MIGRATOR)
        .routes_async(|ctx| async move {
            let pool = ctx.pool().clone();
            let config = Config::from_env().unwrap_or_else(|err| {
                tracing::error!("GL config error: {}", err);
                panic!("GL config error: {}", err);
            });

            // Get event bus from SDK context and start all consumers.
            // GL consumers have their own retry + DLQ handling.
            let bus = ctx.bus_arc().expect("GL requires event bus");

            start_gl_posting_consumer(bus.clone(), pool.clone()).await;
            start_gl_reversal_consumer(bus.clone(), pool.clone()).await;
            start_gl_writeoff_consumer(bus.clone(), pool.clone()).await;
            start_gl_inventory_consumer(bus.clone(), pool.clone()).await;
            start_ar_tax_committed_consumer(bus.clone(), pool.clone()).await;
            start_ar_tax_voided_consumer(bus.clone(), pool.clone()).await;
            start_fixed_assets_depreciation_consumer(bus.clone(), pool.clone()).await;
            start_gl_credit_note_consumer(bus.clone(), pool.clone()).await;
            start_ap_vendor_bill_approved_consumer(bus.clone(), pool.clone()).await;
            start_gl_fx_realized_consumer(bus.clone(), pool.clone()).await;
            start_gl_labor_cost_consumer(bus, pool.clone()).await;

            let metrics = Arc::new(
                gl_rs::metrics::GlMetrics::new().expect("Failed to create metrics registry"),
            );
            // Register SLO metrics with global prometheus registry so
            // SDK's /metrics endpoint picks them up via prometheus::gather().
            let _ = prometheus::register(Box::new(
                metrics.http_request_duration_seconds.clone(),
            ));
            let _ = prometheus::register(Box::new(
                metrics.http_requests_total.clone(),
            ));
            let _ = prometheus::register(Box::new(
                metrics.outbox_queue_depth.clone(),
            ));

            let app_state = Arc::new(AppState {
                pool: pool.clone(),
                dlq_validation_enabled: config.dlq_validation_enabled,
                metrics,
            });

            // GL mutation routes — require gl.post permission.
            let gl_mutations = Router::new()
                .route(
                    "/api/gl/periods/{period_id}/validate-close",
                    post(validate_close),
                )
                .route(
                    "/api/gl/periods/{period_id}/close",
                    post(close_period_handler),
                )
                .route(
                    "/api/gl/periods/{period_id}/checklist",
                    post(create_checklist_item),
                )
                .route(
                    "/api/gl/periods/{period_id}/checklist/{item_id}/complete",
                    post(complete_checklist_item),
                )
                .route(
                    "/api/gl/periods/{period_id}/checklist/{item_id}/waive",
                    post(waive_checklist_item),
                )
                .route(
                    "/api/gl/periods/{period_id}/approvals",
                    post(create_approval),
                )
                .route("/api/gl/periods/{period_id}/reopen", post(request_reopen))
                .route(
                    "/api/gl/periods/{period_id}/reopen/{request_id}/approve",
                    post(approve_reopen),
                )
                .route(
                    "/api/gl/periods/{period_id}/reopen/{request_id}/reject",
                    post(reject_reopen),
                )
                .route("/api/gl/fx-rates", post(create_fx_rate))
                .route("/api/gl/revrec/contracts", post(create_contract))
                .route("/api/gl/revrec/schedules", post(generate_schedule_handler))
                .route(
                    "/api/gl/revrec/recognition-runs",
                    post(run_recognition_handler),
                )
                .route("/api/gl/revrec/amendments", post(amend_contract))
                .route("/api/gl/accruals/templates", post(create_template_handler))
                .route("/api/gl/accruals/create", post(create_accrual_handler))
                .route(
                    "/api/gl/accruals/reversals/execute",
                    post(execute_reversals_handler),
                )
                .route("/api/gl/exports", post(create_export))
                .route("/api/gl/accounts", post(create_account))
                .route_layer(RequirePermissionsLayer::new(&[permissions::GL_POST]))
                .with_state(app_state.clone());

            let gl_reads = Router::new()
                .route("/api/gl/trial-balance", get(get_trial_balance))
                .route("/api/gl/income-statement", get(get_income_statement))
                .route("/api/gl/balance-sheet", get(get_balance_sheet))
                .route(
                    "/api/gl/reporting/trial-balance",
                    get(get_reporting_trial_balance),
                )
                .route(
                    "/api/gl/reporting/income-statement",
                    get(get_reporting_income_statement),
                )
                .route(
                    "/api/gl/reporting/balance-sheet",
                    get(get_reporting_balance_sheet),
                )
                .route(
                    "/api/gl/periods/{period_id}/summary",
                    get(get_period_summary),
                )
                .route(
                    "/api/gl/periods/{period_id}/close-status",
                    get(get_close_status),
                )
                .route(
                    "/api/gl/periods/{period_id}/checklist",
                    get(get_checklist_status),
                )
                .route("/api/gl/periods/{period_id}/approvals", get(get_approvals))
                .route(
                    "/api/gl/periods/{period_id}/reopen",
                    get(list_reopen_requests),
                )
                .route("/api/gl/detail", get(get_gl_detail))
                .route(
                    "/api/gl/accounts/{account_code}/activity",
                    get(get_account_activity),
                )
                .route("/api/gl/fx-rates/latest", get(get_latest_fx_rate))
                .route("/api/gl/cash-flow", get(get_cash_flow))
                .route_layer(RequirePermissionsLayer::new(&[permissions::GL_READ]))
                .with_state(app_state.clone());

            Router::new()
                .route("/api/openapi.json", get(openapi_json))
                .with_state(app_state)
                .merge(gl_reads)
                .merge(gl_mutations)
                .merge(gl_rs::http::admin::admin_router(pool))
        })
        .run()
        .await
        .expect("gl module failed");
}

async fn openapi_json() -> Json<utoipa::openapi::OpenApi> {
    Json(gl_rs::http::ApiDoc::openapi())
}
