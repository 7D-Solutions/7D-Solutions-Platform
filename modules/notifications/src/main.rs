use axum::{extract::DefaultBodyLimit, routing::get, Extension, Router};
use ::event_bus::{EventBus, InMemoryBus, NatsBus};
use notifications_rs::{config, config::Config, db, event_bus, consumer_tasks, metrics, routes};
use security::middleware::{
    default_rate_limiter, rate_limit_middleware, timeout_middleware, DEFAULT_BODY_LIMIT,
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

    // Load and validate configuration (fail-fast on missing/invalid config)
    let config = Config::from_env().unwrap_or_else(|err| {
        eprintln!("Configuration error: {}", err);
        eprintln!("Notifications service cannot start without valid configuration.");
        std::process::exit(1);
    });

    tracing::info!(
        "Configuration loaded: bus_type={:?}, host={}, port={}",
        config.bus_type,
        config.host,
        config.port
    );

    // Database configuration
    let db = db::resolver::resolve_pool(&config.database_url)
        .await
        .expect("Failed to connect to Postgres");

    // Run migrations
    sqlx::migrate!("./db/migrations")
        .run(&db)
        .await
        .expect("Failed to run database migrations");

    tracing::info!("Database migrations applied successfully");

    // Event bus configuration
    let bus: Arc<dyn EventBus> = match config.bus_type {
        config::BusType::Nats => {
            let nats_url = config
                .nats_url
                .as_ref()
                .expect("NATS_URL must be set when BUS_TYPE=nats");
            tracing::info!("Connecting to NATS at {}", nats_url);
            let nats_client = async_nats::connect(nats_url)
                .await
                .expect("Failed to connect to NATS");
            Arc::new(NatsBus::new(nats_client))
        }
        config::BusType::InMemory => {
            tracing::info!("Using InMemoryBus for event messaging");
            Arc::new(InMemoryBus::new())
        }
    };

    // Spawn outbox publisher task
    tokio::spawn(event_bus::start_outbox_publisher(db.clone(), bus.clone()));

    // Spawn event consumer tasks
    consumer_tasks::start_invoice_issued_consumer(bus.clone(), db.clone()).await;
    consumer_tasks::start_payment_succeeded_consumer(bus.clone(), db.clone()).await;
    consumer_tasks::start_payment_failed_consumer(bus.clone(), db.clone()).await;

    // HTTP server configuration
    let app = Router::new()
        .route("/api/health", get(routes::health::health))
        .route("/api/ready", get(routes::health::ready))
        .route("/api/version", get(routes::health::version))
        .route("/metrics", get(metrics::metrics_handler))
        .with_state(db)
        .layer(DefaultBodyLimit::max(DEFAULT_BODY_LIMIT))
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

    let addr: SocketAddr = format!("{}:{}", config.host, config.port)
        .parse()
        .expect("Invalid HOST:PORT");
    tracing::info!("Notifications module listening on {}", addr);

    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .expect("Failed to bind");

    axum::serve(listener, app)
        .await
        .expect("Server failed to start");
}
