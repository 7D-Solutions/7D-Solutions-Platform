use std::sync::Arc;

use ttp_rs::{http, metrics, AppState};
use ttp_rs::clients::ar::ArClient;
use ttp_rs::clients::tenant_registry::TenantRegistryClient;
use platform_sdk::ModuleBuilder;

static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./db/migrations");

#[tokio::main]
async fn main() {
    ModuleBuilder::from_manifest("module.toml")
        .migrator(&MIGRATOR)
        .routes(|ctx| {
            let ttp_metrics =
                Arc::new(metrics::TtpMetrics::new().expect("TTP: failed to create metrics"));

            let registry_url = std::env::var("TENANT_REGISTRY_URL")
                .unwrap_or_else(|_| "http://localhost:8092".to_string());
            let ar_url = std::env::var("AR_BASE_URL")
                .unwrap_or_else(|_| "http://localhost:8086".to_string());

            tracing::info!(registry_url = %registry_url, ar_url = %ar_url, "TTP: resolved service URLs at startup");

            let app_state = Arc::new(AppState {
                pool: ctx.pool().clone(),
                metrics: ttp_metrics,
                registry_client: TenantRegistryClient::new(registry_url),
                ar_client: ArClient::new(ar_url),
            });

            http::router(app_state)
        })
        .run()
        .await
        .expect("ttp module failed");
}
