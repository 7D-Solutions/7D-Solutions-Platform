use axum::{
    routing::{get, post},
    Json, Router,
};
use utoipa::OpenApi;
use maintenance_rs::{config::Config, http, metrics, AppState};
use security::{permissions, RequirePermissionsLayer};
use std::sync::Arc;

use maintenance_rs::consumers::production_downtime_bridge::{
    process_downtime_ended, process_downtime_started, DowntimeEndedPayload, DowntimeStartedPayload,
};
use maintenance_rs::consumers::production_workcenter_bridge::{
    upsert_workcenter_projection, WorkcenterCreatedPayload, WorkcenterDeactivatedPayload,
    WorkcenterUpdatedPayload,
};
use platform_sdk::{ConsumerError, EventEnvelope, ModuleBuilder, ModuleContext};

#[derive(OpenApi)]
#[openapi(
    info(
        title = "Maintenance Service",
        version = "2.1.0",
        description = "Maintenance management: work orders, preventive plans, meters, calibration, \
                        downtime tracking, and labor management.\n\n\
                        **Authentication:** Bearer JWT. Tenant derived from JWT claims.\n\
                        Permissions: MAINTENANCE_READ for queries, MAINTENANCE_MUTATE for writes."
    ),
    security(("bearer" = [])),
    modifiers(&SecurityAddon),
)]
struct ApiDoc;

struct SecurityAddon;

impl utoipa::Modify for SecurityAddon {
    fn modify(&self, openapi: &mut utoipa::openapi::OpenApi) {
        let components = openapi.components.get_or_insert_with(Default::default);
        components.add_security_scheme(
            "bearer",
            utoipa::openapi::security::SecurityScheme::Http(
                utoipa::openapi::security::HttpBuilder::new()
                    .scheme(utoipa::openapi::security::HttpAuthScheme::Bearer)
                    .bearer_format("JWT")
                    .build(),
            ),
        );
    }
}

static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./db/migrations");

