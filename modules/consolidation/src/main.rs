use std::sync::Arc;

use consolidation::{http, metrics, AppState};
use platform_sdk::ModuleBuilder;

static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./db/migrations");

#[tokio::main]
async fn main() {
    ModuleBuilder::from_manifest("module.toml")
        .migrator(&MIGRATOR)
        .routes(|ctx| {
            let consolidation_metrics = Arc::new(
                metrics::ConsolidationMetrics::new()
                    .expect("Consolidation: failed to create metrics"),
            );
            let gl_base_url = std::env::var("GL_BASE_URL")
                .unwrap_or_else(|_| "http://localhost:8090".to_string());
            let app_state = Arc::new(AppState {
                pool: ctx.pool().clone(),
                metrics: consolidation_metrics,
                gl_base_url,
            });

            http::router()
                .with_state(app_state)
                .merge(http::admin::admin_router(ctx.pool().clone()))
        })
        .run()
        .await
        .expect("consolidation module failed");
}
