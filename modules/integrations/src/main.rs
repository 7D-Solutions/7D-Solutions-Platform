use axum::{extract::DefaultBodyLimit, routing::get, Extension, Json, Router};
use event_bus::{EventBus, InMemoryBus, NatsBus};
use security::middleware::{
    default_rate_limiter, rate_limit_middleware, timeout_middleware, DEFAULT_BODY_LIMIT,
};
use security::{optional_claims_mw, JwtVerifier};
use std::net::SocketAddr;
use std::sync::Arc;
use tower_http::cors::{AllowOrigin, CorsLayer};
use tracing_subscriber::EnvFilter;
use utoipa::OpenApi;

use integrations_rs::{
    config::Config, db, domain::oauth::refresh, http, metrics, outbox, AppState,
};
use integrations_rs::config::BusType;
use integrations_rs::domain::connectors::{
    ConfigField, ConfigFieldType, ConnectorCapabilities, ConnectorConfig,
    RegisterConnectorRequest, RunTestActionRequest, TestActionResult,
};
use integrations_rs::domain::external_refs::{
    CreateExternalRefRequest, ExternalRef, UpdateExternalRefRequest,
};
use integrations_rs::domain::oauth::{ConnectionStatus, OAuthConnectionInfo};
use integrations_rs::http::qbo_invoice::{UpdateInvoiceRequest, UpdateInvoiceResponse};
use platform_http_contracts::{ApiError, FieldError, PaginatedResponse, PaginationMeta};

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
        integrations_rs::http::webhooks::inbound_webhook,
        integrations_rs::http::qbo_invoice::update_invoice,
    ),
    components(schemas(
        ExternalRef, CreateExternalRefRequest, UpdateExternalRefRequest,
        ConnectorConfig, ConnectorCapabilities, ConfigField, ConfigFieldType,
        RegisterConnectorRequest, RunTestActionRequest, TestActionResult,
        OAuthConnectionInfo, ConnectionStatus,
        UpdateInvoiceRequest, UpdateInvoiceResponse,
        ApiError, FieldError, PaginatedResponse<ExternalRef>,
        PaginatedResponse<ConnectorCapabilities>, PaginatedResponse<ConnectorConfig>,
        PaginationMeta,
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

#[tokio::main]
async fn main() {
    dotenvy::dotenv().ok();

    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();

    let config = Config::from_env().unwrap_or_else(|err| {
        eprintln!("Configuration error: {}", err);
        eprintln!("Integrations service cannot start without valid configuration.");
        std::process::exit(1);
    });

    tracing::info!(
        "Integrations: config loaded: bus_type={:?}, host={}, port={}",
        config.bus_type,
        config.host,
        config.port
    );

    let pool = db::resolve_pool(&config.database_url)
        .await
        .expect("Integrations: failed to connect to Postgres");

    let shutdown_pool = pool.clone();

    sqlx::migrate!("./db/migrations")
        .run(&pool)
        .await
        .expect("Integrations: failed to run database migrations");

    tracing::info!("Integrations: database migrations applied");

    let event_bus: Arc<dyn EventBus> = match config.bus_type {
        BusType::Nats => {
            let nats_url = config
                .nats_url
                .as_ref()
                .expect("NATS_URL must be set when BUS_TYPE=nats");
            tracing::info!("Integrations: connecting to NATS at {}", nats_url);
            let client = event_bus::connect_nats(nats_url)
                .await
                .expect("Integrations: failed to connect to NATS");
            Arc::new(NatsBus::new(client))
        }
        BusType::InMemory => {
            tracing::info!("Integrations: using in-memory event bus");
            Arc::new(InMemoryBus::new())
        }
    };

    let publisher_pool = pool.clone();
    let publisher_bus = event_bus.clone();
    tokio::spawn(async move {
        outbox::run_publisher_task(publisher_pool, publisher_bus).await;
    });
    tracing::info!("Integrations: outbox publisher task started");

    // OAuth token refresh worker (only if QBO_CLIENT_ID is set)
    let (_shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
    if std::env::var("QBO_CLIENT_ID").is_ok() {
        let refresher: Arc<dyn refresh::TokenRefresher> = Arc::new(refresh::HttpTokenRefresher {
            client: reqwest::Client::new(),
            qbo_client_id: std::env::var("QBO_CLIENT_ID").unwrap_or_default(),
            qbo_client_secret: std::env::var("QBO_CLIENT_SECRET").unwrap_or_default(),
            qbo_token_url: std::env::var("QBO_TOKEN_URL").unwrap_or_else(|_| {
                "https://oauth.platform.intuit.com/oauth2/v1/tokens/bearer".to_string()
            }),
        });
        refresh::spawn_refresh_worker(
            pool.clone(),
            refresher,
            std::time::Duration::from_secs(30),
            shutdown_rx,
        );
        tracing::info!("Integrations: OAuth refresh worker started (30s poll)");

        // CDC polling worker — polls QBO for changes every 15 minutes
        let (_cdc_shutdown_tx, cdc_shutdown_rx) = tokio::sync::watch::channel(false);
        let cdc_interval_secs: u64 = std::env::var("CDC_POLL_INTERVAL_SECS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(integrations_rs::domain::qbo::cdc::DEFAULT_CDC_POLL_INTERVAL_SECS);
        integrations_rs::domain::qbo::cdc::spawn_cdc_worker(
            pool.clone(),
            std::time::Duration::from_secs(cdc_interval_secs),
            cdc_shutdown_rx,
        );
        tracing::info!(
            interval_secs = cdc_interval_secs,
            "Integrations: QBO CDC polling worker started"
        );
    } else {
        tracing::info!("Integrations: OAuth refresh worker skipped (QBO_CLIENT_ID not set)");
    }

    let integrations_metrics = Arc::new(
        metrics::IntegrationsMetrics::new().expect("Integrations: failed to create metrics"),
    );
    tracing::info!("Integrations: metrics initialized");

    let app_state = Arc::new(AppState {
        pool: pool.clone(),
        metrics: integrations_metrics,
        bus: event_bus,
    });

    // Optional JWT verifier for claims extraction (requires JWT_PUBLIC_KEY env var).
    let maybe_verifier = JwtVerifier::from_env_with_overlap().map(Arc::new);

    let app = http::router(app_state)
        .merge(Router::new().route("/api/openapi.json", get(openapi_json)))
        .layer(DefaultBodyLimit::max(DEFAULT_BODY_LIMIT))
        .layer(axum::middleware::from_fn(
            security::tracing::tracing_context_middleware,
        ))
        .layer(axum::middleware::from_fn(timeout_middleware))
        .layer(axum::middleware::from_fn(rate_limit_middleware))
        .layer(Extension(default_rate_limiter()))
        .layer(axum::middleware::from_fn_with_state(
            maybe_verifier,
            optional_claims_mw,
        ))
        .layer(build_cors_layer(&config))
        .into_make_service_with_connect_info::<SocketAddr>();

    let addr: SocketAddr = format!("{}:{}", config.host, config.port)
        .parse()
        .expect("Invalid HOST:PORT");

    tracing::info!("Integrations service listening on {addr}");

    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .expect("Integrations: failed to bind address");

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .expect("Integrations: failed to start server");

    tracing::info!("Server stopped — closing resources");
    shutdown_pool.close().await;
    tracing::info!("Shutdown complete");
}

async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install SIGTERM handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }

    tracing::info!("Shutdown signal received — draining in-flight requests");
}

fn build_cors_layer(config: &Config) -> CorsLayer {
    let is_wildcard = config.cors_origins.len() == 1 && config.cors_origins[0] == "*";

    if is_wildcard && config.env != "development" {
        tracing::warn!(
            "CORS_ORIGINS is set to wildcard — restrict to specific origins in production"
        );
    }

    let layer = if is_wildcard {
        CorsLayer::new().allow_origin(AllowOrigin::any())
    } else {
        let origins: Vec<_> = config
            .cors_origins
            .iter()
            .filter_map(|o| o.parse().ok())
            .collect();
        CorsLayer::new().allow_origin(origins)
    };

    layer
        .allow_methods(tower_http::cors::Any)
        .allow_headers(tower_http::cors::Any)
        .allow_credentials(false)
}
