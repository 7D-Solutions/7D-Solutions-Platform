use axum::{extract::DefaultBodyLimit, routing::{get, post}, Extension, Router};
use event_bus::{EventBus, InMemoryBus, NatsBus};
use security::middleware::{
    default_rate_limiter, rate_limit_middleware, timeout_middleware, DEFAULT_BODY_LIMIT,
};
use std::net::SocketAddr;
use std::sync::Arc;
use tower_http::cors::CorsLayer;
use tracing_subscriber::EnvFilter;

use gl_rs::{
    config::Config,
    routes::account_activity::get_account_activity,
    routes::balance_sheet::get_balance_sheet,
    routes::gl_detail::get_gl_detail,
    routes::health::{health, ready, version},
    routes::income_statement::get_income_statement,
    routes::close_checklist::{
        complete_checklist_item, create_approval, create_checklist_item,
        get_approvals, get_checklist_status, waive_checklist_item,
    },
    routes::period_close::{
        close_period_handler, get_close_status, validate_close,
        request_reopen, approve_reopen, reject_reopen, list_reopen_requests,
    },
    routes::period_summary::get_period_summary,
    routes::fx_rates::{create_fx_rate, get_latest_rate as get_latest_fx_rate},
    routes::accruals::{create_template_handler, create_accrual_handler, execute_reversals_handler},
    routes::revrec::{amend_contract, create_contract, generate_schedule_handler, run_recognition_handler},
    routes::trial_balance::get_trial_balance,
    routes::cashflow::get_cash_flow,
    routes::reporting_currency::{
        get_reporting_trial_balance,
        get_reporting_income_statement,
        get_reporting_balance_sheet,
    },
    consumer::ar_tax_liability::{start_ar_tax_committed_consumer, start_ar_tax_voided_consumer},
    consumer::fixed_assets_depreciation::start_fixed_assets_depreciation_consumer,
    consumer::gl_inventory_consumer::start_gl_inventory_consumer,
    consumer::gl_writeoff_consumer::start_gl_writeoff_consumer,
    start_gl_posting_consumer,
    start_gl_reversal_consumer,
    AppState,
};

