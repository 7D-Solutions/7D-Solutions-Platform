use axum::{extract::DefaultBodyLimit, routing::{get, post}, Extension, Router};
use security::{
    middleware::{
        default_rate_limiter, rate_limit_middleware, timeout_middleware, DEFAULT_BODY_LIMIT,
    },
    optional_claims_mw, JwtVerifier,
};
use std::net::SocketAddr;
use std::sync::Arc;
use tower_http::cors::{AllowOrigin, CorsLayer};
use tracing_subscriber::EnvFilter;

use production_rs::{
    db::resolver::resolve_pool,
    http::health::{health as health_fn, ready, version},
    http::component_issue,
    http::fg_receipt,
    http::operations,
    http::routings,
    http::work_orders,
    http::workcenters,
    metrics::{metrics_handler, ProductionMetrics},
    AppState, Config,
};

#[tokio::main]
async fn main() {
    dotenvy::dotenv().ok();

    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();

    tracing::info!("Starting Production service...");

    let config = Config::from_env().unwrap_or_else(|err| {
        eprintln!("Configuration error: {}", err);
        std::process::exit(1);
    });

    tracing::info!(
        "Configuration loaded: host={}, port={}",
        config.host,
        config.port
    );

    let pool = resolve_pool(&config.database_url)
        .await
        .expect("Failed to connect to database");

    sqlx::migrate!("db/migrations")
        .run(&pool)
        .await
        .expect("Failed to run database migrations");

    let shutdown_pool = pool.clone();
    let metrics = Arc::new(ProductionMetrics::new().expect("Failed to create metrics registry"));
    let app_state = Arc::new(AppState { pool, metrics });

    let maybe_verifier = JwtVerifier::from_env_with_overlap().map(Arc::new);

    let app = Router::new()
        .route("/healthz", get(health::healthz))
        .route("/api/health", get(health_fn))
        .route("/api/ready", get(ready))
        .route("/api/version", get(version))
        .route("/metrics", get(metrics_handler))
        .route("/api/production/workcenters", get(workcenters::list_workcenters).post(workcenters::create_workcenter))
        .route("/api/production/workcenters/{id}", get(workcenters::get_workcenter).put(workcenters::update_workcenter))
        .route("/api/production/workcenters/{id}/deactivate", post(workcenters::deactivate_workcenter))
        .route("/api/production/work-orders", post(work_orders::create_work_order))
        .route("/api/production/work-orders/{id}", get(work_orders::get_work_order))
        .route("/api/production/work-orders/{id}/release", post(work_orders::release_work_order))
        .route("/api/production/work-orders/{id}/close", post(work_orders::close_work_order))
        .route("/api/production/work-orders/{id}/component-issues", post(component_issue::post_component_issue))
        .route("/api/production/work-orders/{id}/fg-receipt", post(fg_receipt::post_fg_receipt))
        .route("/api/production/work-orders/{id}/operations", get(operations::list_operations))
        .route("/api/production/work-orders/{id}/operations/initialize", post(operations::initialize_operations))
        .route("/api/production/work-orders/{wo_id}/operations/{op_id}/start", post(operations::start_operation))
        .route("/api/production/work-orders/{wo_id}/operations/{op_id}/complete", post(operations::complete_operation))
        .route("/api/production/routings", get(routings::list_routings).post(routings::create_routing))
        .route("/api/production/routings/by-item", get(routings::find_routings_by_item))
        .route("/api/production/routings/{id}", get(routings::get_routing).put(routings::update_routing))
        .route("/api/production/routings/{id}/release", post(routings::release_routing))
        .route("/api/production/routings/{id}/steps", get(routings::list_routing_steps).post(routings::add_routing_step))
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

    let addr = SocketAddr::from(([0, 0, 0, 0], config.port));
    tracing::info!("Production service listening on {}", addr);

    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .expect("Failed to bind address");

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
