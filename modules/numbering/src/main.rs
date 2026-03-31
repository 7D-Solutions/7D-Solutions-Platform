use axum::{
    routing::{get, post, put},
    Json, Router,
};
use std::sync::Arc;
use utoipa::OpenApi;

use numbering::{http, metrics, AppState};
use security::{permissions, RequirePermissionsLayer};
use platform_sdk::ModuleBuilder;

async fn openapi_json() -> Json<utoipa::openapi::OpenApi> {
    Json(http::ApiDoc::openapi())
}

static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./db/migrations");

#[tokio::main]
async fn main() {
    ModuleBuilder::from_manifest("module.toml")
        .migrator(&MIGRATOR)
        .routes(|ctx| {
            let app_metrics = Arc::new(
                metrics::NumberingMetrics::new().expect("Numbering: failed to create metrics"),
            );
            let app_state = Arc::new(AppState {
                pool: ctx.pool().clone(),
                metrics: app_metrics,
            });

            Router::new()
                .route("/api/openapi.json", get(openapi_json))
                .merge(
                    Router::new()
                        .route("/allocate", post(http::allocate::allocate))
                        .route("/confirm", post(http::confirm::confirm))
                        .route_layer(RequirePermissionsLayer::new(&[
                            permissions::NUMBERING_ALLOCATE,
                        ])),
                )
                .merge(
                    Router::new()
                        .route(
                            "/policies/{entity}",
                            put(http::policy::upsert_policy).get(http::policy::get_policy),
                        )
                        .route_layer(RequirePermissionsLayer::new(&[
                            permissions::NUMBERING_ALLOCATE,
                        ])),
                )
                .with_state(app_state)
        })
        .run()
        .await
        .expect("numbering module failed");
}
