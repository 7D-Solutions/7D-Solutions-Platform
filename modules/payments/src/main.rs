use axum::{extract::DefaultBodyLimit, routing::{get, post}, Extension, Router};
use event_bus::{EventBus, InMemoryBus};
use security::{
    middleware::{default_rate_limiter, rate_limit_middleware, timeout_middleware, DEFAULT_BODY_LIMIT},
    optional_claims_mw, permissions, JwtVerifier, RequirePermissionsLayer,
};
use std::net::SocketAddr;
use std::sync::Arc;
use tower_http::cors::{AllowOrigin, CorsLayer};
use tracing_subscriber::EnvFilter;

mod db;

use payments_rs::Config;

/// Handler for /metrics endpoint
async fn metrics_handler() -> String {
    use prometheus_client::encoding::text::encode;

    // Encode prometheus-client metrics
    let registry = payments_rs::metrics::METRICS_REGISTRY.lock().unwrap();
    let mut buffer = String::new();
    encode(&mut buffer, &registry).unwrap();

    // Append standard prometheus metrics (projection + SLO metrics)
    use prometheus::{Encoder, TextEncoder};
    let encoder = TextEncoder::new();

    let projection_families = payments_rs::metrics::PROJECTION_METRICS.registry().gather();
    let mut prometheus_buffer = Vec::new();
    encoder.encode(&projection_families, &mut prometheus_buffer)
        .expect("Failed to encode projection metrics");

    // SLO metrics (latency, error rate, consumer lag)
    let slo_families = payments_rs::metrics::SLO_REGISTRY.gather();
    encoder.encode(&slo_families, &mut prometheus_buffer)
        .expect("Failed to encode SLO metrics");

    buffer.push_str("\n");
    buffer.push_str(&String::from_utf8(prometheus_buffer)
        .expect("Failed to convert metrics to UTF-8"));

    buffer
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
        eprintln!("Payments service cannot start without valid configuration.");
        std::process::exit(1);
    });

    tracing::info!(
        "Configuration loaded: bus_type={:?}, host={}, port={}",
        config.bus_type,
        config.host,
        config.port
    );

    // Database connection
    tracing::info!("Connecting to database...");
    let pool = db::resolver::resolve_pool(&config.database_url)
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
    let bus: Arc<dyn EventBus> = match config.bus_type {
        payments_rs::config::BusType::InMemory => {
            tracing::info!("Using InMemory event bus");
            Arc::new(InMemoryBus::new())
        }
        payments_rs::config::BusType::Nats => {
            let nats_url = config
                .nats_url
                .as_ref()
                .expect("NATS_URL must be set when BUS_TYPE=nats");
            tracing::info!("Connecting to NATS at {}", nats_url);
            let client = async_nats::connect(nats_url)
                .await
                .expect("Failed to connect to NATS");
            Arc::new(event_bus::NatsBus::new(client))
        }
    };

    // Spawn outbox publisher task
    let publisher_pool = pool.clone();
    let publisher_bus = bus.clone();
    tokio::spawn(async move {
        tracing::info!("Starting outbox publisher...");
        if let Err(e) = payments_rs::events::outbox::start_outbox_publisher(publisher_pool, publisher_bus).await {
            tracing::error!("Outbox publisher error: {}", e);
        }
    });

    // Spawn payment collection consumer task
    let consumer_pool = pool.clone();
    let consumer_bus = bus.clone();
    payments_rs::start_payment_collection_consumer(consumer_bus, consumer_pool).await;

    // Create application state
    let app_state = Arc::new(payments_rs::AppState {
        pool: pool.clone(),
        tilled_api_key: config.tilled_api_key.clone(),
        tilled_account_id: config.tilled_account_id.clone(),
        tilled_webhook_secret: config.tilled_webhook_secret.clone(),
        tilled_webhook_secret_prev: config.tilled_webhook_secret_prev.clone(),
    });

    let maybe_verifier = JwtVerifier::from_env_with_overlap().map(Arc::new);

    let app = Router::new()
        .route("/healthz", get(health::healthz))
        .route("/api/health", get(payments_rs::routes::health::health))
        .route("/api/ready", get(payments_rs::routes::health::ready))
        .route("/api/version", get(payments_rs::routes::health::version))
        .route("/metrics", get(metrics_handler))
        .route(
            "/api/payments/webhook/tilled",
            post(payments_rs::routes::checkout_sessions::tilled_webhook),
        )
        .merge(
            Router::new()
                // Checkout session endpoints (bd-ddsm, bd-x0rt)
                .route(
                    "/api/payments/checkout-sessions",
                    post(payments_rs::routes::checkout_sessions::create_checkout_session),
                )
                .route(
                    "/api/payments/checkout-sessions/{id}",
                    get(payments_rs::routes::checkout_sessions::get_checkout_session),
                )
                .route(
                    "/api/payments/checkout-sessions/{id}/present",
                    post(payments_rs::routes::checkout_sessions::present_checkout_session),
                )
                .route(
                    "/api/payments/checkout-sessions/{id}/status",
                    get(payments_rs::routes::checkout_sessions::poll_checkout_session_status),
                )
                .merge(payments_rs::routes::admin::admin_router(app_state.clone()))
                .route_layer(RequirePermissionsLayer::new(&[permissions::PAYMENTS_MUTATE])),
        )
        .with_state(app_state)
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
    tracing::info!("Payments module listening on {}", addr);

    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .expect("Failed to bind");

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .expect("Server failed to start");

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
}

#[cfg(test)]
mod tests {
    use super::*;
    use payments_rs::config::{BusType, PaymentsProvider};

    #[test]
    fn cors_wildcard_parses() {
        let config = Config {
            database_url: "postgresql://localhost/test".to_string(),
            bus_type: BusType::InMemory,
            nats_url: None,
            host: "0.0.0.0".to_string(),
            port: 8088,
            env: "development".to_string(),
            cors_origins: vec!["*".to_string()],
            payments_provider: PaymentsProvider::Mock,
            tilled_api_key: None,
            tilled_account_id: None,
            tilled_webhook_secret: None,
            tilled_webhook_secret_prev: None,
        };
        let _layer = build_cors_layer(&config);
    }

    #[test]
    fn cors_specific_origins_parse() {
        let config = Config {
            database_url: "postgresql://localhost/test".to_string(),
            bus_type: BusType::InMemory,
            nats_url: None,
            host: "0.0.0.0".to_string(),
            port: 8088,
            env: "development".to_string(),
            cors_origins: vec![
                "http://localhost:3000".to_string(),
                "https://app.example.com".to_string(),
            ],
            payments_provider: PaymentsProvider::Mock,
            tilled_api_key: None,
            tilled_account_id: None,
            tilled_webhook_secret: None,
            tilled_webhook_secret_prev: None,
        };
        let _layer = build_cors_layer(&config);
    }
}
