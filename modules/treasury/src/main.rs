use axum::{
    routing::{get, post, put},
    Router,
};
use std::sync::Arc;

use treasury::{http, metrics, AppState};
use security::{permissions, RequirePermissionsLayer};
use platform_sdk::ModuleBuilder;

static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./db/migrations");

#[tokio::main]
async fn main() {
    ModuleBuilder::from_manifest("module.toml")
        .migrator(&MIGRATOR)
        .routes(|ctx| {
            let treasury_metrics = Arc::new(
                metrics::TreasuryMetrics::new().expect("Treasury: failed to create metrics"),
            );
            let app_state = Arc::new(AppState {
                pool: ctx.pool().clone(),
                metrics: treasury_metrics,
            });

            let treasury_mutations = Router::new()
                .route(
                    "/api/treasury/accounts/bank",
                    post(http::accounts::create_bank_account),
                )
                .route(
                    "/api/treasury/accounts/credit-card",
                    post(http::accounts::create_credit_card_account),
                )
                .route(
                    "/api/treasury/accounts/{id}",
                    put(http::accounts::update_account),
                )
                .route(
                    "/api/treasury/accounts/{id}/deactivate",
                    post(http::accounts::deactivate_account),
                )
                .route(
                    "/api/treasury/recon/auto-match",
                    post(http::recon::auto_match),
                )
                .route(
                    "/api/treasury/recon/manual-match",
                    post(http::recon::manual_match),
                )
                .route(
                    "/api/treasury/recon/gl-link",
                    post(http::recon_gl::link_to_gl),
                )
                .route(
                    "/api/treasury/recon/gl-unmatched-entries",
                    post(http::recon_gl::unmatched_gl_entries),
                )
                .route(
                    "/api/treasury/statements/import",
                    post(http::import::import_statement),
                )
                .route_layer(RequirePermissionsLayer::new(&[
                    permissions::TREASURY_MUTATE,
                ]))
                .with_state(app_state.clone());

            let treasury_reads = Router::new()
                .route("/api/treasury/accounts", get(http::accounts::list_accounts))
                .route(
                    "/api/treasury/accounts/{id}",
                    get(http::accounts::get_account),
                )
                .route(
                    "/api/treasury/cash-position",
                    get(http::reports::cash_position),
                )
                .route("/api/treasury/forecast", get(http::reports::forecast))
                .route(
                    "/api/treasury/recon/matches",
                    get(http::recon::list_matches),
                )
                .route(
                    "/api/treasury/recon/unmatched",
                    get(http::recon::list_unmatched),
                )
                .route(
                    "/api/treasury/recon/gl-unmatched-txns",
                    get(http::recon_gl::unmatched_bank_txns),
                )
                .route_layer(RequirePermissionsLayer::new(&[
                    permissions::TREASURY_READ,
                ]))
                .layer(axum::middleware::from_fn_with_state(
                    app_state.clone(),
                    metrics::latency_layer,
                ))
                .with_state(app_state.clone());

            Router::new()
                .route("/api/openapi.json", get(http::openapi_json))
                .merge(treasury_reads)
                .merge(treasury_mutations)
                .merge(http::admin::admin_router(ctx.pool().clone()))
        })
        .run()
        .await
        .expect("treasury module failed");
}
