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
            "ar.events.ar.invoice_opened",
            |_ctx, env| async move {
                tracing::info!(
                    event_type = %env.event_type,
                    tenant = %env.tenant_id,
                    "vertical-proof consumed AR invoice_opened event"
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
                            let mut map = serde_json::Map::new();
                            map.insert(
                                "all_passed".into(),
                                serde_json::Value::Bool(results.all_passed()),
                            );
                            for (name, result) in results.as_slice() {
                                map.insert(
                                    name.to_lowercase().replace('-', "_"),
                                    fmt_result(result),
                                );
                            }
                            Json(serde_json::Value::Object(map))
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
