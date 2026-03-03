use ::event_bus::{EventBus, InMemoryBus, NatsBus};
use axum::{extract::DefaultBodyLimit, routing::get, Extension, Router};
use notifications_rs::{
    config,
    config::{Config, EmailSenderType, SmsSenderType},
    consumer_tasks, db, event_bus, http, metrics,
    scheduled::{
        dispatch_once, reset_orphaned_claims, ChannelRouter, HttpEmailSender, HttpSmsSender,
        LoggingSender, NotificationSender, RetryPolicy,
    },
};
use security::{
    middleware::{
        default_rate_limiter, rate_limit_middleware, timeout_middleware, DEFAULT_BODY_LIMIT,
    },
    optional_claims_mw, permissions, JwtVerifier, RequirePermissionsLayer,
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

    let shutdown_pool = db.clone();

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
            Ok(n) if n > 0 => {
                tracing::warn!(count = n, "reset orphaned claimed notifications on startup")
            }
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
        let retry_policy = RetryPolicy {
            max_attempts: config.retry_max_attempts,
            backoff_base_secs: config.retry_backoff_base_secs,
            backoff_multiplier: config.retry_backoff_multiplier,
            backoff_max_secs: config.retry_backoff_max_secs,
        };
        let email_sender: Arc<dyn NotificationSender> = match config.email_sender_type {
            EmailSenderType::Logging => Arc::new(LoggingSender),
            EmailSenderType::Http => Arc::new(HttpEmailSender::new(
                config
                    .email_http_endpoint
                    .clone()
                    .expect("EMAIL_HTTP_ENDPOINT required for HTTP sender"),
                config.email_from.clone(),
                config.email_api_key.clone(),
            )),
        };
        let sms_sender: Arc<dyn NotificationSender> = match config.sms_sender_type {
            SmsSenderType::Logging => Arc::new(LoggingSender),
            SmsSenderType::Http => Arc::new(HttpSmsSender::new(
                config
                    .sms_http_endpoint
                    .clone()
                    .expect("SMS_HTTP_ENDPOINT required for HTTP SMS sender"),
                config.sms_from_number.clone(),
                config.sms_api_key.clone(),
            )),
        };
        let dispatch_sender: Arc<dyn NotificationSender> = Arc::new(ChannelRouter {
            email: email_sender,
            sms: sms_sender,
        });
        tokio::spawn(async move {
            loop {
                if let Err(e) =
                    dispatch_once(&dispatch_pool, dispatch_sender.clone(), retry_policy).await
                {
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
        .route("/ready", get(http::health::ready))
        .route("/api/health", get(http::health::health))
        .route("/api/ready", get(http::health::ready))
        .route("/api/version", get(http::health::version))
        .route("/metrics", get(metrics::metrics_handler))
        .with_state(db.clone())
        .merge(
            http::admin::admin_router(db.clone()).route_layer(RequirePermissionsLayer::new(&[
                permissions::NOTIFICATIONS_MUTATE,
            ])),
        )
        .merge(
            http::dlq::dlq_read_router(db.clone()).route_layer(RequirePermissionsLayer::new(&[
                permissions::NOTIFICATIONS_READ,
            ])),
        )
        .merge(
            http::dlq::dlq_mutate_router(db.clone()).route_layer(RequirePermissionsLayer::new(&[
                permissions::NOTIFICATIONS_MUTATE,
            ])),
        )
        .merge(
            http::inbox::inbox_read_router(db.clone()).route_layer(
                RequirePermissionsLayer::new(&[permissions::NOTIFICATIONS_READ]),
            ),
        )
        .merge(
            http::inbox::inbox_mutate_router(db).route_layer(RequirePermissionsLayer::new(&[
                permissions::NOTIFICATIONS_MUTATE,
            ])),
        )
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
    tracing::info!("Notifications module listening on {}", addr);

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
            bus_type: config::BusType::InMemory,
            nats_url: None,
            host: "0.0.0.0".to_string(),
            port: 8089,
            env: "development".to_string(),
            cors_origins: vec!["*".to_string()],
            email_sender_type: config::EmailSenderType::Logging,
            email_http_endpoint: None,
            email_from: "no-reply@notifications.local".to_string(),
            email_api_key: None,
            sms_sender_type: config::SmsSenderType::Logging,
            sms_http_endpoint: None,
            sms_from_number: "+10000000000".to_string(),
            sms_api_key: None,
            retry_max_attempts: 5,
            retry_backoff_base_secs: 300,
            retry_backoff_multiplier: 1.0,
            retry_backoff_max_secs: 3600,
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
            env: "development".to_string(),
            cors_origins: vec![
                "http://localhost:3000".to_string(),
                "https://app.example.com".to_string(),
            ],
            email_sender_type: config::EmailSenderType::Logging,
            email_http_endpoint: None,
            email_from: "no-reply@notifications.local".to_string(),
            email_api_key: None,
            sms_sender_type: config::SmsSenderType::Logging,
            sms_http_endpoint: None,
            sms_from_number: "+10000000000".to_string(),
            sms_api_key: None,
            retry_max_attempts: 5,
            retry_backoff_base_secs: 300,
            retry_backoff_multiplier: 1.0,
            retry_backoff_max_secs: 3600,
        };
        let _layer = build_cors_layer(&config);
    }
}
