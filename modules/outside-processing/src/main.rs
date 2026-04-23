use axum::{
    routing::{get, post, put},
    Router,
};
use std::sync::Arc;

use outside_processing_rs::{consumers, http, metrics, AppState};
use platform_sdk::ModuleBuilder;
use security::{permissions, RequirePermissionsLayer};

static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./db/migrations");

#[tokio::main]
async fn main() {
    ModuleBuilder::from_manifest("module.toml")
        .migrator(&MIGRATOR)
        .routes(|ctx| {
            let op_metrics =
                Arc::new(metrics::OpMetrics::new().expect("OP: failed to create metrics"));

            let state = Arc::new(AppState {
                pool: ctx.pool().clone(),
                metrics: op_metrics,
            });

            if let Ok(bus) = ctx.bus_arc() {
                consumers::shipment_shipped::start_consumer(bus.clone(), ctx.pool().clone());
                consumers::shipment_received::start_consumer(bus.clone(), ctx.pool().clone());
                consumers::ap_po_approved::start_consumer(bus.clone(), ctx.pool().clone());
                consumers::ap_po_closed::start_consumer(bus.clone(), ctx.pool().clone());
                consumers::inventory_lot_split::start_consumer(bus.clone(), ctx.pool().clone());
                tracing::info!("OP: all consumers started");
            }

            let mutations = Router::new()
                .route(
                    "/api/outside-processing/orders",
                    post(http::orders::create_order),
                )
                .route(
                    "/api/outside-processing/orders/{id}",
                    put(http::orders::update_order),
                )
                .route(
                    "/api/outside-processing/orders/{id}/issue",
                    post(http::orders::issue_order),
                )
                .route(
                    "/api/outside-processing/orders/{id}/cancel",
                    post(http::orders::cancel_order),
                )
                .route(
                    "/api/outside-processing/orders/{id}/close",
                    post(http::orders::close_order),
                )
                .route(
                    "/api/outside-processing/orders/{id}/ship-events",
                    post(http::ship_events::create_ship_event),
                )
                .route(
                    "/api/outside-processing/orders/{id}/return-events",
                    post(http::return_events::create_return_event),
                )
                .route(
                    "/api/outside-processing/orders/{id}/reviews",
                    post(http::reviews::create_review),
                )
                .route(
                    "/api/outside-processing/orders/{id}/re-identifications",
                    post(http::re_identifications::create_re_identification),
                )
                .route(
                    "/api/outside-processing/status-labels/{canonical}",
                    put(http::labels::upsert_status_label),
                )
                .route(
                    "/api/outside-processing/service-type-labels/{code}",
                    put(http::labels::upsert_service_type_label),
                )
                .route_layer(RequirePermissionsLayer::new(&[
                    permissions::OUTSIDE_PROCESSING_MUTATE,
                ]))
                .with_state(state.clone());

            let reads = Router::new()
                .route(
                    "/api/outside-processing/orders",
                    get(http::orders::list_orders),
                )
                .route(
                    "/api/outside-processing/orders/{id}",
                    get(http::orders::get_order),
                )
                .route(
                    "/api/outside-processing/orders/{id}/ship-events",
                    get(http::ship_events::list_ship_events),
                )
                .route(
                    "/api/outside-processing/orders/{id}/return-events",
                    get(http::return_events::list_return_events),
                )
                .route(
                    "/api/outside-processing/orders/{id}/reviews",
                    get(http::reviews::list_reviews),
                )
                .route(
                    "/api/outside-processing/orders/{id}/re-identifications",
                    get(http::re_identifications::list_re_identifications),
                )
                .route(
                    "/api/outside-processing/status-labels",
                    get(http::labels::list_status_labels),
                )
                .route(
                    "/api/outside-processing/service-type-labels",
                    get(http::labels::list_service_type_labels),
                )
                .route_layer(RequirePermissionsLayer::new(&[
                    permissions::OUTSIDE_PROCESSING_READ,
                ]))
                .with_state(state);

            Router::new().merge(mutations).merge(reads)
        })
        .run()
        .await
        .expect("outside-processing module failed");
}
