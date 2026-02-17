use axum::{routing::{get, post}, Router};
use event_bus::{EventBus, InMemoryBus, NatsBus};
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
    routes::period_close::{close_period_handler, get_close_status, validate_close},
    routes::period_summary::get_period_summary,
    routes::fx_rates::{create_fx_rate, get_latest_rate as get_latest_fx_rate},
    routes::accruals::{create_template_handler, create_accrual_handler, execute_reversals_handler},
    routes::revrec::{create_contract, generate_schedule_handler, run_recognition_handler},
    routes::trial_balance::get_trial_balance,
    routes::cashflow::get_cash_flow,
    routes::reporting_currency::{
        get_reporting_trial_balance,
        get_reporting_income_statement,
        get_reporting_balance_sheet,
    },
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
        .route("/api/gl/detail", get(get_gl_detail))
        .route("/api/gl/accounts/{account_code}/activity", get(get_account_activity))
        .route("/api/gl/fx-rates", post(create_fx_rate))
        .route("/api/gl/fx-rates/latest", get(get_latest_fx_rate))
        .route("/api/gl/revrec/contracts", post(create_contract))
        .route("/api/gl/revrec/schedules", post(generate_schedule_handler))
        .route("/api/gl/revrec/recognition-runs", post(run_recognition_handler))
        .route("/api/gl/accruals/templates", post(create_template_handler))
        .route("/api/gl/accruals/create", post(create_accrual_handler))
        .route("/api/gl/accruals/reversals/execute", post(execute_reversals_handler))
        .route("/api/gl/cash-flow", get(get_cash_flow))
        .with_state(app_state)
        .layer(
            CorsLayer::new()
                .allow_origin(tower_http::cors::Any)
                .allow_methods(tower_http::cors::Any)
                .allow_headers(tower_http::cors::Any),
        );

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
