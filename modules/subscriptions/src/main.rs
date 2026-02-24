mod admin;
mod config;
mod db;
mod consumer;
mod cycle_gating;
mod dlq;
mod envelope;
mod envelope_validation;
mod gated_invoice_creation;
mod invariants;
mod lifecycle;
mod metrics;
mod models;
mod outbox;
mod publisher;
mod routes;

use axum::{extract::DefaultBodyLimit, routing::get, Extension, Router};
use config::Config;
use event_bus::{EventBus, InMemoryBus, NatsBus};
use security::{
    middleware::{default_rate_limiter, rate_limit_middleware, timeout_middleware, DEFAULT_BODY_LIMIT},
    optional_claims_mw, permissions, JwtVerifier, RequirePermissionsLayer,
};

use std::net::SocketAddr;
use std::sync::Arc;
use tower_http::cors::{AllowOrigin, CorsLayer};
use tracing_subscriber::EnvFilter;

/// Application state shared across HTTP handlers
pub struct AppState {
    pub pool: sqlx::PgPool,
    pub metrics: Arc<metrics::SubscriptionsMetrics>,
}

#[tokio::main]
async fn main() {
    dotenvy::dotenv().ok();

    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();

    // Load and validate configuration (fail-fast on missing/invalid config)
    let config = Config::from_env().unwrap_or_else(|err| {
        eprintln!("Configuration error: {}", err);
        eprintln!("Subscriptions service cannot start without valid configuration.");
        std::process::exit(1);
    });

    tracing::info!("Configuration loaded: {:?}", config.bus_type);

    // Initialize database pool
    let pool = db::resolver::resolve_pool(&config.database_url)
        .await
        .expect("Failed to connect to database");

    tracing::info!("Database connection established");

    // Run migrations
    sqlx::migrate!("./migrations")
        .run(&pool)
        .await
        .expect("Failed to run migrations");

    tracing::info!("Database migrations completed");

    // Initialize event bus
    let bus: Arc<dyn EventBus> = match config.bus_type {
        config::BusType::Nats => {
            let nats_url = config.nats_url.as_ref().expect("NATS_URL required for NATS bus");
            tracing::info!("Connecting to NATS at {}", nats_url);
            let nats_client = async_nats::connect(nats_url)
                .await
                .expect("Failed to connect to NATS");
            Arc::new(NatsBus::new(nats_client))
        }
        config::BusType::InMemory => {
            tracing::info!("Using in-memory event bus");
            Arc::new(InMemoryBus::new())
        }
    };

    // Spawn background publisher task
    let publisher_pool = pool.clone();
    let publisher_bus = bus.clone();
    tokio::spawn(async move {
        publisher::run_publisher(publisher_pool, publisher_bus).await;
    });

    tracing::info!("Background event publisher started");

    // Initialize metrics
    let metrics = Arc::new(metrics::SubscriptionsMetrics::new().expect("Failed to create metrics"));
    tracing::info!("Metrics initialized");

    // Create application state
    let app_state = Arc::new(AppState {
        pool: pool.clone(),
        metrics: metrics.clone(),
    });

    let host = std::env::var("HOST").unwrap_or_else(|_| "0.0.0.0".to_string());
    let port: u16 = std::env::var("PORT")
        .unwrap_or_else(|_| "8087".to_string())
        .parse()
        .expect("PORT must be a valid u16");

    let maybe_verifier = JwtVerifier::from_env_with_overlap().map(Arc::new);

    let app = Router::new()
        .route("/healthz", get(health::healthz))
        .route("/api/health", get(routes::health))
        .route("/api/ready", get(routes::ready))
        .route("/api/version", get(routes::version))
        .route("/metrics", get(metrics::metrics_handler))
        .with_state(app_state.clone())
        .merge(
            routes::subscriptions_router(pool.clone())
                .route_layer(RequirePermissionsLayer::new(&[permissions::SUBSCRIPTIONS_MUTATE])),
        )
        .merge(admin::admin_router(pool))
        .layer(DefaultBodyLimit::max(DEFAULT_BODY_LIMIT))
        .layer(axum::middleware::from_fn(security::tracing::tracing_context_middleware))
        .layer(axum::middleware::from_fn(timeout_middleware))
        .layer(axum::middleware::from_fn(rate_limit_middleware))
        .layer(Extension(default_rate_limiter()))
        .layer(axum::middleware::from_fn_with_state(maybe_verifier, optional_claims_mw))
        .layer(build_cors_layer(&config))
        .into_make_service_with_connect_info::<SocketAddr>();

    let addr = SocketAddr::from(([0, 0, 0, 0], port));
    tracing::info!("Subscriptions module listening on {}", addr);

    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .expect("Failed to bind");

    axum::serve(listener, app)
        .await
        .expect("Server failed to start");
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cors_wildcard_parses() {
        let config = Config {
            bus_type: config::BusType::InMemory,
            database_url: "postgresql://localhost/test".to_string(),
            nats_url: None,
            cors_origins: vec!["*".to_string()],
        };
        let _layer = build_cors_layer(&config);
    }

    #[test]
    fn cors_specific_origins_parse() {
        let config = Config {
            bus_type: config::BusType::InMemory,
            database_url: "postgresql://localhost/test".to_string(),
            nats_url: None,
            cors_origins: vec![
                "http://localhost:3000".to_string(),
                "https://app.example.com".to_string(),
            ],
        };
        let _layer = build_cors_layer(&config);
    }
}
