use std::sync::Arc;

use platform_sdk::ModuleBuilder;
use ttp_rs::clients::ar::ArClient;
use ttp_rs::clients::tenant_registry::TenantRegistryClient;
use ttp_rs::{http, metrics, AppState};

static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./db/migrations");

#[tokio::main]
async fn main() {
    ModuleBuilder::from_manifest("module.toml")
        .migrator(&MIGRATOR)
        .routes(|ctx| {
            let ttp_metrics =
                Arc::new(metrics::TtpMetrics::new().expect("TTP: failed to create metrics"));

            let registry_client = ctx.platform_client::<TenantRegistryClient>();
            let ar_client = ctx.platform_client::<ArClient>();

            let app_state = Arc::new(AppState {
                pool: ctx.pool().clone(),
                metrics: ttp_metrics,
                registry_client,
                ar_client,
            });

            http::router(app_state)
        })
        .run()
        .await
        .expect("ttp module failed");
}
