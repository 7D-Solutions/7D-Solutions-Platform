use axum::{extract::DefaultBodyLimit, routing::get, Extension, Router};
use security::{
    middleware::{default_rate_limiter, rate_limit_middleware, timeout_middleware, DEFAULT_BODY_LIMIT},
    optional_claims_mw, permissions, JwtVerifier, RequirePermissionsLayer,
};
use std::net::SocketAddr;
use std::sync::Arc;
use tower_http::cors::CorsLayer;
use tracing_subscriber::EnvFilter;

use inventory_rs::{
    db::resolver::resolve_pool,
    http::{
        cycle_counts::{post_cycle_count_approve, post_cycle_count_submit, post_cycle_count_task},
        history::get_movement_history,
        lots::get_lots_for_item,
        reorder::{get_reorder_policy, list_reorder_policies, post_reorder_policy, put_reorder_policy},
        serials::get_serials_for_item,
        status::post_status_transfer,
        trace::{trace_lot_handler, trace_serial_handler},
        valuation::{get_valuation_snapshot, list_valuation_snapshots, post_valuation_snapshot},
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
        reservations::{post_fulfill, post_release, post_reserve},
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

    let admin_pool = pool.clone();
    let app_state = Arc::new(AppState {
        pool,
        metrics,
    });

    let maybe_verifier = JwtVerifier::from_env().map(Arc::new);

    let inv_mutations = Router::new()
        // Item master — write
        .route("/api/inventory/items", axum::routing::post(create_item))
        .route("/api/inventory/items/{id}", axum::routing::put(update_item))
        .route("/api/inventory/items/{id}/deactivate", axum::routing::post(deactivate_item))
        // Stock receipts — write
        .route("/api/inventory/receipts", axum::routing::post(post_receipt))
        // Stock issues — write
        .route("/api/inventory/issues", axum::routing::post(post_issue))
        // Reservations — write
        .route("/api/inventory/reservations/reserve", axum::routing::post(post_reserve))
        .route("/api/inventory/reservations/release", axum::routing::post(post_release))
        .route("/api/inventory/reservations/{id}/fulfill", axum::routing::post(post_fulfill))
        // UoM — write
        .route("/api/inventory/uoms", axum::routing::post(create_uom))
        .route("/api/inventory/items/{id}/uom-conversions", axum::routing::post(create_conversion))
        // Stock adjustments — write
        .route("/api/inventory/adjustments", axum::routing::post(post_adjustment))
        // Transfers — write
        .route("/api/inventory/transfers", axum::routing::post(post_transfer))
        .route("/api/inventory/status-transfers", axum::routing::post(post_status_transfer))
        // Cycle counts — write
        .route("/api/inventory/cycle-count-tasks", axum::routing::post(post_cycle_count_task))
        .route(
            "/api/inventory/cycle-count-tasks/{task_id}/submit",
            axum::routing::post(post_cycle_count_submit),
        )
        .route(
            "/api/inventory/cycle-count-tasks/{task_id}/approve",
            axum::routing::post(post_cycle_count_approve),
        )
        // Reorder policies — write
        .route("/api/inventory/reorder-policies", axum::routing::post(post_reorder_policy))
        .route("/api/inventory/reorder-policies/{id}", axum::routing::put(put_reorder_policy))
        // Valuation snapshots — write
        .route("/api/inventory/valuation-snapshots", axum::routing::post(post_valuation_snapshot))
        // Locations — write
        .route("/api/inventory/locations", axum::routing::post(create_location))
        .route("/api/inventory/locations/{id}", axum::routing::put(update_location))
        .route("/api/inventory/locations/{id}/deactivate", axum::routing::post(deactivate_location))
        .route_layer(RequirePermissionsLayer::new(&[permissions::INVENTORY_MUTATE]))
        .with_state(app_state.clone());

    let app = Router::new()
        .route("/healthz", get(health::healthz))
        .route("/api/health", get(health))
        .route("/api/ready", get(ready))
        .route("/api/version", get(version))
        .route("/metrics", get(metrics_handler))
        // Item master — read
        .route("/api/inventory/items/{id}", axum::routing::get(get_item))
        // UoM — read
        .route("/api/inventory/uoms", axum::routing::get(list_uoms))
        .route("/api/inventory/items/{id}/uom-conversions", axum::routing::get(list_conversions))
        // Movement history — read
        .route("/api/inventory/items/{item_id}/history", axum::routing::get(get_movement_history))
        // Lot / serial queries — read
        .route("/api/inventory/items/{item_id}/lots", axum::routing::get(get_lots_for_item))
        .route("/api/inventory/items/{item_id}/serials", axum::routing::get(get_serials_for_item))
        .route(
            "/api/inventory/items/{item_id}/lots/{lot_code}/trace",
            axum::routing::get(trace_lot_handler),
        )
        .route(
            "/api/inventory/items/{item_id}/serials/{serial_code}/trace",
            axum::routing::get(trace_serial_handler),
        )
        // Reorder policies — read
        .route("/api/inventory/reorder-policies/{id}", axum::routing::get(get_reorder_policy))
        .route(
            "/api/inventory/items/{item_id}/reorder-policies",
            axum::routing::get(list_reorder_policies),
        )
        // Valuation snapshots — read
        .route("/api/inventory/valuation-snapshots", axum::routing::get(list_valuation_snapshots))
        .route(
            "/api/inventory/valuation-snapshots/{id}",
            axum::routing::get(get_valuation_snapshot),
        )
        // Locations — read
        .route("/api/inventory/locations/{id}", axum::routing::get(get_location))
        .route(
            "/api/inventory/warehouses/{warehouse_id}/locations",
            axum::routing::get(list_locations),
        )
        .with_state(app_state)
        .merge(inv_mutations)
        .merge(inventory_rs::http::admin::admin_router(admin_pool))
        .layer(DefaultBodyLimit::max(DEFAULT_BODY_LIMIT))
        .layer(axum::middleware::from_fn(security::tracing::tracing_context_middleware))
        .layer(axum::middleware::from_fn(timeout_middleware))
        .layer(axum::middleware::from_fn(rate_limit_middleware))
        .layer(Extension(default_rate_limiter()))
        .layer(axum::middleware::from_fn_with_state(maybe_verifier, optional_claims_mw))
        .layer(security::AuthzLayer::from_env())
        .layer(
            CorsLayer::new()
                .allow_origin(tower_http::cors::Any)
                .allow_methods(tower_http::cors::Any)
                .allow_headers(tower_http::cors::Any),
        )
        .into_make_service_with_connect_info::<SocketAddr>();

    let addr = SocketAddr::from(([0, 0, 0, 0], config.port));
    tracing::info!("Inventory service listening on {}", addr);

    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .expect("Failed to bind address");

    axum::serve(listener, app)
        .await
        .expect("Server failed to start");
}
