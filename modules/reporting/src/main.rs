use axum::{extract::DefaultBodyLimit, http::Method, routing::{get, post}, Extension, Router};
use security::{
    middleware::{default_rate_limiter, rate_limit_middleware, timeout_middleware, DEFAULT_BODY_LIMIT},
    optional_claims_mw, permissions, JwtVerifier, RequirePermissionsLayer,
};
use std::net::SocketAddr;
use std::sync::Arc;
use tower_http::cors::{AllowOrigin, CorsLayer};
use tracing_subscriber::EnvFilter;

use reporting::{config::Config, db, http, metrics, AppState};

#[tokio::main]
async fn main() {
    dotenvy::dotenv().ok();

    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();

    let config = Config::from_env().unwrap_or_else(|err| {
        eprintln!("Configuration error: {}", err);
        eprintln!("Reporting service cannot start without valid configuration.");
        std::process::exit(1);
    });

    tracing::info!(
        "Reporting: config loaded: host={}, port={}",
        config.host,
        config.port
    );

    // Resolve DB pool through the app_id-scoped resolver seam.
    // DATABASE_URL must name the database following the reporting_{app_id}_db convention.
    let pool = db::resolve_pool(&config.database_url)
        .await
        .expect("Reporting: failed to connect to Postgres");

    let shutdown_pool = pool.clone();

    sqlx::migrate!("./db/migrations")
        .run(&pool)
        .await
        .expect("Reporting: failed to run database migrations");

    tracing::info!("Reporting: database migrations applied");

    // Metrics
    let reporting_metrics = Arc::new(
        metrics::ReportingMetrics::new().expect("Reporting: failed to create metrics"),
    );
    tracing::info!("Reporting: metrics initialized");

    let app_state = Arc::new(AppState {
        pool: pool.clone(),
        metrics: reporting_metrics,
    });

    let maybe_verifier = JwtVerifier::from_env_with_overlap().map(Arc::new);

    let reporting_mutations = Router::new()
        // Rebuild trigger — write
        .route("/api/reporting/rebuild", post(http::admin::rebuild))
        .route_layer(RequirePermissionsLayer::new(&[permissions::REPORTING_MUTATE]))
        .with_state(app_state.clone());

    let app = Router::new()
        .route("/healthz", get(health::healthz))
        .route("/api/health", get(http::health))
        .route("/api/ready", get(http::ready))
        .route("/api/version", get(http::version))
        .route("/metrics", get(metrics::metrics_handler))
        // Reports — read
        .route("/api/reporting/pl", get(http::statements::get_pl))
        .route("/api/reporting/balance-sheet", get(http::statements::get_balance_sheet))
        .route("/api/reporting/cashflow", get(http::cashflow::get_cashflow))
        .route("/api/reporting/ar-aging", get(http::aging::get_ar_aging))
        .route("/api/reporting/ap-aging", get(http::aging::get_ap_aging))
        .route("/api/reporting/kpis", get(http::kpis::get_kpis))
        .route("/api/reporting/forecast", get(http::forecast::get_forecast))
        .with_state(app_state)
        .merge(reporting_mutations)
        .merge(http::admin::admin_router(pool.clone()))
        .layer(DefaultBodyLimit::max(DEFAULT_BODY_LIMIT))
        .layer(axum::middleware::from_fn(security::tracing::tracing_context_middleware))
        .layer(axum::middleware::from_fn(timeout_middleware))
        .layer(axum::middleware::from_fn(rate_limit_middleware))
        .layer(Extension(default_rate_limiter()))
        .layer(axum::middleware::from_fn_with_state(maybe_verifier, optional_claims_mw))
        .layer(build_cors_layer(&config))
        .into_make_service_with_connect_info::<SocketAddr>();

    let addr: SocketAddr = format!("{}:{}", config.host, config.port)
        .parse()
        .expect("Invalid HOST:PORT");

    tracing::info!("Reporting service listening on {addr}");

    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .expect("Reporting: failed to bind address");

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .expect("Reporting: failed to start server");

    tracing::info!("Server stopped — closing resources");
    shutdown_pool.close().await;
    tracing::info!("Shutdown complete");
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
        tracing::warn!("CORS_ORIGINS is set to wildcard — restrict to specific origins in production");
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
        .allow_credentials(false)
}
