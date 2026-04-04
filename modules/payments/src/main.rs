use axum::{
    routing::{get, post},
    Json, Router,
};
use utoipa::OpenApi;
use security::{permissions, RequirePermissionsLayer};
use std::sync::Arc;

use payments_rs::{AppState, Config, PaymentsProvider, TilledPaymentProcessor};
use platform_sdk::{ConsumerError, EventEnvelope, ModuleBuilder, ModuleContext};

#[derive(OpenApi)]
#[openapi(
    info(
        title = "Payments Service",
        version = "4.0.0",
        description = "Payment processing: checkout sessions, payment retrieval, and Tilled webhooks.\n\n\
                        **Authentication:** Bearer JWT. Tenant identity derived from JWT claims.",
    ),
    paths(
        payments_rs::http::checkout_sessions::create_checkout_session,
        payments_rs::http::checkout_sessions::get_checkout_session,
        payments_rs::http::checkout_sessions::present_checkout_session,
        payments_rs::http::checkout_sessions::poll_checkout_session_status,
        payments_rs::http::checkout_sessions::tilled_webhook,
        payments_rs::http::payments::get_payment,
        payments_rs::http::health::health,
        payments_rs::http::health::ready,
        payments_rs::http::health::version,
        payments_rs::http::admin::projection_status,
        payments_rs::http::admin::consistency_check,
        payments_rs::http::admin::list_projections,
    ),
    components(schemas(
        payments_rs::http::checkout_sessions::CreateCheckoutSessionRequest,
        payments_rs::http::checkout_sessions::CreateCheckoutSessionResponse,
        payments_rs::http::checkout_sessions::CheckoutSessionStatusResponse,
        payments_rs::http::checkout_sessions::SessionStatusPollResponse,
        payments_rs::http::payments::PaymentResponse,
        payments_rs::http::payments::DataSource,
        payments_rs::http::admin::ProjectionStatusSchema,
        payments_rs::http::admin::CursorStatusSchema,
        payments_rs::http::admin::ConsistencyCheckSchema,
        payments_rs::http::admin::ProjectionSummarySchema,
        platform_http_contracts::ApiError,
        platform_http_contracts::FieldError,
        platform_http_contracts::PaginatedResponse<payments_rs::http::admin::ProjectionSummarySchema>,
        platform_http_contracts::PaginationMeta,
    )),
    security(("bearer" = [])),
    modifiers(&SecurityAddon),
)]
struct ApiDoc;

struct SecurityAddon;
impl utoipa::Modify for SecurityAddon {
    fn modify(&self, openapi: &mut utoipa::openapi::OpenApi) {
        let components = openapi.components.get_or_insert_with(Default::default);
        components.add_security_scheme(
            "bearer",
            utoipa::openapi::security::SecurityScheme::Http(
                utoipa::openapi::security::Http::new(
                    utoipa::openapi::security::HttpAuthScheme::Bearer,
                ),
            ),
        );
    }
}

static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./db/migrations");

#[tokio::main]
async fn main() {
    // Load config eagerly — fail fast if PAYMENTS_PROVIDER is missing or invalid
    let config = Config::from_env().unwrap_or_else(|err| {
        eprintln!("Payments config error: {}", err);
        std::process::exit(1);
    });

    // Create processor from config (currently only Tilled is supported)
    let processor: Arc<dyn payments_rs::PaymentProcessor> = match config.payments_provider {
        PaymentsProvider::Tilled => {
            let api_key = config.tilled_api_key.clone()
                .expect("TILLED_API_KEY required when PAYMENTS_PROVIDER=tilled");
            let account_id = config.tilled_account_id.clone()
                .expect("TILLED_ACCOUNT_ID required when PAYMENTS_PROVIDER=tilled");
            Arc::new(TilledPaymentProcessor::new(api_key, account_id))
        }
    };

    ModuleBuilder::from_manifest("module.toml")
        .migrator(&MIGRATOR)
        .state(processor.clone())
        .state(config.clone())
        .consumer(
            "ar.events.payment.collection.requested",
            on_payment_collection_requested,
        )
        .routes(move |ctx| {
            // Register SLO metrics with global prometheus registry so
            // SDK's /metrics endpoint picks them up via prometheus::gather().
            let _ = prometheus::register(Box::new(
                payments_rs::metrics::PAYMENTS_HTTP_REQUEST_DURATION_SECONDS.clone(),
            ));
            let _ = prometheus::register(Box::new(
                payments_rs::metrics::PAYMENTS_HTTP_REQUESTS_TOTAL.clone(),
            ));
            let _ = prometheus::register(Box::new(
                payments_rs::metrics::PAYMENTS_EVENT_CONSUMER_LAG_MESSAGES.clone(),
            ));
            let _ = prometheus::register(Box::new(
                payments_rs::metrics::PAYMENTS_OUTBOX_QUEUE_DEPTH.clone(),
            ));

            let processor = ctx.state::<Arc<dyn payments_rs::PaymentProcessor>>().clone();
            let config = ctx.state::<Config>().clone();

            let app_state = Arc::new(AppState {
                pool: ctx.pool().clone(),
                processor,
                tilled_api_key: config.tilled_api_key.clone(),
                tilled_account_id: config.tilled_account_id.clone(),
                tilled_webhook_secret: config.tilled_webhook_secret.clone(),
                tilled_webhook_secret_prev: config.tilled_webhook_secret_prev.clone(),
            });

            Router::new()
                .route(
                    "/api/openapi.json",
                    get(|| async { Json(ApiDoc::openapi()) }),
                )
                .route(
                    "/api/payments/webhook/tilled",
                    post(payments_rs::http::checkout_sessions::tilled_webhook),
                )
                .merge(
                    Router::new()
                        .route(
                            "/api/payments/checkout-sessions",
                            post(payments_rs::http::checkout_sessions::create_checkout_session),
                        )
                        .route(
                            "/api/payments/checkout-sessions/{id}",
                            get(payments_rs::http::checkout_sessions::get_checkout_session),
                        )
                        .route(
                            "/api/payments/checkout-sessions/{id}/present",
                            post(payments_rs::http::checkout_sessions::present_checkout_session),
                        )
                        .route(
                            "/api/payments/checkout-sessions/{id}/status",
                            get(payments_rs::http::checkout_sessions::poll_checkout_session_status),
                        )
                        .merge(payments_rs::http::admin::admin_router(app_state.clone()))
                        .route_layer(RequirePermissionsLayer::new(&[
                            permissions::PAYMENTS_MUTATE,
                        ])),
                )
                .with_state(app_state)
        })
        .run()
        .await
        .expect("payments module failed");
}

