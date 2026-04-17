use axum::{
    routing::{delete, get, post, put},
    Router,
};
use std::sync::Arc;

use platform_sdk::ModuleBuilder;
use security::{permissions, RequirePermissionsLayer};
use shop_floor_gates_rs::{consumers, http, metrics::SfgMetrics, AppState, Config};

static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./db/migrations");

#[tokio::main]
async fn main() {
    ModuleBuilder::from_manifest("module.toml")
        .migrator(&MIGRATOR)
        .routes(|ctx| {
            let metrics = Arc::new(SfgMetrics::new().expect("SFG: failed to create metrics"));

            let app_state = Arc::new(AppState {
                pool: ctx.pool().clone(),
                metrics,
                config: Config::from_env(),
            });

            if let Ok(bus) = ctx.bus_arc() {
                consumers::work_order_cancelled::start_work_order_cancelled_consumer(
                    bus.clone(),
                    ctx.pool().clone(),
                );
                consumers::work_order_completed::start_work_order_completed_consumer(
                    bus.clone(),
                    ctx.pool().clone(),
                );
                consumers::operation_completed::start_operation_completed_consumer(
                    bus,
                    ctx.pool().clone(),
                );
                tracing::info!("SFG: event consumers started");
            }

            let mutate = RequirePermissionsLayer::new(&[permissions::SHOP_FLOOR_GATES_MUTATE]);
            let read = RequirePermissionsLayer::new(&[permissions::SHOP_FLOOR_GATES_READ]);

            let mutations = Router::new()
                // Holds
                .route("/api/sfg/holds", post(http::holds::place_hold))
                .route(
                    "/api/sfg/holds/{hold_id}/release",
                    post(http::holds::release_hold),
                )
                .route(
                    "/api/sfg/holds/{hold_id}/cancel",
                    post(http::holds::cancel_hold),
                )
                // Handoffs
                .route("/api/sfg/handoffs", post(http::handoffs::initiate_handoff))
                .route(
                    "/api/sfg/handoffs/{handoff_id}/accept",
                    post(http::handoffs::accept_handoff),
                )
                .route(
                    "/api/sfg/handoffs/{handoff_id}/reject",
                    post(http::handoffs::reject_handoff),
                )
                .route(
                    "/api/sfg/handoffs/{handoff_id}/cancel",
                    post(http::handoffs::cancel_handoff),
                )
                // Verifications
                .route(
                    "/api/sfg/verifications",
                    post(http::verifications::create_verification),
                )
                .route(
                    "/api/sfg/verifications/{verification_id}/operator-confirm",
                    post(http::verifications::operator_confirm),
                )
                .route(
                    "/api/sfg/verifications/{verification_id}/verify",
                    post(http::verifications::verify),
                )
                .route(
                    "/api/sfg/verifications/{verification_id}/skip",
                    post(http::verifications::skip_verification),
                )
                // Signoffs
                .route("/api/sfg/signoffs", post(http::signoffs::record_signoff))
                // Labels
                .route(
                    "/api/sfg/labels/{table}",
                    put(http::labels::upsert_label),
                )
                .route(
                    "/api/sfg/labels/{table}/{id}",
                    delete(http::labels::delete_label),
                )
                .layer(mutate)
                .with_state(app_state.clone());

            let reads = Router::new()
                // Holds
                .route("/api/sfg/holds", get(http::holds::list_holds))
                .route("/api/sfg/holds/{hold_id}", get(http::holds::get_hold))
                .route(
                    "/api/sfg/work-orders/{work_order_id}/active-holds",
                    get(http::holds::active_hold_count),
                )
                // Handoffs
                .route("/api/sfg/handoffs", get(http::handoffs::list_handoffs))
                .route(
                    "/api/sfg/handoffs/{handoff_id}",
                    get(http::handoffs::get_handoff),
                )
                // Verifications
                .route(
                    "/api/sfg/verifications",
                    get(http::verifications::list_verifications),
                )
                .route(
                    "/api/sfg/verifications/{verification_id}",
                    get(http::verifications::get_verification),
                )
                // Signoffs
                .route("/api/sfg/signoffs", get(http::signoffs::list_signoffs))
                .route(
                    "/api/sfg/signoffs/{signoff_id}",
                    get(http::signoffs::get_signoff),
                )
                // Labels
                .route("/api/sfg/labels/{table}", get(http::labels::list_labels))
                .layer(read)
                .with_state(app_state.clone());

            let infra = Router::new()
                .route("/health/ready", get(http::health_check))
                .route(
                    "/metrics",
                    get(metrics_handler_wrapper).with_state(app_state.clone()),
                )
                .with_state(app_state);

            mutations.merge(reads).merge(infra)
        })
        .run()
        .await
        .expect("shop-floor-gates module failed");
}

async fn metrics_handler_wrapper(
    axum::extract::State(state): axum::extract::State<Arc<AppState>>,
) -> impl axum::response::IntoResponse {
    shop_floor_gates_rs::metrics::metrics_handler(axum::extract::State(state)).await
}
