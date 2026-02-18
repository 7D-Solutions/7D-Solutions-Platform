use axum::{routing::get, Router};
use std::net::SocketAddr;
use std::sync::Arc;
use tower_http::cors::CorsLayer;
use tracing_subscriber::EnvFilter;

use inventory_rs::{
    db::resolver::resolve_pool,
    http::{
        cycle_counts::{post_cycle_count_approve, post_cycle_count_submit, post_cycle_count_task},
        lots::get_lots_for_item,
        reorder::{get_reorder_policy, list_reorder_policies, post_reorder_policy, put_reorder_policy},
        serials::get_serials_for_item,
        status::post_status_transfer,
        trace::{trace_lot_handler, trace_serial_handler},
        valuation::post_valuation_snapshot,
    },
    metrics::{metrics_handler, InventoryMetrics},
    routes::{
        adjustments::post_adjustment,
        health::{health, ready, version},
        issues::post_issue,
        items::{create_item, deactivate_item, get_item, update_item},
        locations::{
            create_location, deactivate_location, get_location, list_locations, update_location,
        },
        receipts::post_receipt,
        reservations::{post_release, post_reserve},
        transfers::post_transfer,
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
        // Stock adjustments (compensating ledger entries)
        .route(
            "/api/inventory/adjustments",
            axum::routing::post(post_adjustment),
        )
        // Inter-warehouse transfers (paired ledger entries, FIFO consumption)
        .route(
            "/api/inventory/transfers",
            axum::routing::post(post_transfer),
        )
        // Status bucket transfers (available ↔ quarantine ↔ damaged)
        .route(
            "/api/inventory/status-transfers",
            axum::routing::post(post_status_transfer),
        )
        // Lot / serial queries and traceability
        .route(
            "/api/inventory/items/{item_id}/lots",
            axum::routing::get(get_lots_for_item),
        )
        .route(
            "/api/inventory/items/{item_id}/serials",
            axum::routing::get(get_serials_for_item),
        )
        .route(
            "/api/inventory/items/{item_id}/lots/{lot_code}/trace",
            axum::routing::get(trace_lot_handler),
        )
        .route(
            "/api/inventory/items/{item_id}/serials/{serial_code}/trace",
            axum::routing::get(trace_serial_handler),
        )
        // Cycle count tasks
        .route(
            "/api/inventory/cycle-count-tasks",
            axum::routing::post(post_cycle_count_task),
        )
        .route(
            "/api/inventory/cycle-count-tasks/{task_id}/submit",
            axum::routing::post(post_cycle_count_submit),
        )
        .route(
            "/api/inventory/cycle-count-tasks/{task_id}/approve",
            axum::routing::post(post_cycle_count_approve),
        )
        // Reorder policies (low-stock configuration)
        .route(
            "/api/inventory/reorder-policies",
            axum::routing::post(post_reorder_policy),
        )
        .route(
            "/api/inventory/reorder-policies/{id}",
            axum::routing::get(get_reorder_policy).put(put_reorder_policy),
        )
        .route(
            "/api/inventory/items/{item_id}/reorder-policies",
            axum::routing::get(list_reorder_policies),
        )
        // Valuation snapshots (FIFO-derived point-in-time value)
        .route(
            "/api/inventory/valuation-snapshots",
            axum::routing::post(post_valuation_snapshot),
        )
        // Locations (warehouse bins/shelves)
        .route("/api/inventory/locations", axum::routing::post(create_location))
        .route(
            "/api/inventory/locations/{id}",
            axum::routing::get(get_location).put(update_location),
        )
        .route(
            "/api/inventory/locations/{id}/deactivate",
            axum::routing::post(deactivate_location),
        )
        .route(
            "/api/inventory/warehouses/{warehouse_id}/locations",
            axum::routing::get(list_locations),
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
