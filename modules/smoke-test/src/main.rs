use axum::{
    routing::{delete, get, post, put},
    Json, Router,
};
use security::RequirePermissionsLayer;
use std::sync::Arc;
use utoipa::OpenApi;

use platform_sdk::ModuleBuilder;
use smoke_test::{consumer, http, AppState};

async fn openapi_json() -> Json<utoipa::openapi::OpenApi> {
    Json(http::ApiDoc::openapi())
}

static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./db/migrations");

#[tokio::main]
async fn main() {
    ModuleBuilder::from_manifest("module.toml")
        .migrator(&MIGRATOR)
        .routes_async(|ctx| async move {
            let pool = ctx.pool().clone();

            // Start consumer
            if let Ok(bus) = ctx.bus_arc() {
                consumer::start_echo_consumer(bus, pool.clone()).await;
            }

            let app_state = Arc::new(AppState { pool });

            let mutations = Router::new()
                .route("/api/smoke/items", post(http::items::create_item))
                .route(
                    "/api/smoke/items/{id}",
                    put(http::items::update_item).delete(http::items::delete_item),
                )
                .route_layer(RequirePermissionsLayer::new(&["smoke.mutate"]))
                .with_state(app_state.clone());

            let reads = Router::new()
                .route("/api/smoke/items", get(http::items::list_items))
                .route("/api/smoke/items/{id}", get(http::items::get_item))
                .route_layer(RequirePermissionsLayer::new(&["smoke.read"]))
                .with_state(app_state.clone());

            Router::new()
                .route("/api/openapi.json", get(openapi_json))
                .with_state(app_state)
                .merge(mutations)
                .merge(reads)
        })
        .run()
        .await
        .expect("smoke-test module failed");
}
