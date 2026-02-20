use axum::{extract::DefaultBodyLimit, http::Method, routing::{get, post}, Extension, Router};
use security::{
    middleware::{default_rate_limiter, rate_limit_middleware, timeout_middleware, DEFAULT_BODY_LIMIT},
    optional_claims_mw, permissions, JwtVerifier, RequirePermissionsLayer,
};
use std::net::SocketAddr;
use std::sync::Arc;
use tower_http::cors::CorsLayer;
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

    let cors = CorsLayer::new()
        .allow_origin([
            "http://localhost:5173".parse().unwrap(),
            "http://localhost:3000".parse().unwrap(),
        ])
        .allow_methods([Method::GET, Method::POST, Method::PUT, Method::DELETE])
        .allow_headers([
            axum::http::header::CONTENT_TYPE,
            axum::http::header::AUTHORIZATION,
        ])
        .allow_credentials(true);

    let maybe_verifier = JwtVerifier::from_env().map(Arc::new);

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
        .with_state(app_state)
        .merge(reporting_mutations)
        .merge(http::admin::admin_router(pool.clone()))
        .layer(DefaultBodyLimit::max(DEFAULT_BODY_LIMIT))
        .layer(axum::middleware::from_fn(security::tracing::tracing_context_middleware))
        .layer(axum::middleware::from_fn(timeout_middleware))
        .layer(axum::middleware::from_fn(rate_limit_middleware))
        .layer(Extension(default_rate_limiter()))
        .layer(axum::middleware::from_fn_with_state(maybe_verifier, optional_claims_mw))
        .layer(security::AuthzLayer::from_env())
        .layer(cors)
        .into_make_service_with_connect_info::<SocketAddr>();

    let addr: SocketAddr = format!("{}:{}", config.host, config.port)
        .parse()
        .expect("Invalid HOST:PORT");

    tracing::info!("Reporting service listening on {addr}");

    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .expect("Reporting: failed to bind address");

    axum::serve(listener, app)
        .await
        .expect("Reporting: failed to start server");
}
