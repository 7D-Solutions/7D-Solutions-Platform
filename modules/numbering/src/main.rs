use axum::{
    extract::DefaultBodyLimit,
    routing::{get, post},
    Extension, Router,
};
use event_bus::{EventBus, InMemoryBus, NatsBus};
use numbering::{config::Config, http, metrics, outbox, AppState};
use security::{
    middleware::{
        default_rate_limiter, rate_limit_middleware, timeout_middleware, DEFAULT_BODY_LIMIT,
    },
    optional_claims_mw, permissions, JwtVerifier, RequirePermissionsLayer,
};
use std::net::SocketAddr;
use std::sync::Arc;
use tower_http::cors::{AllowOrigin, CorsLayer};
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() {
    dotenvy::dotenv().ok();

    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();

    let config = Config::from_env().unwrap_or_else(|err| {
        eprintln!("Configuration error: {}", err);
        std::process::exit(1);
    });

    tracing::info!(
        "Numbering: config loaded: bus_type={:?}, host={}, port={}",
        config.bus_type,
        config.host,
        config.port
    );

    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(10)
        .connect(&config.database_url)
        .await
        .expect("Numbering: failed to connect to Postgres");

    let shutdown_pool = pool.clone();

    sqlx::migrate!("./db/migrations")
        .run(&pool)
        .await
        .expect("Numbering: failed to run database migrations");

    tracing::info!("Numbering: database migrations applied");

    let event_bus: Arc<dyn EventBus> = match config.bus_type {
        numbering::config::BusType::Nats => {
            let nats_url = config
                .nats_url
                .as_ref()
                .expect("NATS_URL must be set when BUS_TYPE=nats");
            tracing::info!("Numbering: connecting to NATS at {}", nats_url);
            let client = async_nats::connect(nats_url)
                .await
                .expect("Numbering: failed to connect to NATS");
            Arc::new(NatsBus::new(client))
        }
        numbering::config::BusType::InMemory => {
            tracing::info!("Numbering: using in-memory event bus");
            Arc::new(InMemoryBus::new())
        }
    };

    let publisher_pool = pool.clone();
    let publisher_bus = event_bus.clone();
    tokio::spawn(async move {
        outbox::run_publisher_task(publisher_pool, publisher_bus).await;
    });
    tracing::info!("Numbering: outbox publisher task started");

    let app_metrics = Arc::new(
        metrics::NumberingMetrics::new().expect("Numbering: failed to create metrics"),
    );

    let app_state = Arc::new(AppState {
        pool: pool.clone(),
        metrics: app_metrics,
    });

    let maybe_verifier = JwtVerifier::from_env_with_overlap().map(Arc::new);

    let app = Router::new()
        .route("/healthz", get(health::healthz))
        .route("/api/health", get(http::health::health))
        .route("/api/ready", get(http::health::ready))
        .route("/api/version", get(http::health::version))
        .route("/api/schema-version", get(http::health::schema_version))
        .route("/metrics", get(metrics::metrics_handler))
        .merge(
            Router::new()
                .route("/allocate", post(http::allocate::allocate))
                .route_layer(RequirePermissionsLayer::new(&[
                    permissions::NUMBERING_ALLOCATE,
                ])),
        )
        .with_state(app_state)
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

    let addr: SocketAddr = format!("{}:{}", config.host, config.port)
        .parse()
        .expect("Invalid HOST:PORT");

    tracing::info!("Numbering module listening on {}", addr);

    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .expect("Numbering: failed to bind address");

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .expect("Numbering: failed to start server");

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