#[tokio::main]
async fn main() {
    // Load environment variables from .env file (if present)
    dotenvy::dotenv().ok();

    // Initialize tracing/logging
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();

    tracing::info!("Starting GL service...");

    // Load and validate configuration (fail-fast on missing/invalid config)
    let config = Config::from_env().unwrap_or_else(|err| {
        eprintln!("Configuration error: {}", err);
        eprintln!("GL service cannot start without valid configuration.");
        std::process::exit(1);
    });

    tracing::info!(
        "Configuration loaded: host={}, port={}, bus_type={}",
        config.host,
        config.port,
        config.bus_type
    );

    // Database connection
    tracing::info!("Connecting to database...");
    let pool = gl_rs::db::init_pool(&config.database_url)
        .await
        .expect("Failed to connect to database");

    // Run migrations
    tracing::info!("Running migrations...");
    sqlx::migrate!("./db/migrations")
        .run(&pool)
        .await
        .expect("Failed to run migrations");

    // Create event bus
    let bus: Arc<dyn EventBus> = match config.bus_type.to_lowercase().as_str() {
        "inmemory" => {
            tracing::info!("Using InMemory event bus");
            Arc::new(InMemoryBus::new())
        }
        "nats" => {
            tracing::info!("Connecting to NATS at {}", config.nats_url);
            let client = async_nats::connect(&config.nats_url)
        .await
                .expect("Failed to connect to NATS");
            Arc::new(NatsBus::new(client))
        }
        _ => panic!("Invalid BUS_TYPE: {}. Must be 'inmemory' or 'nats'", config.bus_type),
    };

    // Start GL posting consumer
    let consumer_pool = pool.clone();
    let consumer_bus = bus.clone();
    start_gl_posting_consumer(consumer_bus, consumer_pool).await;

    // Start GL reversal consumer
    let reversal_pool = pool.clone();
    let reversal_bus = bus.clone();
    start_gl_reversal_consumer(reversal_bus, reversal_pool).await;

    // Start GL write-off consumer
    let writeoff_pool = pool.clone();
    let writeoff_bus = bus.clone();
    start_gl_writeoff_consumer(writeoff_bus, writeoff_pool).await;

    // Start GL inventory COGS consumer
    let inventory_pool = pool.clone();
    let inventory_bus = bus.clone();
    start_gl_inventory_consumer(inventory_bus, inventory_pool).await;

    // Start AR tax liability consumers (tax.committed + tax.voided)
    let tax_committed_pool = pool.clone();
    let tax_committed_bus = bus.clone();
    start_ar_tax_committed_consumer(tax_committed_bus, tax_committed_pool).await;

    let tax_voided_pool = pool.clone();
    let tax_voided_bus = bus.clone();
    start_ar_tax_voided_consumer(tax_voided_bus, tax_voided_pool).await;

    // Start GL fixed assets depreciation consumer
    let fa_depr_pool = pool.clone();
    let fa_depr_bus = bus.clone();
    start_fixed_assets_depreciation_consumer(fa_depr_bus, fa_depr_pool).await;

    // Create metrics registry
    let metrics = Arc::new(
        gl_rs::metrics::GlMetrics::new()
            .expect("Failed to create metrics registry")
    );

    // Create application state
    let app_state = Arc::new(AppState {
        pool: pool.clone(),
        dlq_validation_enabled: config.dlq_validation_enabled,
        metrics: metrics.clone(),
    });

    // Build the application router
    let app = Router::new()
        .route("/api/health", get(health))
        .route("/api/ready", get(ready))
        .route("/api/version", get(version))
        .route("/metrics", get(gl_rs::metrics::metrics_handler))
        .route("/api/gl/trial-balance", get(get_trial_balance))
        .route("/api/gl/income-statement", get(get_income_statement))
        .route("/api/gl/balance-sheet", get(get_balance_sheet))
        .route("/api/gl/reporting/trial-balance", get(get_reporting_trial_balance))
        .route("/api/gl/reporting/income-statement", get(get_reporting_income_statement))
        .route("/api/gl/reporting/balance-sheet", get(get_reporting_balance_sheet))
        .route("/api/gl/periods/{period_id}/summary", get(get_period_summary))
        .route("/api/gl/periods/{period_id}/validate-close", post(validate_close))
        .route("/api/gl/periods/{period_id}/close", post(close_period_handler))
        .route("/api/gl/periods/{period_id}/close-status", get(get_close_status))
        .route("/api/gl/periods/{period_id}/checklist", get(get_checklist_status).post(create_checklist_item))
        .route("/api/gl/periods/{period_id}/checklist/{item_id}/complete", post(complete_checklist_item))
        .route("/api/gl/periods/{period_id}/checklist/{item_id}/waive", post(waive_checklist_item))
        .route("/api/gl/periods/{period_id}/approvals", get(get_approvals).post(create_approval))
        .route("/api/gl/periods/{period_id}/reopen", get(list_reopen_requests).post(request_reopen))
        .route("/api/gl/periods/{period_id}/reopen/{request_id}/approve", post(approve_reopen))
        .route("/api/gl/periods/{period_id}/reopen/{request_id}/reject", post(reject_reopen))
        .route("/api/gl/detail", get(get_gl_detail))
        .route("/api/gl/accounts/{account_code}/activity", get(get_account_activity))
        .route("/api/gl/fx-rates", post(create_fx_rate))
        .route("/api/gl/fx-rates/latest", get(get_latest_fx_rate))
        .route("/api/gl/revrec/contracts", post(create_contract))
        .route("/api/gl/revrec/schedules", post(generate_schedule_handler))
        .route("/api/gl/revrec/recognition-runs", post(run_recognition_handler))
        .route("/api/gl/revrec/amendments", post(amend_contract))
        .route("/api/gl/accruals/templates", post(create_template_handler))
        .route("/api/gl/accruals/create", post(create_accrual_handler))
        .route("/api/gl/accruals/reversals/execute", post(execute_reversals_handler))
        .route("/api/gl/cash-flow", get(get_cash_flow))
        .with_state(app_state)
        .layer(DefaultBodyLimit::max(DEFAULT_BODY_LIMIT))
        .layer(axum::middleware::from_fn(security::tracing::tracing_context_middleware))
        .layer(axum::middleware::from_fn(timeout_middleware))
        .layer(axum::middleware::from_fn(rate_limit_middleware))
        .layer(Extension(default_rate_limiter()))
        .layer(security::AuthzLayer::from_env())
        .layer(
            CorsLayer::new()
                .allow_origin(tower_http::cors::Any)
                .allow_methods(tower_http::cors::Any)
                .allow_headers(tower_http::cors::Any),
        )
        .into_make_service_with_connect_info::<SocketAddr>();

    // Bind to the configured address
    let addr = SocketAddr::from(([0, 0, 0, 0], config.port));
    tracing::info!("GL service listening on {}", addr);

    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .expect("Failed to bind address");

    // Start the server
    axum::serve(listener, app)
        .await
        .expect("Server failed to start");
}
