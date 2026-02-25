use axum::{extract::DefaultBodyLimit, routing::get, Extension, Router};
use ::event_bus::{EventBus, InMemoryBus, NatsBus};
use notifications_rs::{
    config, config::Config, db, event_bus, consumer_tasks, metrics, routes,
    scheduled::{dispatch_once, reset_orphaned_claims, LoggingSender, NotificationSender},
};
use security::{
    middleware::{default_rate_limiter, rate_limit_middleware, timeout_middleware, DEFAULT_BODY_LIMIT},
    optional_claims_mw, JwtVerifier,
};
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tower_http::cors::{AllowOrigin, CorsLayer};
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

    // Startup: recover any notifications that were claimed but never completed
    // (e.g., process crashed mid-dispatch).
    {
        use chrono::Utc;
        let cutoff = Utc::now() - chrono::Duration::minutes(5);
        match reset_orphaned_claims(&db, cutoff).await {
            Ok(n) if n > 0 => tracing::warn!(count = n, "reset orphaned claimed notifications on startup"),
            Ok(_) => tracing::debug!("no orphaned claimed notifications found on startup"),
            Err(e) => tracing::error!(error = %e, "failed to reset orphaned claims on startup"),
        }
    }

    // Spawn background notification dispatcher loop.
    {
        let interval_secs: u64 = std::env::var("NOTIFICATIONS_DISPATCH_INTERVAL_SECS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(60);
        let dispatch_pool = db.clone();
        let dispatch_sender: Arc<dyn NotificationSender> = Arc::new(LoggingSender);
        tokio::spawn(async move {
            loop {
                if let Err(e) = dispatch_once(&dispatch_pool, dispatch_sender.clone()).await {
                    tracing::error!(error = %e, "dispatch_once error");
                }
                tokio::time::sleep(Duration::from_secs(interval_secs)).await;
            }
        });
    }

    // Spawn outbox publisher task
    tokio::spawn(event_bus::start_outbox_publisher(db.clone(), bus.clone()));

    // Spawn event consumer tasks
    consumer_tasks::start_invoice_issued_consumer(bus.clone(), db.clone()).await;
    consumer_tasks::start_payment_succeeded_consumer(bus.clone(), db.clone()).await;
    consumer_tasks::start_payment_failed_consumer(bus.clone(), db.clone()).await;

    // HTTP server configuration
    let maybe_verifier = JwtVerifier::from_env_with_overlap().map(Arc::new);

    let app = Router::new()
        .route("/healthz", get(health::healthz))
        .route("/api/health", get(routes::health::health))
        .route("/api/ready", get(routes::health::ready))
        .route("/api/version", get(routes::health::version))
        .route("/metrics", get(metrics::metrics_handler))
        .with_state(db.clone())
        .merge(routes::admin::admin_router(db))
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
    tracing::info!("Notifications module listening on {}", addr);

    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .expect("Failed to bind");

    axum::serve(listener, app)
        .await
        .expect("Server failed to start");
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

    #[test]
    fn cors_wildcard_parses() {
        let config = Config {
            database_url: "postgresql://localhost/test".to_string(),
            bus_type: config::BusType::InMemory,
            nats_url: None,
            host: "0.0.0.0".to_string(),
            port: 8089,
            cors_origins: vec!["*".to_string()],
        };
        let _layer = build_cors_layer(&config);
    }

    #[test]
    fn cors_specific_origins_parse() {
        let config = Config {
            database_url: "postgresql://localhost/test".to_string(),
            bus_type: config::BusType::InMemory,
            nats_url: None,
            host: "0.0.0.0".to_string(),
            port: 8089,
            cors_origins: vec![
                "http://localhost:3000".to_string(),
                "https://app.example.com".to_string(),
            ],
        };
        let _layer = build_cors_layer(&config);
    }
}
