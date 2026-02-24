use axum::{extract::DefaultBodyLimit, routing::get, Extension, Router};
use event_bus::{EventBus, InMemoryBus, NatsBus};
use maintenance_rs::{config::Config, metrics, outbox, routes, AppState};
use security::{
    middleware::{default_rate_limiter, rate_limit_middleware, timeout_middleware, DEFAULT_BODY_LIMIT},
    optional_claims_mw, JwtVerifier,
};
use std::net::SocketAddr;
use std::sync::Arc;
use tower_http::cors::CorsLayer;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() {
    dotenvy::dotenv().ok();

    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();

    let config = Config::from_env().unwrap_or_else(|err| {
        eprintln!("Configuration error: {}", err);
        eprintln!("Maintenance service cannot start without valid configuration.");
        std::process::exit(1);
    });

    tracing::info!(
        "Maintenance: config loaded: bus_type={:?}, host={}, port={}",
        config.bus_type,
        config.host,
        config.port
    );

    // Connect to database
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(10)
        .connect(&config.database_url)
        .await
        .expect("Maintenance: failed to connect to Postgres");

    // Run migrations
    sqlx::migrate!("./db/migrations")
        .run(&pool)
        .await
        .expect("Maintenance: failed to run database migrations");

    tracing::info!("Maintenance: database migrations applied");

    // Initialize event bus
    let event_bus: Arc<dyn EventBus> = match config.bus_type {
        maintenance_rs::config::BusType::Nats => {
            let nats_url = config
                .nats_url
                .as_ref()
                .expect("NATS_URL must be set when BUS_TYPE=nats");
            tracing::info!("Maintenance: connecting to NATS at {}", nats_url);
            let client = async_nats::connect(nats_url)
                .await
                .expect("Maintenance: failed to connect to NATS");
            Arc::new(NatsBus::new(client))
        }
        maintenance_rs::config::BusType::InMemory => {
            tracing::info!("Maintenance: using in-memory event bus");
            Arc::new(InMemoryBus::new())
        }
    };

    // Spawn outbox publisher loop
    let publisher_pool = pool.clone();
    let publisher_bus = event_bus.clone();
    tokio::spawn(async move {
        outbox::run_publisher_task(publisher_pool, publisher_bus).await;
    });
    tracing::info!("Maintenance: outbox publisher task started");

    // Initialize metrics
    let app_metrics = Arc::new(
        metrics::MaintenanceMetrics::new().expect("Maintenance: failed to create metrics"),
    );
    tracing::info!("Maintenance: metrics initialized");

    let app_state = Arc::new(AppState {
        pool: pool.clone(),
        metrics: app_metrics,
    });

    let maybe_verifier = JwtVerifier::from_env_with_overlap().map(Arc::new);

    let app = Router::new()
        .route("/healthz", get(health::healthz))
        .route("/api/health", get(routes::health::health))
        .route("/api/ready", get(routes::health::ready))
        .route("/api/version", get(routes::health::version))
        .route("/metrics", get(metrics::metrics_handler))
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
        .layer(security::AuthzLayer::from_env())
        .layer(
            CorsLayer::new()
                .allow_origin(tower_http::cors::Any)
                .allow_methods(tower_http::cors::Any)
                .allow_headers(tower_http::cors::Any),
        )
        .into_make_service_with_connect_info::<SocketAddr>();

    let addr: SocketAddr = format!("{}:{}", config.host, config.port)
        .parse()
        .expect("Invalid HOST:PORT");

    tracing::info!("Maintenance module listening on {}", addr);

    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .expect("Maintenance: failed to bind address");

    axum::serve(listener, app)
        .await
        .expect("Maintenance: failed to start server");
}
