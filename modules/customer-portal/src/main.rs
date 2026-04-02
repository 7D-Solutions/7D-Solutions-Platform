use axum::Extension;
use customer_portal::{auth::PortalJwt, build_router, config::Config, metrics::PortalMetrics, AppState};
use std::sync::Arc;
use platform_sdk::ModuleBuilder;

static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./db/migrations");

#[tokio::main]
async fn main() {
    ModuleBuilder::from_manifest("module.toml")
        .migrator(&MIGRATOR)
        .routes(|ctx| {
            let config = Config::from_env().unwrap_or_else(|err| {
                panic!("customer-portal config error: {}", err);
            });
            let metrics = PortalMetrics::new().expect("customer-portal: metrics init failed");
            let portal_jwt = Arc::new(
                PortalJwt::new(
                    &config.portal_jwt_private_key,
                    &config.portal_jwt_public_key,
                )
                .expect("customer-portal: invalid portal JWT keys"),
            );

            let state = Arc::new(AppState {
                pool: ctx.pool().clone(),
                metrics,
                portal_jwt,
                config,
            });

            let doc_mgmt_client = Arc::new(
                ctx.platform_client::<platform_client_doc_mgmt::DistributionsClient>(),
            );

            build_router(state)
                .layer(Extension(doc_mgmt_client))
        })
        .run()
        .await
        .expect("customer-portal module failed");
}