#[tokio::main]
async fn main() {
    ModuleBuilder::from_manifest("module.toml")
        .migrator(&MIGRATOR)
        .consumer(
            "production.workcenter_created",
            on_workcenter_created,
        )
        .consumer(
            "production.workcenter_updated",
            on_workcenter_updated,
        )
        .consumer(
            "production.workcenter_deactivated",
            on_workcenter_deactivated,
        )
        .consumer("production.downtime.started", on_downtime_started)
        .consumer("production.downtime.ended", on_downtime_ended)
        .routes(|ctx| {
            let pool = ctx.pool().clone();
            let config = Config::from_env().unwrap_or_else(|err| {
                tracing::error!("Maintenance config error: {}", err);
                panic!("Maintenance config error: {}", err);
            });

            // Initialize metrics and register with global prometheus registry
            let app_metrics = Arc::new(
                metrics::MaintenanceMetrics::new()
                    .expect("Maintenance: failed to create metrics"),
            );
            let _ = prometheus::register(Box::new(
                app_metrics.http_request_duration_seconds.clone(),
            ));
            let _ = prometheus::register(Box::new(
                app_metrics.http_requests_total.clone(),
            ));
            let _ = prometheus::register(Box::new(
                app_metrics.outbox_queue_depth.clone(),
            ));
            let _ = prometheus::register(Box::new(
                app_metrics.events_enqueued_total.clone(),
            ));

            // Spawn scheduler tick loop
            let scheduler_pool = pool.clone();
            let scheduler_interval = config.scheduler_interval_secs;
            tokio::spawn(async move {
                maintenance_rs::domain::scheduler::run_scheduler_task(
                    scheduler_pool,
                    scheduler_interval,
                )
                .await;
            });

            let app_state = Arc::new(AppState {
                pool: pool.clone(),
                metrics: app_metrics,
            });

            Router::new()
                .route(
                    "/api/openapi.json",
                    get(|| async { Json(ApiDoc::openapi()) }),
                )
                // Read-only endpoints (MAINTENANCE_READ)
                .merge(
                    Router::new()
                        .route(
                            "/api/maintenance/assets",
                            get(http::assets::list_assets),
                        )
                        .route(
                            "/api/maintenance/assets/{asset_id}",
                            get(http::assets::get_asset),
                        )
                        .route(
                            "/api/maintenance/meter-types",
                            get(http::meters::list_meter_types),
                        )
                        .route(
                            "/api/maintenance/assets/{asset_id}/readings",
                            get(http::meters::list_readings),
                        )
                        .route(
                            "/api/maintenance/plans",
                            get(http::plans::list_plans),
                        )
                        .route(
                            "/api/maintenance/plans/{plan_id}",
                            get(http::plans::get_plan),
                        )
                        .route(
                            "/api/maintenance/assignments",
                            get(http::plans::list_assignments),
                        )
                        .route(
                            "/api/maintenance/work-orders",
                            get(http::work_orders::list_work_orders),
                        )
                        .route(
                            "/api/maintenance/work-orders/{wo_id}",
                            get(http::work_orders::get_work_order),
                        )
                        .route(
                            "/api/maintenance/work-orders/{wo_id}/parts",
                            get(http::work_order_parts::list_parts),
                        )
                        .route(
                            "/api/maintenance/work-orders/{wo_id}/labor",
                            get(http::work_order_labor::list_labor),
                        )
                        .route(
                            "/api/maintenance/downtime-events",
                            get(http::downtime::list_downtime),
                        )
                        .route(
                            "/api/maintenance/downtime-events/{id}",
                            get(http::downtime::get_downtime),
                        )
                        .route(
                            "/api/maintenance/assets/{asset_id}/downtime",
                            get(http::downtime::list_asset_downtime),
                        )
                        .route(
                            "/api/maintenance/assets/{asset_id}/calibration-status",
                            get(http::calibration_events::get_calibration_status),
                        )
                        .route_layer(RequirePermissionsLayer::new(&[
                            permissions::MAINTENANCE_READ,
                        ])),
                )
                // Mutation endpoints (MAINTENANCE_MUTATE)
                .merge(
                    Router::new()
                        .route(
                            "/api/maintenance/assets",
                            post(http::assets::create_asset),
                        )
                        .route(
                            "/api/maintenance/assets/{asset_id}",
                            axum::routing::patch(http::assets::update_asset),
                        )
                        .route(
                            "/api/maintenance/meter-types",
                            post(http::meters::create_meter_type),
                        )
                        .route(
                            "/api/maintenance/assets/{asset_id}/readings",
                            post(http::meters::record_reading),
                        )
                        .route(
                            "/api/maintenance/plans",
                            post(http::plans::create_plan),
                        )
                        .route(
                            "/api/maintenance/plans/{plan_id}",
                            axum::routing::patch(http::plans::update_plan),
                        )
                        .route(
                            "/api/maintenance/plans/{plan_id}/assign",
                            post(http::plans::assign_plan),
                        )
                        .route(
                            "/api/maintenance/work-orders",
                            post(http::work_orders::create_work_order),
                        )
                        .route(
                            "/api/maintenance/work-orders/{wo_id}/transition",
                            axum::routing::patch(http::work_orders::transition_work_order),
                        )
                        .route(
                            "/api/maintenance/work-orders/{wo_id}/parts",
                            post(http::work_order_parts::add_part),
                        )
                        .route(
                            "/api/maintenance/work-orders/{wo_id}/parts/{part_id}",
                            axum::routing::delete(http::work_order_parts::remove_part),
                        )
                        .route(
                            "/api/maintenance/work-orders/{wo_id}/labor",
                            post(http::work_order_labor::add_labor),
                        )
                        .route(
                            "/api/maintenance/work-orders/{wo_id}/labor/{labor_id}",
                            axum::routing::delete(http::work_order_labor::remove_labor),
                        )
                        .route(
                            "/api/maintenance/downtime-events",
                            post(http::downtime::create_downtime),
                        )
                        .route(
                            "/api/maintenance/assets/{asset_id}/calibration-events",
                            post(http::calibration_events::record_calibration_event),
                        )
                        .route_layer(RequirePermissionsLayer::new(&[
                            permissions::MAINTENANCE_MUTATE,
                        ])),
                )
                .with_state(app_state)
        })
        .run()
        .await
        .expect("maintenance module failed");
}

