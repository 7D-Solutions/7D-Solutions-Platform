use axum::{extract::DefaultBodyLimit, routing::get, Extension, Router};
use event_bus::{EventBus, InMemoryBus, NatsBus};
use security::middleware::{
    default_rate_limiter, rate_limit_middleware, timeout_middleware, DEFAULT_BODY_LIMIT,
};
use security::{optional_claims_mw, JwtVerifier};
use std::net::SocketAddr;
use std::sync::Arc;
use tower_http::cors::{AllowOrigin, CorsLayer};
use tracing_subscriber::EnvFilter;

use ar_rs::{config::Config, consumer_tasks, db, events::run_publisher_task, http, AppState};

#[tokio::main]
async fn main() {
    dotenvy::dotenv().ok();

    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();

    // Load and validate configuration (fail-fast on missing/invalid config)
    let config = Config::from_env().unwrap_or_else(|err| {
        eprintln!("Configuration error: {}", err);
        eprintln!("AR service cannot start without valid configuration.");
        std::process::exit(1);
    });

    tracing::info!(
        "Configuration loaded: bus_type={:?}, host={}, port={}",
        config.bus_type,
        config.host,
        config.port
    );

    // Resolve DB pool through centralized resolver (Phase 16: PDAA preparation)
    let db = db::resolve_pool(&config.database_url)
        .await
        .expect("Failed to connect to Postgres");

    let shutdown_pool = db.clone();

    sqlx::migrate!("./db/migrations")
        .run(&db)
        .await
        .expect("Failed to run database migrations");

    tracing::info!("Database migrations applied successfully");

    // Initialize event bus based on configuration
    let event_bus: Arc<dyn EventBus> = match config.bus_type {
        ar_rs::config::BusType::Nats => {
            let nats_url = config
                .nats_url
                .as_ref()
                .expect("NATS_URL must be set when BUS_TYPE=nats");
            tracing::info!("Connecting to NATS at {}", nats_url);
            let nats_client = event_bus::connect_nats(&nats_url)
                .await
                .expect("Failed to connect to NATS");
            Arc::new(NatsBus::new(nats_client))
        }
        ar_rs::config::BusType::InMemory => {
            tracing::info!("Using in-memory event bus");
            Arc::new(InMemoryBus::new())
        }
    };

    // Spawn background publisher task
    let publisher_db = db.clone();
    let publisher_bus = event_bus.clone();
    tokio::spawn(async move {
        run_publisher_task(publisher_db, publisher_bus).await;
    });

    tracing::info!("Event publisher task started");

    // Spawn event consumer tasks
    consumer_tasks::start_payment_succeeded_consumer(event_bus.clone(), db.clone()).await;

    tracing::info!("Event consumer tasks started");

    // Initialize metrics
    let metrics = Arc::new(ar_rs::metrics::ArMetrics::new().expect("Failed to create metrics"));
    tracing::info!("Metrics initialized");

    // Create application state
    let app_state = Arc::new(AppState {
        pool: db.clone(),
        metrics: metrics.clone(),
    });

    // Optional JWT verifier for claims extraction (requires JWT_PUBLIC_KEY env var).
    let maybe_verifier = JwtVerifier::from_env_with_overlap().map(Arc::new);

    let rate_limiter = default_rate_limiter();

    let app = Router::new()
        .route("/healthz", get(health::healthz))
        .route("/api/health", get(http::health::health))
        .route("/api/ready", get(http::health::ready))
        .route("/api/version", get(http::health::version))
        .route("/metrics", get(ar_rs::metrics::metrics_handler))
        .with_state(app_state.clone())
        .merge(http::ar_router(db.clone()))
        .merge(http::tax::tax_router(db.clone()))
        .merge(http::admin::admin_router(db))
        .layer(DefaultBodyLimit::max(DEFAULT_BODY_LIMIT))
        .layer(axum::middleware::from_fn(
            security::tracing::tracing_context_middleware,
        ))
        .layer(axum::middleware::from_fn(timeout_middleware))
        .layer(axum::middleware::from_fn(rate_limit_middleware))
        .layer(Extension(rate_limiter))
        .layer(axum::middleware::from_fn_with_state(
            maybe_verifier,
            optional_claims_mw,
        ))
        .layer(build_cors_layer(&config))
        .into_make_service_with_connect_info::<SocketAddr>();

    let addr: SocketAddr = format!("{}:{}", config.host, config.port)
        .parse()
        .expect("Invalid HOST:PORT");

    tracing::info!("AR service listening on {addr}");

    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .expect("Failed to bind address");

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .expect("Failed to start server");

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
        .allow_credentials(false)
}
