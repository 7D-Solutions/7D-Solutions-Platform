use axum::{
    routing::{get, post, put},
    Router,
};
use std::sync::Arc;

use platform_sdk::ModuleBuilder;
use sales_orders_rs::{consumers, http, metrics::SoMetrics, AppState, Config};
use security::{permissions, RequirePermissionsLayer};

static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./db/migrations");

#[tokio::main]
async fn main() {
    ModuleBuilder::from_manifest("module.toml")
        .migrator(&MIGRATOR)
        .routes(|ctx| {
            let so_metrics =
                Arc::new(SoMetrics::new().expect("SO: failed to create metrics"));

            let app_state = Arc::new(AppState {
                pool: ctx.pool().clone(),
                metrics: so_metrics,
                config: Config::from_env(),
            });

            if let Ok(bus) = ctx.bus_arc() {
                consumers::shipment_shipped::start_shipment_shipped_consumer(
                    bus.clone(),
                    ctx.pool().clone(),
                );
                consumers::invoice_issued::start_invoice_issued_consumer(
                    bus,
                    ctx.pool().clone(),
                );
                tracing::info!("SO: event consumers started");
            }

            let so_mutate =
                RequirePermissionsLayer::new(&[permissions::SALES_ORDERS_MUTATE]);
            let so_read =
                RequirePermissionsLayer::new(&[permissions::SALES_ORDERS_READ]);

            let mutations = Router::new()
                .route("/api/so/orders", post(http::orders::create_order))
                .route("/api/so/orders/{order_id}", put(http::orders::update_order))
                .route(
                    "/api/so/orders/{order_id}/book",
                    post(http::orders::book_order),
                )
                .route(
                    "/api/so/orders/{order_id}/cancel",
                    post(http::orders::cancel_order),
                )
                .route(
                    "/api/so/orders/{order_id}/lines",
                    post(http::orders::add_line),
                )
                .route(
                    "/api/so/orders/{order_id}/lines/{line_id}",
                    put(http::orders::update_line).delete(http::orders::remove_line),
                )
                .route("/api/so/blankets", post(http::blankets::create_blanket))
                .route(
                    "/api/so/blankets/{blanket_id}",
                    put(http::blankets::update_blanket),
                )
                .route(
                    "/api/so/blankets/{blanket_id}/activate",
                    post(http::blankets::activate_blanket),
                )
                .route(
                    "/api/so/blankets/{blanket_id}/lines",
                    post(http::blankets::add_blanket_line),
                )
                .route(
                    "/api/so/blankets/{blanket_id}/releases",
                    post(http::releases::create_release),
                )
                .route(
                    "/api/so/labels/{label_type}/{status_key}",
                    put(http::labels::upsert_label).delete(http::labels::delete_label),
                )
                .layer(so_mutate)
                .with_state(app_state.clone());

            let reads = Router::new()
                .route("/api/so/orders", get(http::orders::list_orders))
                .route("/api/so/orders/{order_id}", get(http::orders::get_order))
                .route("/api/so/blankets", get(http::blankets::list_blankets))
                .route(
                    "/api/so/blankets/{blanket_id}",
                    get(http::blankets::get_blanket),
                )
                .route(
                    "/api/so/blankets/{blanket_id}/lines/{line_id}/releases",
                    get(http::releases::list_releases),
                )
                .route("/api/so/labels", get(http::labels::list_labels))
                .layer(so_read)
                .with_state(app_state.clone());

            let infra = Router::new()
                .route("/health/ready", get(http::health_check))
                .route("/openapi.json", get(http::openapi_json))
                .route(
                    "/metrics",
                    get(crate::metrics_handler_wrapper).with_state(app_state.clone()),
                )
                .with_state(app_state);

            mutations.merge(reads).merge(infra)
        })
        .run()
        .await
        .expect("sales-orders module failed");
}

async fn metrics_handler_wrapper(
    axum::extract::State(state): axum::extract::State<Arc<AppState>>,
) -> impl axum::response::IntoResponse {
    sales_orders_rs::metrics::metrics_handler(axum::extract::State(state)).await
}