/// SDK consumer adapter for ar.events.payment.collection.requested.
async fn on_payment_collection_requested(
    ctx: ModuleContext,
    envelope: EventEnvelope<serde_json::Value>,
) -> Result<(), ConsumerError> {
    let pool = ctx.pool();
    let event_id = envelope.event_id;

    // Idempotency check
    let consumer = payments_rs::EventConsumer::new(pool.clone());
    if consumer
        .is_processed(event_id)
        .await
        .map_err(|e| ConsumerError::Processing(e.to_string()))?
    {
        tracing::info!(event_id = %event_id, "Duplicate payment.collection.requested event ignored");
        return Ok(());
    }

    // Parse payload
    let payload: payments_rs::PaymentCollectionRequestedPayload =
        serde_json::from_value(envelope.payload.clone())
            .map_err(|e| ConsumerError::Processing(format!("payload parse: {e}")))?;

    let metadata = payments_rs::handlers::EnvelopeMetadata {
        event_id,
        tenant_id: envelope.tenant_id.to_string(),
        correlation_id: envelope.correlation_id.map(|id| id.to_string()),
    };

    tracing::info!(
        event_id = %event_id,
        "Processing payment.collection.requested event"
    );

    // Business logic — processor selected at startup via PAYMENTS_PROVIDER
    let processor = ctx.state::<Arc<dyn payments_rs::PaymentProcessor>>();
    payments_rs::handle_payment_collection_requested(pool, processor.as_ref(), payload, metadata)
        .await
        .map_err(|e| ConsumerError::Processing(e.to_string()))?;

    // Mark as processed
    consumer
        .mark_processed(
            event_id,
            "ar.events.payment.collection.requested",
            "ar",
        )
        .await
        .map_err(|e| ConsumerError::Processing(e.to_string()))?;

    tracing::info!(event_id = %event_id, "Payment collection request processed");

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn openapi_spec_is_valid_json_with_endpoints() {
        let spec = ApiDoc::openapi();
        let json = serde_json::to_value(&spec).expect("spec serializes to JSON");

        // Has paths
        let paths = json["paths"].as_object().expect("paths is an object");
        assert!(paths.len() >= 5, "expected at least 5 paths, got {}", paths.len());

        // Key endpoints present
        assert!(paths.contains_key("/api/payments/checkout-sessions"));
        assert!(paths.contains_key("/api/payments/checkout-sessions/{id}"));
        assert!(paths.contains_key("/api/payments/webhook/tilled"));
        assert!(paths.contains_key("/api/payments/payments"));

        // Has security scheme
        let schemes = &json["components"]["securitySchemes"];
        assert!(schemes["bearer"].is_object(), "bearer security scheme missing");

        // Has schemas
        let schemas = json["components"]["schemas"].as_object().expect("schemas object");
        assert!(schemas.contains_key("ApiError"), "ApiError schema missing");
        assert!(schemas.contains_key("CreateCheckoutSessionRequest"), "Request schema missing");
    }
}
