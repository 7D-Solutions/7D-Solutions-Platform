use axum::{extract::DefaultBodyLimit, routing::{get, post}, Extension, Router};
use event_bus::{EventBus, InMemoryBus, NatsBus};
use maintenance_rs::{config::Config, metrics, outbox, routes, AppState};
use security::{
    middleware::{default_rate_limiter, rate_limit_middleware, timeout_middleware, DEFAULT_BODY_LIMIT},
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

    let shutdown_pool = pool.clone();

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

    // Spawn scheduler tick loop
    let scheduler_pool = pool.clone();
    let scheduler_interval = config.scheduler_interval_secs;
    tokio::spawn(async move {
        maintenance_rs::domain::scheduler::run_scheduler_task(
            scheduler_pool,
            scheduler_interval,
        )
        .await;
    });
    tracing::info!(
        interval_secs = config.scheduler_interval_secs,
        "Maintenance: scheduler task started"
    );

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
        .merge(
            Router::new()
                // Asset endpoints
                .route(
                    "/api/maintenance/assets",
                    post(routes::assets::create_asset).get(routes::assets::list_assets),
                )
                .route(
                    "/api/maintenance/assets/{asset_id}",
                    get(routes::assets::get_asset).patch(routes::assets::update_asset),
                )
                // Meter type endpoints
                .route(
                    "/api/maintenance/meter-types",
                    post(routes::meters::create_meter_type).get(routes::meters::list_meter_types),
                )
                // Meter reading endpoints
                .route(
                    "/api/maintenance/assets/{asset_id}/readings",
                    post(routes::meters::record_reading).get(routes::meters::list_readings),
                )
                // Plan endpoints
                .route(
                    "/api/maintenance/plans",
                    post(routes::plans::create_plan).get(routes::plans::list_plans),
                )
                .route(
                    "/api/maintenance/plans/{plan_id}",
                    get(routes::plans::get_plan).patch(routes::plans::update_plan),
                )
                .route(
                    "/api/maintenance/plans/{plan_id}/assign",
                    post(routes::plans::assign_plan),
                )
                // Assignment endpoints
                .route(
                    "/api/maintenance/assignments",
                    get(routes::plans::list_assignments),
                )
                // Work order endpoints
                .route(
                    "/api/maintenance/work-orders",
                    post(routes::work_orders::create_work_order)
                        .get(routes::work_orders::list_work_orders),
                )
                .route(
                    "/api/maintenance/work-orders/{wo_id}",
                    get(routes::work_orders::get_work_order),
                )
                .route(
                    "/api/maintenance/work-orders/{wo_id}/transition",
                    axum::routing::patch(routes::work_orders::transition_work_order),
                )
                // Work order parts subresource
                .route(
                    "/api/maintenance/work-orders/{wo_id}/parts",
                    post(routes::work_order_parts::add_part)
                        .get(routes::work_order_parts::list_parts),
                )
                .route(
                    "/api/maintenance/work-orders/{wo_id}/parts/{part_id}",
                    axum::routing::delete(routes::work_order_parts::remove_part),
                )
                // Work order labor subresource
                .route(
                    "/api/maintenance/work-orders/{wo_id}/labor",
                    post(routes::work_order_labor::add_labor)
                        .get(routes::work_order_labor::list_labor),
                )
                .route(
                    "/api/maintenance/work-orders/{wo_id}/labor/{labor_id}",
                    axum::routing::delete(routes::work_order_labor::remove_labor),
                )
                .route_layer(RequirePermissionsLayer::new(&[
                    permissions::MAINTENANCE_MUTATE,
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

    tracing::info!("Maintenance module listening on {}", addr);

    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .expect("Maintenance: failed to bind address");

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .expect("Maintenance: failed to start server");

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
    use maintenance_rs::config::BusType;

    #[test]
    fn cors_wildcard_parses() {
        let config = Config {
            database_url: "postgresql://localhost/test".to_string(),
            bus_type: BusType::InMemory,
            nats_url: None,
            host: "0.0.0.0".to_string(),
            port: 8101,
            env: "development".to_string(),
            cors_origins: vec!["*".to_string()],
            scheduler_interval_secs: 60,
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
            port: 8101,
            env: "development".to_string(),
            cors_origins: vec![
                "http://localhost:3000".to_string(),
                "https://app.example.com".to_string(),
            ],
            scheduler_interval_secs: 60,
        };
        let _layer = build_cors_layer(&config);
    }
}
