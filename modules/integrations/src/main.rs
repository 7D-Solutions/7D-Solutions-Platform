use axum::{routing::get, Json};
use std::sync::Arc;
use utoipa::OpenApi;

use integrations_rs::{
    domain::connectors::{
        ConfigField, ConfigFieldType, ConnectorCapabilities, ConnectorConfig,
        RegisterConnectorRequest, RunTestActionRequest, TestActionResult,
    },
    domain::external_refs::{CreateExternalRefRequest, ExternalRef, UpdateExternalRefRequest},
    domain::oauth::{refresh, ConnectionStatus, OAuthConnectionInfo},
    http,
    http::qbo_invoice::{UpdateInvoiceRequest, UpdateInvoiceResponse},
    metrics, AppState,
};
use platform_http_contracts::{ApiError, FieldError, PaginatedResponse, PaginationMeta};
use platform_sdk::ModuleBuilder;

#[derive(OpenApi)]
#[openapi(
    info(
        title = "Integrations Service",
        version = "2.3.0",
        description = "External system connectors, webhook routing, OAuth connection management, \
                        and reference linking.\n\n\
                        **Authentication:** Bearer JWT. Tenant identity derived from JWT claims \
                        (not headers). Permissions: `integrations.read` for queries, \
                        `integrations.mutate` for writes.\n\n\
                        **Webhooks:** Inbound webhooks (Stripe, GitHub, QuickBooks) are \
                        unauthenticated and gated by HMAC-SHA256 signature verification.",
    ),
    paths(
        integrations_rs::http::external_refs::create_external_ref,
        integrations_rs::http::external_refs::list_by_entity,
        integrations_rs::http::external_refs::get_by_external,
        integrations_rs::http::external_refs::get_external_ref,
        integrations_rs::http::external_refs::update_external_ref,
        integrations_rs::http::external_refs::delete_external_ref,
        integrations_rs::http::connectors::list_connector_types,
        integrations_rs::http::connectors::register_connector,
        integrations_rs::http::connectors::list_connectors,
        integrations_rs::http::connectors::get_connector,
        integrations_rs::http::connectors::run_connector_test,
        integrations_rs::http::oauth::connect,
        integrations_rs::http::oauth::callback,
        integrations_rs::http::oauth::status,
        integrations_rs::http::oauth::disconnect,
        integrations_rs::http::oauth::import_tokens,
        integrations_rs::http::webhooks::inbound_webhook,
        integrations_rs::http::qbo_invoice::update_invoice,
    ),
    components(schemas(
        ExternalRef, CreateExternalRefRequest, UpdateExternalRefRequest,
        ConnectorConfig, ConnectorCapabilities, ConfigField, ConfigFieldType,
        RegisterConnectorRequest, RunTestActionRequest, TestActionResult,
        OAuthConnectionInfo, ConnectionStatus,
        integrations_rs::http::oauth::ImportTokensRequest,
        UpdateInvoiceRequest, UpdateInvoiceResponse,
        ApiError, FieldError,
        PaginatedResponse<ExternalRef>, PaginatedResponse<ConnectorConfig>, PaginationMeta,
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
                utoipa::openapi::security::HttpBuilder::new()
                    .scheme(utoipa::openapi::security::HttpAuthScheme::Bearer)
                    .bearer_format("JWT")
                    .build(),
            ),
        );
    }
}

async fn openapi_json() -> Json<utoipa::openapi::OpenApi> {
    Json(ApiDoc::openapi())
}

static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./db/migrations");

/// Validate the QBO env contract before any worker starts.
///
/// Called only when QBO_CLIENT_ID is present (i.e. QBO integration is enabled).
/// Panics with an actionable message listing every missing or invalid var so ops
/// never gets a silent misconfiguration.
fn validate_qbo_env() {
    const REQUIRED: &[&str] = &[
        "QBO_CLIENT_ID",
        "QBO_CLIENT_SECRET",
        "QBO_REDIRECT_URI",
        "OAUTH_ENCRYPTION_KEY",
    ];

    let missing: Vec<&str> = REQUIRED
        .iter()
        .filter(|var| {
            std::env::var(var)
                .map_or(true, |v| v.is_empty())
        })
        .copied()
        .collect();

    if !missing.is_empty() {
        panic!(
            "Startup validation failed: QBO is enabled (QBO_CLIENT_ID is set) but required \
             env vars are missing or empty: {}",
            missing.join(", ")
        );
    }

    let redirect_uri = std::env::var("QBO_REDIRECT_URI")
        .expect("QBO_REDIRECT_URI presence already validated above");
    if !redirect_uri.starts_with("https://") && !redirect_uri.starts_with("http://localhost") {
        panic!(
            "Startup validation failed: QBO_REDIRECT_URI '{}' is invalid — must start with \
             https:// (or http://localhost for dev)",
            redirect_uri
        );
    }

    // Production requires a real NATS bus — in-memory bus silently drops sync events.
    let env_name = std::env::var("ENV").unwrap_or_default();
    if env_name == "production" {
        let bus_type = std::env::var("BUS_TYPE").unwrap_or_default().to_lowercase();
        if bus_type == "inmemory" || bus_type.is_empty() {
            panic!(
                "Startup validation failed: BUS_TYPE=inmemory is not allowed in production. \
                 Set BUS_TYPE=nats and NATS_URL to a reachable NATS server. \
                 Sync events (authority changes, conflict notifications) would be silently dropped."
            );
        }
        let nats_url = std::env::var("NATS_URL").unwrap_or_default();
        if nats_url.is_empty() {
            panic!(
                "Startup validation failed: NATS_URL is required in production when QBO is enabled. \
                 Sync events cannot be delivered without a NATS connection."
            );
        }
    }
}

