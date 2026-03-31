use axum::{routing::get, Json};
use std::sync::Arc;
use utoipa::OpenApi;

use timekeeping::{http, http::ApiDoc, metrics, AppState};
use platform_sdk::ModuleBuilder;

async fn openapi_json() -> Json<utoipa::openapi::OpenApi> {
    Json(ApiDoc::openapi())
}

static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./db/migrations");

#[tokio::main]
async fn main() {
    ModuleBuilder::from_manifest("module.toml")
        .migrator(&MIGRATOR)
        .routes(|ctx| {
            let tk_metrics = Arc::new(
                metrics::TimekeepingMetrics::new().expect("Timekeeping: failed to create metrics"),
            );
            let app_state = Arc::new(AppState {
                pool: ctx.pool().clone(),
                metrics: tk_metrics,
            });

            http::router(app_state)
                .route("/api/openapi.json", get(openapi_json))
                .merge(http::admin::admin_router(ctx.pool().clone()))
        })
        .run()
        .await
        .expect("timekeeping module failed");
}
