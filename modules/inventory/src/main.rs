use axum::{routing::get, Router};
use std::net::SocketAddr;
use std::sync::Arc;
use tower_http::cors::CorsLayer;
use tracing_subscriber::EnvFilter;

use inventory_rs::{
    db::resolver::resolve_pool,
    metrics::{metrics_handler, InventoryMetrics},
    routes::{
        health::{health, ready, version},
        issues::post_issue,
        items::{create_item, deactivate_item, get_item, update_item},
        receipts::post_receipt,
        reservations::{post_release, post_reserve},
        uom::{create_conversion, create_uom, list_conversions, list_uoms},
    },
    AppState, Config,
};

#[tokio::main]
async fn main() {
    dotenvy::dotenv().ok();

    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();

    tracing::info!("Starting Inventory service...");

    let config = Config::from_env().unwrap_or_else(|err| {
        eprintln!("Configuration error: {}", err);
        eprintln!("Inventory service cannot start without valid configuration.");
        std::process::exit(1);
    });

    tracing::info!(
        "Configuration loaded: host={}, port={}",
        config.host,
        config.port
    );

    tracing::info!("Connecting to database...");
    let pool = resolve_pool(&config.database_url)
        .await
        .expect("Failed to connect to database");

    let metrics = Arc::new(
        InventoryMetrics::new().expect("Failed to create metrics registry"),
    );

    let app_state = Arc::new(AppState {
        pool,
        metrics,
    });

    let app = Router::new()
        .route("/api/health", get(health))
        .route("/api/ready", get(ready))
        .route("/api/version", get(version))
        .route("/metrics", get(metrics_handler))
        // Item master
        .route("/api/inventory/items", axum::routing::post(create_item))
        .route(
            "/api/inventory/items/{id}",
            axum::routing::get(get_item).put(update_item),
        )
        .route(
            "/api/inventory/items/{id}/deactivate",
            axum::routing::post(deactivate_item),
        )
        // Stock receipts
        .route("/api/inventory/receipts", axum::routing::post(post_receipt))
        // Stock issues (FIFO consumption)
        .route("/api/inventory/issues", axum::routing::post(post_issue))
        // Reservations (compensating model: reserve + release)
        .route(
            "/api/inventory/reservations/reserve",
            axum::routing::post(post_reserve),
        )
        .route(
            "/api/inventory/reservations/release",
            axum::routing::post(post_release),
        )
        // UoM catalog + item conversions
        .route(
            "/api/inventory/uoms",
            axum::routing::post(create_uom).get(list_uoms),
        )
        .route(
            "/api/inventory/items/{id}/uom-conversions",
            axum::routing::post(create_conversion).get(list_conversions),
        )
        .with_state(app_state)
        .layer(
            CorsLayer::new()
                .allow_origin(tower_http::cors::Any)
                .allow_methods(tower_http::cors::Any)
                .allow_headers(tower_http::cors::Any),
        );

    let addr = SocketAddr::from(([0, 0, 0, 0], config.port));
    tracing::info!("Inventory service listening on {}", addr);

    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .expect("Failed to bind address");

    axum::serve(listener, app)
        .await
        .expect("Server failed to start");
}
