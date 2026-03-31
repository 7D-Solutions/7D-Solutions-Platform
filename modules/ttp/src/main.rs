use std::sync::Arc;

use ttp_rs::{http, metrics, AppState};
use platform_sdk::ModuleBuilder;

static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./db/migrations");

#[tokio::main]
async fn main() {
    ModuleBuilder::from_manifest("module.toml")
        .migrator(&MIGRATOR)
        .routes(|ctx| {
            let ttp_metrics =
                Arc::new(metrics::TtpMetrics::new().expect("TTP: failed to create metrics"));
            let app_state = Arc::new(AppState {
                pool: ctx.pool().clone(),
                metrics: ttp_metrics,
            });

            http::router(app_state)
        })
        .run()
        .await
        .expect("ttp module failed");
}
