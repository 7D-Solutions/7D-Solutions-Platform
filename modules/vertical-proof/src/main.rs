use axum::{routing::get, Json, Router};
use platform_sdk::{ConsumerError, ModuleBuilder};
use tracing;

use vertical_proof::wiring_test;

static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./db/migrations");

#[tokio::main]
async fn main() {
    ModuleBuilder::from_manifest("module.toml")
        .migrator(&MIGRATOR)
        .consumer(
            "ar.events.invoice.opened",
            |_ctx, env| async move {
                tracing::info!(
                    event_type = %env.event_type,
                    tenant = %env.tenant_id,
                    "vertical-proof consumed AR invoice.opened event"
                );
                Ok::<(), ConsumerError>(())
            },
        )
        .routes(|ctx| {
            let pool = ctx.pool().clone();

            Router::new()
                .route(
                    "/api/vertical-proof/wiring-test",
                    get({
                        let pool = pool.clone();
                        move || async move {
                            let results = wiring_test::run_all(&pool).await;
                            let summary = results.summary();
                            tracing::info!("\n{}", summary);
                            Json(serde_json::json!({
                                "all_passed": results.all_passed(),
                                "party": fmt_result(&results.party),
                                "ar": fmt_result(&results.ar),
                                "inventory": fmt_result(&results.inventory),
                                "production": fmt_result(&results.production),
                                "notifications": fmt_result(&results.notifications),
                                "outbox": fmt_result(&results.outbox),
                            }))
                        }
                    }),
                )
                .route(
                    "/api/vertical-proof/health",
                    get(|| async { Json(serde_json::json!({ "status": "ok" })) }),
                )
        })
        .run()
        .await
        .expect("vertical-proof module failed");
}

fn fmt_result(r: &Result<(), String>) -> serde_json::Value {
    match r {
        Ok(()) => serde_json::json!({ "status": "pass" }),
        Err(e) => serde_json::json!({ "status": "fail", "error": e }),
    }
}
