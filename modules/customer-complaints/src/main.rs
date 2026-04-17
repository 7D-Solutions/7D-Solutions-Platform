use axum::Router;
use std::sync::Arc;

use customer_complaints_rs::{metrics, AppState};
use platform_sdk::ModuleBuilder;

static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./db/migrations");

#[tokio::main]
async fn main() {
    ModuleBuilder::from_manifest("module.toml")
        .migrator(&MIGRATOR)
        .routes(|ctx| {
            let cc_metrics =
                Arc::new(metrics::CcMetrics::new().expect("CC: failed to create metrics"));

            let _state = Arc::new(AppState {
                pool: ctx.pool().clone(),
                metrics: cc_metrics,
            });

            // HTTP routes and consumers are wired in Phase B (bd-4l79e.1) and Phase C (bd-4l79e.2).
            Router::new()
        })
        .run()
        .await
        .expect("customer-complaints module failed");
}
