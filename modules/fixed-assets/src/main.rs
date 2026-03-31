use axum::{
    routing::{get, post, put},
    Json, Router,
};
use std::sync::Arc;
use utoipa::OpenApi;

use fixed_assets::{consumers, http, metrics, AppState};
use security::{permissions, RequirePermissionsLayer};
use platform_sdk::{ConsumerError, EventEnvelope, ModuleBuilder, ModuleContext};

async fn openapi_json() -> Json<utoipa::openapi::OpenApi> {
    Json(http::ApiDoc::openapi())
}

static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./db/migrations");

/// SDK consumer adapter for ap.events.ap.vendor_bill_approved.
async fn on_bill_approved(
    ctx: ModuleContext,
    envelope: EventEnvelope<serde_json::Value>,
) -> Result<(), ConsumerError> {
    let pool = ctx.pool();
    let event_id = envelope.event_id;

    let payload: consumers::ap_bill_approved::VendorBillApprovedPayload =
        serde_json::from_value(envelope.payload.clone())
            .map_err(|e| ConsumerError::Processing(format!("payload parse: {e}")))?;

    tracing::info!(event_id = %event_id, "Processing ap.vendor_bill_approved");

    consumers::ap_bill_approved::handle_bill_approved(pool, event_id, &payload)
        .await
        .map_err(|e| ConsumerError::Processing(e.to_string()))?;

    Ok(())
}

#[tokio::main]
async fn main() {
    ModuleBuilder::from_manifest("module.toml")
        .migrator(&MIGRATOR)
        .consumer("ap.events.ap.vendor_bill_approved", on_bill_approved)
        .routes(|ctx| {
            let fa_metrics = Arc::new(
                metrics::FixedAssetsMetrics::new()
                    .expect("Fixed Assets: failed to create metrics"),
            );
            let app_state = Arc::new(AppState {
                pool: ctx.pool().clone(),
                metrics: fa_metrics,
            });

            let fa_mutations = Router::new()
                .route(
                    "/api/fixed-assets/categories",
                    post(http::assets::create_category),
                )
                .route(
                    "/api/fixed-assets/categories/{id}",
                    put(http::assets::update_category).delete(http::assets::deactivate_category),
                )
                .route("/api/fixed-assets/assets", post(http::assets::create_asset))
                .route(
                    "/api/fixed-assets/assets/{id}",
                    put(http::assets::update_asset).delete(http::assets::deactivate_asset),
                )
                .route(
                    "/api/fixed-assets/depreciation/schedule",
                    post(http::depreciation::generate_schedule),
                )
                .route(
                    "/api/fixed-assets/depreciation/runs",
                    post(http::depreciation::create_run),
                )
                .route(
                    "/api/fixed-assets/disposals",
                    post(http::disposals::dispose_asset),
                )
                .route_layer(RequirePermissionsLayer::new(&[
                    permissions::FIXED_ASSETS_MUTATE,
                ]))
                .with_state(app_state.clone());

            let fa_reads = Router::new()
                .route(
                    "/api/fixed-assets/categories/{id}",
                    get(http::assets::get_category),
                )
                .route(
                    "/api/fixed-assets/categories",
                    get(http::assets::list_categories),
                )
                .route(
                    "/api/fixed-assets/assets/{id}",
                    get(http::assets::get_asset),
                )
                .route("/api/fixed-assets/assets", get(http::assets::list_assets))
                .route(
                    "/api/fixed-assets/depreciation/runs",
                    get(http::depreciation::list_runs),
                )
                .route(
                    "/api/fixed-assets/depreciation/runs/{id}",
                    get(http::depreciation::get_run),
                )
                .route(
                    "/api/fixed-assets/disposals",
                    get(http::disposals::list_disposals),
                )
                .route(
                    "/api/fixed-assets/disposals/{id}",
                    get(http::disposals::get_disposal),
                )
                .route_layer(RequirePermissionsLayer::new(&[
                    permissions::FIXED_ASSETS_READ,
                ]))
                .with_state(app_state);

            Router::new()
                .route("/api/openapi.json", get(openapi_json))
                .merge(fa_reads)
                .merge(fa_mutations)
                .merge(http::admin::admin_router(ctx.pool().clone()))
        })
        .run()
        .await
        .expect("fixed-assets module failed");
}