/// SDK consumer adapter for production.workcenter_created.
async fn on_workcenter_created(
    ctx: ModuleContext,
    envelope: EventEnvelope<serde_json::Value>,
) -> Result<(), ConsumerError> {
    let pool = ctx.pool();

    let payload: WorkcenterCreatedPayload =
        serde_json::from_value(envelope.payload.clone())
            .map_err(|e| ConsumerError::Processing(format!("payload parse: {e}")))?;

    upsert_workcenter_projection(
        pool,
        envelope.event_id,
        payload.workcenter_id,
        &payload.tenant_id,
        &payload.code,
        &payload.name,
        true,
    )
    .await
    .map_err(|e| ConsumerError::Processing(format!("DB error: {e}")))?;

    Ok(())
}

/// SDK consumer adapter for production.workcenter_updated.
async fn on_workcenter_updated(
    ctx: ModuleContext,
    envelope: EventEnvelope<serde_json::Value>,
) -> Result<(), ConsumerError> {
    let pool = ctx.pool();

    let payload: WorkcenterUpdatedPayload =
        serde_json::from_value(envelope.payload.clone())
            .map_err(|e| ConsumerError::Processing(format!("payload parse: {e}")))?;

    let existing_name: Option<(String,)> = sqlx::query_as(
        "SELECT name FROM workcenter_projections WHERE workcenter_id = $1",
    )
    .bind(payload.workcenter_id)
    .fetch_optional(pool)
    .await
    .map_err(|e| ConsumerError::Processing(format!("DB error: {e}")))?;

    let name = existing_name
        .map(|r| r.0)
        .unwrap_or_else(|| payload.code.clone());

    upsert_workcenter_projection(
        pool,
        envelope.event_id,
        payload.workcenter_id,
        &payload.tenant_id,
        &payload.code,
        &name,
        true,
    )
    .await
    .map_err(|e| ConsumerError::Processing(format!("DB error: {e}")))?;

    Ok(())
}

/// SDK consumer adapter for production.workcenter_deactivated.
async fn on_workcenter_deactivated(
    ctx: ModuleContext,
    envelope: EventEnvelope<serde_json::Value>,
) -> Result<(), ConsumerError> {
    let pool = ctx.pool();

    let payload: WorkcenterDeactivatedPayload =
        serde_json::from_value(envelope.payload.clone())
            .map_err(|e| ConsumerError::Processing(format!("payload parse: {e}")))?;

    let existing: Option<(String, String)> = sqlx::query_as(
        "SELECT code, name FROM workcenter_projections WHERE workcenter_id = $1",
    )
    .bind(payload.workcenter_id)
    .fetch_optional(pool)
    .await
    .map_err(|e| ConsumerError::Processing(format!("DB error: {e}")))?;

    let (code, name) = existing.unwrap_or_else(|| ("unknown".to_string(), "unknown".to_string()));

    upsert_workcenter_projection(
        pool,
        envelope.event_id,
        payload.workcenter_id,
        &payload.tenant_id,
        &code,
        &name,
        false,
    )
    .await
    .map_err(|e| ConsumerError::Processing(format!("DB error: {e}")))?;

    Ok(())
}

/// SDK consumer adapter for production.downtime.started.
async fn on_downtime_started(
    ctx: ModuleContext,
    envelope: EventEnvelope<serde_json::Value>,
) -> Result<(), ConsumerError> {
    let pool = ctx.pool();

    let payload: DowntimeStartedPayload =
        serde_json::from_value(envelope.payload.clone())
            .map_err(|e| ConsumerError::Processing(format!("payload parse: {e}")))?;

    process_downtime_started(pool, envelope.event_id, &payload)
        .await
        .map_err(|e| ConsumerError::Processing(e))?;

    Ok(())
}

/// SDK consumer adapter for production.downtime.ended.
async fn on_downtime_ended(
    ctx: ModuleContext,
    envelope: EventEnvelope<serde_json::Value>,
) -> Result<(), ConsumerError> {
    let pool = ctx.pool();

    let payload: DowntimeEndedPayload =
        serde_json::from_value(envelope.payload.clone())
            .map_err(|e| ConsumerError::Processing(format!("payload parse: {e}")))?;

    process_downtime_ended(pool, envelope.event_id, &payload)
        .await
        .map_err(|e| ConsumerError::Processing(e))?;

    Ok(())
}
