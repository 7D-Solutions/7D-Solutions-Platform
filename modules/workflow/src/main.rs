use axum::{
    routing::{get, patch, post},
    Router,
};
use std::sync::Arc;

use platform_sdk::ModuleBuilder;
use security::{permissions, RequirePermissionsLayer};
use workflow::{http, metrics, AppState};

static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./db/migrations");

#[tokio::main]
async fn main() {
    ModuleBuilder::from_manifest("module.toml")
        .migrator(&MIGRATOR)
        .routes(|ctx| {
            let app_metrics = Arc::new(
                metrics::WorkflowMetrics::new().expect("Workflow: failed to create metrics"),
            );
            let app_state = Arc::new(AppState {
                pool: ctx.pool().clone(),
                metrics: app_metrics,
            });

            Router::new()
                .route("/api/openapi.json", get(http::openapi_json))
                .merge(
                    Router::new()
                        .route(
                            "/api/workflow/definitions",
                            post(http::definitions::create_definition)
                                .get(http::definitions::list_definitions),
                        )
                        .route(
                            "/api/workflow/definitions/{def_id}",
                            get(http::definitions::get_definition),
                        )
                        .route(
                            "/api/workflow/instances",
                            post(http::instances::start_instance)
                                .get(http::instances::list_instances),
                        )
                        .route(
                            "/api/workflow/instances/{instance_id}",
                            get(http::instances::get_instance),
                        )
                        .route(
                            "/api/workflow/instances/{instance_id}/advance",
                            patch(http::instances::advance_instance),
                        )
                        .route(
                            "/api/workflow/instances/{instance_id}/transitions",
                            get(http::instances::list_transitions),
                        )
                        .route_layer(RequirePermissionsLayer::new(&[
                            permissions::WORKFLOW_MUTATE,
                        ])),
                )
                .with_state(app_state)
        })
        .run()
        .await
        .expect("workflow module failed");
}
