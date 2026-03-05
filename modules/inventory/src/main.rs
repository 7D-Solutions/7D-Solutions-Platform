use axum::{extract::DefaultBodyLimit, routing::get, Extension, Router};
use security::{
    middleware::{
        default_rate_limiter, rate_limit_middleware, timeout_middleware, DEFAULT_BODY_LIMIT,
    },
    optional_claims_mw, permissions, JwtVerifier, RequirePermissionsLayer,
};
use std::net::SocketAddr;
use std::sync::Arc;
use tower_http::cors::{AllowOrigin, CorsLayer};
use tracing_subscriber::EnvFilter;

use inventory_rs::{
    db::resolver::resolve_pool,
    http::{
        adjustments::post_adjustment,
        health::{health, ready, version},
        issues::post_issue,
        items::{create_item, deactivate_item, get_item, update_item},
        make_buy::put_make_buy,
        locations::{
            create_location, deactivate_location, get_location, list_locations, update_location,
        },
        receipts::post_receipt,
        reservations::{post_fulfill, post_release, post_reserve},
        transfers::post_transfer,
        uom::{create_conversion, create_uom, list_conversions, list_uoms},
    },
    http::{
        cycle_counts::{post_cycle_count_approve, post_cycle_count_submit, post_cycle_count_task},
        expiry::{post_expiry_alert_scan, put_lot_expiry},
        genealogy::{get_lot_children, get_lot_parents, post_lot_merge, post_lot_split},
        history::get_movement_history,
        labels::{get_label_by_id, get_list_labels, post_generate_label},
        lots::get_lots_for_item,
        reorder::{
            get_reorder_policy, list_reorder_policies, post_reorder_policy, put_reorder_policy,
        },
        revisions::{
            get_list_revisions, get_revision_at, post_activate_revision, post_create_revision,
            put_revision_policy,
        },
        serials::get_serials_for_item,
        status::post_status_transfer,
        trace::{trace_lot_handler, trace_serial_handler},
        valuation::{get_valuation_snapshot, list_valuation_snapshots, post_valuation_snapshot},
    },
    metrics::{metrics_handler, InventoryMetrics},
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

    let shutdown_pool = pool.clone();

    let metrics = Arc::new(InventoryMetrics::new().expect("Failed to create metrics registry"));

    let admin_pool = pool.clone();
    let app_state = Arc::new(AppState { pool, metrics });

    let maybe_verifier = JwtVerifier::from_env_with_overlap().map(Arc::new);

    let inv_mutations = Router::new()
        // Item master — write
        .route("/api/inventory/items", axum::routing::post(create_item))
        .route("/api/inventory/items/{id}", axum::routing::put(update_item))
        .route(
            "/api/inventory/items/{id}/deactivate",
            axum::routing::post(deactivate_item),
        )
        .route(
            "/api/inventory/items/{id}/make-buy",
            axum::routing::put(put_make_buy),
        )
        // Stock receipts — write
        .route("/api/inventory/receipts", axum::routing::post(post_receipt))
        // Stock issues — write
        .route("/api/inventory/issues", axum::routing::post(post_issue))
        // Reservations — write
        .route(
            "/api/inventory/reservations/reserve",
            axum::routing::post(post_reserve),
        )
        .route(
            "/api/inventory/reservations/release",
            axum::routing::post(post_release),
        )
        .route(
            "/api/inventory/reservations/{id}/fulfill",
            axum::routing::post(post_fulfill),
        )
        // UoM — write
        .route("/api/inventory/uoms", axum::routing::post(create_uom))
        .route(
            "/api/inventory/items/{id}/uom-conversions",
            axum::routing::post(create_conversion),
        )
        // Stock adjustments — write
        .route(
            "/api/inventory/adjustments",
            axum::routing::post(post_adjustment),
        )
        // Transfers — write
        .route(
            "/api/inventory/transfers",
            axum::routing::post(post_transfer),
        )
        .route(
            "/api/inventory/status-transfers",
            axum::routing::post(post_status_transfer),
        )
        // Cycle counts — write
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
        // Reorder policies — write
        .route(
            "/api/inventory/reorder-policies",
            axum::routing::post(post_reorder_policy),
        )
        .route(
            "/api/inventory/reorder-policies/{id}",
            axum::routing::put(put_reorder_policy),
        )
        // Valuation snapshots — write
        .route(
            "/api/inventory/valuation-snapshots",
            axum::routing::post(post_valuation_snapshot),
        )
        // Item revisions — write
        .route(
            "/api/inventory/items/{item_id}/revisions",
            axum::routing::post(post_create_revision),
        )
        .route(
            "/api/inventory/items/{item_id}/revisions/{revision_id}/activate",
            axum::routing::post(post_activate_revision),
        )
        .route(
            "/api/inventory/items/{item_id}/revisions/{revision_id}/policy-flags",
            axum::routing::put(put_revision_policy),
        )
        // Labels — write
        .route(
            "/api/inventory/items/{item_id}/labels",
            axum::routing::post(post_generate_label),
        )
        // Expiry — write
        .route(
            "/api/inventory/lots/{lot_id}/expiry",
            axum::routing::put(put_lot_expiry),
        )
        .route(
            "/api/inventory/expiry-alerts/scan",
            axum::routing::post(post_expiry_alert_scan),
        )
        // Lot genealogy — write
        .route(
            "/api/inventory/lots/split",
            axum::routing::post(post_lot_split),
        )
        .route(
            "/api/inventory/lots/merge",
            axum::routing::post(post_lot_merge),
        )
        // Locations — write
        .route(
            "/api/inventory/locations",
            axum::routing::post(create_location),
        )
        .route(
            "/api/inventory/locations/{id}",
            axum::routing::put(update_location),
        )
        .route(
            "/api/inventory/locations/{id}/deactivate",
            axum::routing::post(deactivate_location),
        )
        .route_layer(RequirePermissionsLayer::new(&[
            permissions::INVENTORY_MUTATE,
        ]))
        .with_state(app_state.clone());

    let inv_reads = Router::new()
        // Item master — read
        .route("/api/inventory/items/{id}", axum::routing::get(get_item))
        // UoM — read
        .route("/api/inventory/uoms", axum::routing::get(list_uoms))
        .route(
            "/api/inventory/items/{id}/uom-conversions",
            axum::routing::get(list_conversions),
        )
        // Movement history — read
        .route(
            "/api/inventory/items/{item_id}/history",
            axum::routing::get(get_movement_history),
        )
        // Lot / serial queries — read
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
        // Item revisions — read
        .route(
            "/api/inventory/items/{item_id}/revisions/at",
            axum::routing::get(get_revision_at),
        )
        .route(
            "/api/inventory/items/{item_id}/revisions",
            axum::routing::get(get_list_revisions),
        )
        // Labels — read
        .route(
            "/api/inventory/items/{item_id}/labels",
            axum::routing::get(get_list_labels),
        )
        .route(
            "/api/inventory/labels/{label_id}",
            axum::routing::get(get_label_by_id),
        )
        // Reorder policies — read
        .route(
            "/api/inventory/reorder-policies/{id}",
            axum::routing::get(get_reorder_policy),
        )
        .route(
            "/api/inventory/items/{item_id}/reorder-policies",
            axum::routing::get(list_reorder_policies),
        )
        // Valuation snapshots — read
        .route(
            "/api/inventory/valuation-snapshots",
            axum::routing::get(list_valuation_snapshots),
        )
        .route(
            "/api/inventory/valuation-snapshots/{id}",
            axum::routing::get(get_valuation_snapshot),
        )
        // Lot genealogy — read
        .route(
            "/api/inventory/lots/{lot_id}/children",
            axum::routing::get(get_lot_children),
        )
        .route(
            "/api/inventory/lots/{lot_id}/parents",
            axum::routing::get(get_lot_parents),
        )
        // Locations — read
        .route(
            "/api/inventory/locations/{id}",
            axum::routing::get(get_location),
        )
        .route(
            "/api/inventory/warehouses/{warehouse_id}/locations",
            axum::routing::get(list_locations),
        )
        .route_layer(RequirePermissionsLayer::new(&[permissions::INVENTORY_READ]))
        .with_state(app_state.clone());

    let app = Router::new()
        .route("/healthz", get(health::healthz))
        .route("/api/health", get(health))
        .route("/api/ready", get(ready))
        .route("/api/version", get(version))
        .route("/metrics", get(metrics_handler))
        .with_state(app_state)
        .merge(inv_reads)
        .merge(inv_mutations)
        .merge(inventory_rs::http::admin::admin_router(admin_pool))
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
    tracing::info!("Inventory service listening on {}", addr);

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
            host: "0.0.0.0".to_string(),
            port: 8092,
            env: "development".to_string(),
            cors_origins: vec!["*".to_string()],
        };
        let _layer = build_cors_layer(&config);
    }

    #[test]
    fn cors_specific_origins_parse() {
        let config = Config {
            database_url: "postgresql://localhost/test".to_string(),
            host: "0.0.0.0".to_string(),
            port: 8092,
            env: "development".to_string(),
            cors_origins: vec![
                "http://localhost:3000".to_string(),
                "https://app.example.com".to_string(),
            ],
        };
        let _layer = build_cors_layer(&config);
    }
}