#[tokio::main]
async fn main() {
    ModuleBuilder::from_manifest("module.toml")
        .migrator(&MIGRATOR)
        .routes(|ctx| {
            let bus = ctx.bus_arc().expect("Integrations requires event bus");

            // Spawn conditional background workers
            if std::env::var("QBO_CLIENT_ID").is_ok() {
                validate_qbo_env();
                let refresher: Arc<dyn refresh::TokenRefresher> =
                    Arc::new(refresh::HttpTokenRefresher {
                        client: reqwest::Client::new(),
                        qbo_client_id: std::env::var("QBO_CLIENT_ID").unwrap_or_default(),
                        qbo_client_secret: std::env::var("QBO_CLIENT_SECRET").unwrap_or_default(),
                        qbo_token_url: std::env::var("QBO_TOKEN_URL").unwrap_or_else(|_| {
                            "https://oauth.platform.intuit.com/oauth2/v1/tokens/bearer".to_string()
                        }),
                    });
                let (_shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
                refresh::spawn_refresh_worker(
                    ctx.pool().clone(),
                    refresher,
                    std::time::Duration::from_secs(30),
                    shutdown_rx,
                );
                tracing::info!("Integrations: OAuth refresh worker started (30s poll)");

                let (_cdc_shutdown_tx, cdc_shutdown_rx) = tokio::sync::watch::channel(false);
                let cdc_interval_secs: u64 = std::env::var("CDC_POLL_INTERVAL_SECS")
                    .ok()
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(integrations_rs::domain::qbo::cdc::DEFAULT_CDC_POLL_INTERVAL_SECS);
                integrations_rs::domain::qbo::cdc::spawn_cdc_worker(
                    ctx.pool().clone(),
                    std::time::Duration::from_secs(cdc_interval_secs),
                    cdc_shutdown_rx,
                );
                tracing::info!(
                    interval_secs = cdc_interval_secs,
                    "Integrations: QBO CDC polling worker started"
                );

                if integrations_rs::domain::qbo::outbound::legacy_consumers_enabled() {
                    let (_outbound_shutdown_tx, outbound_shutdown_rx) =
                        tokio::sync::watch::channel(false);
                    integrations_rs::domain::qbo::outbound::spawn_outbound_consumer(
                        ctx.pool().clone(),
                        bus.clone(),
                        outbound_shutdown_rx,
                    );
                    tracing::info!("Integrations: QBO outbound consumer started");

                    let (_order_ingested_shutdown_tx, order_ingested_shutdown_rx) =
                        tokio::sync::watch::channel(false);
                    integrations_rs::domain::qbo::outbound::spawn_order_ingested_consumer(
                        ctx.pool().clone(),
                        bus.clone(),
                        order_ingested_shutdown_rx,
                    );
                    tracing::info!("Integrations: QBO order-ingested consumer started");
                } else {
                    tracing::info!(
                        "Integrations: QBO legacy outbound consumers disabled \
                         (QBO_LEGACY_CONSUMERS_ENABLED != 1) — set to 1 to re-enable"
                    );
                }
            }

            integrations_rs::domain::file_jobs::ebay_fulfillment::start_ebay_fulfillment_consumer(
                bus.clone(),
                ctx.pool().clone(),
            );
            tracing::info!("Integrations: eBay fulfillment consumer started");

            tokio::spawn(integrations_rs::domain::sync::push_attempts::run_watchdog_task(
                ctx.pool().clone(),
            ));
            tracing::info!("Integrations: push-attempt watchdog started (60s poll, 10min timeout)");

            let integrations_metrics = Arc::new(
                metrics::IntegrationsMetrics::new()
                    .expect("Integrations: failed to create metrics"),
            );

            let app_state = Arc::new(AppState {
                pool: ctx.pool().clone(),
                metrics: integrations_metrics,
                bus,
            });

            http::router(app_state).route("/api/openapi.json", get(openapi_json))
        })
        .run()
        .await
        .expect("integrations module failed");
}
