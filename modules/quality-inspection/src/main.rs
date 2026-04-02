use axum::Router;
use std::sync::Arc;

use quality_inspection_rs::{
    consumers::production_event_bridge::start_production_event_bridge,
    consumers::receipt_event_bridge::start_receipt_event_bridge,
    http::{
        inspection_routes::{
            get_inspection, get_inspection_plan, get_inspections_by_lot,
            get_inspections_by_part_rev, get_inspections_by_receipt, get_inspections_by_wo,
            post_accept_inspection, post_activate_plan, post_final_inspection,
            post_hold_inspection, post_in_process_inspection, post_inspection_plan,
            post_receiving_inspection, post_reject_inspection, post_release_inspection,
        },
        openapi_json,
    },
    metrics::QualityInspectionMetrics,
    AppState,
};
use security::{permissions, RequirePermissionsLayer};
use platform_sdk::ModuleBuilder;

static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./db/migrations");

#[tokio::main]
async fn main() {
    ModuleBuilder::from_manifest("module.toml")
        .migrator(&MIGRATOR)
        .routes(|ctx| {
            let wc_base_url = std::env::var("WORKFORCE_COMPETENCE_BASE_URL")
                .unwrap_or_else(|_| "http://localhost:8121".to_string());
            let wc_client = platform_sdk::PlatformClient::new(wc_base_url);

            // Spawn consumers using the SDK's bus
            if let Ok(bus) = ctx.bus_arc() {
                let consumer_pool = ctx.pool().clone();
                let consumer_bus = bus.clone();
                tokio::spawn(async move {
                    start_receipt_event_bridge(consumer_bus.clone(), consumer_pool.clone()).await;
                    start_production_event_bridge(consumer_bus, consumer_pool).await;
                });
            }

            let metrics = Arc::new(
                QualityInspectionMetrics::new().expect("Failed to create metrics registry"),
            );
            let app_state = Arc::new(AppState {
                pool: ctx.pool().clone(),
                wc_client,
                metrics,
            });

            let qi_mutations = Router::new()
                .route(
                    "/api/quality-inspection/plans",
                    axum::routing::post(post_inspection_plan),
                )
                .route(
                    "/api/quality-inspection/plans/{plan_id}/activate",
                    axum::routing::post(post_activate_plan),
                )
                .route(
                    "/api/quality-inspection/inspections",
                    axum::routing::post(post_receiving_inspection),
                )
                .route(
                    "/api/quality-inspection/inspections/in-process",
                    axum::routing::post(post_in_process_inspection),
                )
                .route(
                    "/api/quality-inspection/inspections/final",
                    axum::routing::post(post_final_inspection),
                )
                .route(
                    "/api/quality-inspection/inspections/{inspection_id}/hold",
                    axum::routing::post(post_hold_inspection),
                )
                .route(
                    "/api/quality-inspection/inspections/{inspection_id}/release",
                    axum::routing::post(post_release_inspection),
                )
                .route(
                    "/api/quality-inspection/inspections/{inspection_id}/accept",
                    axum::routing::post(post_accept_inspection),
                )
                .route(
                    "/api/quality-inspection/inspections/{inspection_id}/reject",
                    axum::routing::post(post_reject_inspection),
                )
                .route_layer(RequirePermissionsLayer::new(&[
                    permissions::QUALITY_INSPECTION_MUTATE,
                ]))
                .with_state(app_state.clone());

            let qi_reads = Router::new()
                .route(
                    "/api/quality-inspection/plans/{plan_id}",
                    axum::routing::get(get_inspection_plan),
                )
                .route(
                    "/api/quality-inspection/inspections/{inspection_id}",
                    axum::routing::get(get_inspection),
                )
                .route(
                    "/api/quality-inspection/inspections/by-part-rev",
                    axum::routing::get(get_inspections_by_part_rev),
                )
                .route(
                    "/api/quality-inspection/inspections/by-receipt",
                    axum::routing::get(get_inspections_by_receipt),
                )
                .route(
                    "/api/quality-inspection/inspections/by-wo",
                    axum::routing::get(get_inspections_by_wo),
                )
                .route(
                    "/api/quality-inspection/inspections/by-lot",
                    axum::routing::get(get_inspections_by_lot),
                )
                .route_layer(RequirePermissionsLayer::new(&[
                    permissions::QUALITY_INSPECTION_READ,
                ]))
                .with_state(app_state);

            Router::new()
                .route("/api/openapi.json", axum::routing::get(openapi_json))
                .merge(qi_reads)
                .merge(qi_mutations)
        })
        .run()
        .await
        .expect("quality-inspection module failed");
}
