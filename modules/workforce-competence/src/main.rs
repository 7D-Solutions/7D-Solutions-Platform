use axum::Router;
use security::{permissions, RequirePermissionsLayer};
use std::sync::Arc;

use platform_sdk::ModuleBuilder;
use workforce_competence_rs::{
    http::handlers::{
        get_acceptance_authority_check, get_artifact, get_authorization, post_artifact,
        post_assignment, post_grant_authority, post_revoke_authority,
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
