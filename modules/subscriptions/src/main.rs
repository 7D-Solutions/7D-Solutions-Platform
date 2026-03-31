use axum::{routing::get, Json, Router};
use std::sync::Arc;
use utoipa::OpenApi;

use subscriptions_rs::{admin, consumer, http, metrics};
use security::{permissions, RequirePermissionsLayer};
use platform_sdk::{ConsumerError, EventEnvelope, ModuleBuilder, ModuleContext};

pub struct AppState {
    pub pool: sqlx::PgPool,
    pub metrics: Arc<metrics::SubscriptionsMetrics>,
}

/// SDK consumer adapter for ar.events.ar.invoice_suspended.
async fn on_invoice_suspended(
    ctx: ModuleContext,
    envelope: EventEnvelope<serde_json::Value>,
) -> Result<(), ConsumerError> {
    let pool = ctx.pool();
    let event_id = envelope.event_id;

    let payload: consumer::InvoiceSuspendedEvent =
        serde_json::from_value(envelope.payload.clone())
            .map_err(|e| ConsumerError::Processing(format!("payload parse: {e}")))?;

    tracing::info!(
        event_id = %event_id,
        tenant_id = %payload.tenant_id,
        customer_id = %payload.customer_id,
        "Processing ar.invoice_suspended"
    );

    let event_id_str = event_id.to_string();
    match consumer::handle_invoice_suspended(pool, &event_id_str, &payload).await {
        Ok(true) => tracing::info!(event_id = %event_id, "Processed ar.invoice_suspended"),
        Ok(false) => tracing::debug!(event_id = %event_id, "Duplicate ar.invoice_suspended, skipped"),
        Err(e) => return Err(ConsumerError::Processing(e.to_string())),
    }

    Ok(())
}

async fn openapi_json() -> Json<utoipa::openapi::OpenApi> {
    Json(http::ApiDoc::openapi())
}

static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./db/migrations");

#[tokio::main]
async fn main() {
    ModuleBuilder::from_manifest("module.toml")
        .migrator(&MIGRATOR)
        .consumer("ar.events.ar.invoice_suspended", on_invoice_suspended)
        .routes(|ctx| {
            let subs_metrics =
                Arc::new(metrics::SubscriptionsMetrics::new().expect("Failed to create metrics"));

            let app_state = Arc::new(AppState {
                pool: ctx.pool().clone(),
                metrics: subs_metrics,
            });

            Router::new()
                .route("/api/openapi.json", get(openapi_json))
                .merge(
                    http::subscriptions_router(ctx.pool().clone())
                        .route_layer(RequirePermissionsLayer::new(&[
                            permissions::SUBSCRIPTIONS_MUTATE,
                        ])),
                )
                .merge(admin::admin_router(ctx.pool().clone()))
        })
        .run()
        .await
        .expect("subscriptions module failed");
}
