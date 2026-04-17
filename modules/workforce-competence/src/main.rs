use axum::Router;
use security::{permissions, RequirePermissionsLayer};
use std::sync::Arc;

use platform_sdk::ModuleBuilder;
use workforce_competence_rs::{
    http::handlers::{
        get_acceptance_authority_check, get_artifact, get_authorization, get_training_assignment,
        get_training_plan, list_training_assignments, list_training_completions,
        list_training_plans, patch_assignment_status, post_artifact, post_assignment,
        post_grant_authority, post_revoke_authority, post_training_assignment,
        post_training_completion, post_training_plan,
    },
    metrics::WcMetrics,
    AppState,
};

static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./db/migrations");

#[tokio::main]
async fn main() {
    ModuleBuilder::from_manifest("module.toml")
        .migrator(&MIGRATOR)
        .routes(|ctx| {
            let metrics = Arc::new(WcMetrics::new().expect("Failed to create metrics registry"));
            let app_state = Arc::new(AppState {
                pool: ctx.pool().clone(),
                metrics,
            });

            let wc_mutations = Router::new()
                .route(
                    "/api/workforce-competence/artifacts",
                    axum::routing::post(post_artifact),
                )
                .route(
                    "/api/workforce-competence/assignments",
                    axum::routing::post(post_assignment),
                )
                .route(
                    "/api/workforce-competence/acceptance-authorities",
                    axum::routing::post(post_grant_authority),
                )
                .route(
                    "/api/workforce-competence/acceptance-authorities/{id}/revoke",
                    axum::routing::post(post_revoke_authority),
                )
                .route(
                    "/api/workforce-competence/training-plans",
                    axum::routing::post(post_training_plan),
                )
                .route(
                    "/api/workforce-competence/training-assignments",
                    axum::routing::post(post_training_assignment),
                )
                .route(
                    "/api/workforce-competence/training-assignments/{id}/status",
                    axum::routing::patch(patch_assignment_status),
                )
                .route(
                    "/api/workforce-competence/training-completions",
                    axum::routing::post(post_training_completion),
                )
                .route_layer(RequirePermissionsLayer::new(&[
                    permissions::WORKFORCE_COMPETENCE_MUTATE,
                ]))
                .with_state(app_state.clone());

            let wc_reads = Router::new()
                .route(
                    "/api/workforce-competence/artifacts/{id}",
                    axum::routing::get(get_artifact),
                )
                .route(
                    "/api/workforce-competence/authorization",
                    axum::routing::get(get_authorization),
                )
                .route(
                    "/api/workforce-competence/acceptance-authority-check",
                    axum::routing::get(get_acceptance_authority_check),
                )
                .route(
                    "/api/workforce-competence/training-plans",
                    axum::routing::get(list_training_plans),
                )
                .route(
                    "/api/workforce-competence/training-plans/{id}",
                    axum::routing::get(get_training_plan),
                )
                .route(
                    "/api/workforce-competence/training-assignments",
                    axum::routing::get(list_training_assignments),
                )
                .route(
                    "/api/workforce-competence/training-assignments/{id}",
                    axum::routing::get(get_training_assignment),
                )
                .route(
                    "/api/workforce-competence/training-completions",
                    axum::routing::get(list_training_completions),
                )
                .route_layer(RequirePermissionsLayer::new(&[
                    permissions::WORKFORCE_COMPETENCE_READ,
                ]))
                .with_state(app_state);

            Router::new().merge(wc_reads).merge(wc_mutations)
        })
        .run()
        .await
        .expect("workforce-competence module failed");
}
