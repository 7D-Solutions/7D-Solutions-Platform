use axum::{
    extract::DefaultBodyLimit,
    routing::{get, post},
    Extension, Json, Router,
};
use event_bus::{EventBus, InMemoryBus, NatsBus};
use security::middleware::{
    default_rate_limiter, rate_limit_middleware, timeout_middleware, DEFAULT_BODY_LIMIT,
};
use security::{optional_claims_mw, permissions, JwtVerifier, RequirePermissionsLayer};
use std::net::SocketAddr;
use std::sync::Arc;
use tower_http::cors::{AllowOrigin, CorsLayer};
use tracing_subscriber::EnvFilter;
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
    http::fx_rates::{create_fx_rate, get_latest_rate as get_latest_fx_rate},
    http::gl_detail::get_gl_detail,
    http::health::{health, ready, version},
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
    http::exports::create_export,
    http::trial_balance::get_trial_balance,
    start_gl_posting_consumer, start_gl_reversal_consumer, AppState,
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

    let shutdown_pool = pool.clone();

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
            let client = event_bus::connect_nats(&config.nats_url)
                .await
                .expect("Failed to connect to NATS");
            Arc::new(NatsBus::new(client))
        }
        _ => panic!(
            "Invalid BUS_TYPE: {}. Must be 'inmemory' or 'nats'",
            config.bus_type
        ),
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

    // Start GL credit note consumer
    let credit_note_pool = pool.clone();
    let credit_note_bus = bus.clone();
    start_gl_credit_note_consumer(credit_note_bus, credit_note_pool).await;

    // Start AP vendor bill approved consumer
    let ap_bill_pool = pool.clone();
    let ap_bill_bus = bus.clone();
    start_ap_vendor_bill_approved_consumer(ap_bill_bus, ap_bill_pool).await;

    // Start GL realized FX gain/loss consumer
    let fx_realized_pool = pool.clone();
    let fx_realized_bus = bus.clone();
    start_gl_fx_realized_consumer(fx_realized_bus, fx_realized_pool).await;

    // Start timekeeping labor cost consumer
    let labor_pool = pool.clone();
    let labor_bus = bus.clone();
    start_gl_labor_cost_consumer(labor_bus, labor_pool).await;

    // Create metrics registry
    let metrics =
        Arc::new(gl_rs::metrics::GlMetrics::new().expect("Failed to create metrics registry"));

    // Create application state
    let app_state = Arc::new(AppState {
        pool: pool.clone(),
        dlq_validation_enabled: config.dlq_validation_enabled,
        metrics: metrics.clone(),
    });

    // Optional JWT verifier for claims extraction (requires JWT_PUBLIC_KEY env var).
    let maybe_verifier = JwtVerifier::from_env_with_overlap().map(Arc::new);

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

    // Build the application router
    let app = Router::new()
        // Ops
        .route("/healthz", get(health::healthz))
        .route("/api/health", get(health))
        .route("/api/ready", get(ready))
        .route("/api/version", get(version))
        .route("/api/openapi.json", get(openapi_json))
        .route("/metrics", get(gl_rs::metrics::metrics_handler))
        // GL read routes
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
        .with_state(app_state)
        .merge(gl_mutations)
        .merge(gl_rs::http::admin::admin_router(pool.clone()))
        .layer(DefaultBodyLimit::max(DEFAULT_BODY_LIMIT))
        .layer(axum::middleware::from_fn(
            security::tracing::tracing_context_middleware,
        ))
        .layer(axum::middleware::from_fn(timeout_middleware))
        .layer(axum::middleware::from_fn(rate_limit_middleware))
        .layer(Extension(default_rate_limiter()))
        .layer(axum::middleware::from_fn_with_state(
            maybe_verifier,
            optional_claims_mw,
        ))
        .layer(build_cors_layer(&config))
        .into_make_service_with_connect_info::<SocketAddr>();

    // Bind to the configured address
    let addr = SocketAddr::from(([0, 0, 0, 0], config.port));
    tracing::info!("GL service listening on {}", addr);

    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .expect("Failed to bind address");

    // Start the server
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .expect("Server failed to start");

    tracing::info!("Server stopped — closing resources");
    shutdown_pool.close().await;
    tracing::info!("Shutdown complete");
}

async fn openapi_json() -> Json<utoipa::openapi::OpenApi> {
    Json(gl_rs::http::ApiDoc::openapi())
}

async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install SIGTERM handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }

    tracing::info!("Shutdown signal received — draining in-flight requests");
}

fn build_cors_layer(config: &Config) -> CorsLayer {
    let is_wildcard = config.cors_origins.len() == 1 && config.cors_origins[0] == "*";

    if is_wildcard && config.env != "development" {
        tracing::warn!(
            "CORS_ORIGINS is set to wildcard — restrict to specific origins in production"
        );
    }

    let layer = if is_wildcard {
        CorsLayer::new().allow_origin(AllowOrigin::any())
    } else {
        let origins: Vec<_> = config
            .cors_origins
            .iter()
            .filter_map(|o| o.parse().ok())
            .collect();
        CorsLayer::new().allow_origin(origins)
    };

    layer
        .allow_methods(tower_http::cors::Any)
        .allow_headers(tower_http::cors::Any)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cors_wildcard_parses() {
        let config = Config {
            database_url: "postgresql://localhost/test".to_string(),
            bus_type: "inmemory".to_string(),
            nats_url: "nats://localhost:4222".to_string(),
            host: "0.0.0.0".to_string(),
            port: 8090,
            env: "development".to_string(),
            cors_origins: vec!["*".to_string()],
            dlq_validation_enabled: false,
        };
        let _layer = build_cors_layer(&config);
    }

    #[test]
    fn cors_specific_origins_parse() {
        let config = Config {
            database_url: "postgresql://localhost/test".to_string(),
            bus_type: "inmemory".to_string(),
            nats_url: "nats://localhost:4222".to_string(),
            host: "0.0.0.0".to_string(),
            port: 8090,
            env: "development".to_string(),
            cors_origins: vec![
                "http://localhost:3000".to_string(),
                "https://app.example.com".to_string(),
            ],
            dlq_validation_enabled: false,
        };
        let _layer = build_cors_layer(&config);
    }
}
