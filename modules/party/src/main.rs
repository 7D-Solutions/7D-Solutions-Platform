use axum::{extract::DefaultBodyLimit, routing::get, Extension, Json};
use security::middleware::{
    default_rate_limiter, rate_limit_middleware, timeout_middleware, DEFAULT_BODY_LIMIT,
};
use security::{optional_claims_mw, JwtVerifier};
use std::net::SocketAddr;
use std::sync::Arc;
use tower_http::cors::{AllowOrigin, CorsLayer};
use tracing_subscriber::EnvFilter;
use utoipa::OpenApi;

use party_rs::domain::address::{Address, CreateAddressRequest, UpdateAddressRequest};
use party_rs::domain::contact::{
    Contact, CreateContactRequest, PrimaryContactEntry, SetPrimaryRequest, UpdateContactRequest,
};
use party_rs::domain::party::{
    CreateCompanyRequest, CreateIndividualRequest, ExternalRef, Party, PartyCompany,
    PartyIndividual, PartyView, SearchQuery, UpdatePartyRequest,
};
use party_rs::http::party::{DataResponse, ListPartiesQuery};
use party_rs::{config::Config, db, http, metrics, AppState};
use platform_http_contracts::{ApiError, PaginatedResponse, PaginationMeta};

#[derive(OpenApi)]
#[openapi(
    info(
        title = "Party Service",
        version = "2.1.0",
        description = "Party master data: companies, individuals, contacts, and addresses.\n\n\
                        **Authentication:** Bearer JWT. Tenant identity derived from JWT claims \
                        (not headers). Permission: `party.mutate` for writes, reads are open to \
                        any authenticated caller.\n\n\
                        **Multi-tenancy:** All data is scoped by the `app_id` (tenant) in the JWT. \
                        No X-Tenant-Id header required.",
    ),
    paths(
        // Parties
        party_rs::http::party::create_company,
        party_rs::http::party::create_individual,
        party_rs::http::party::list_parties,
        party_rs::http::party::get_party,
        party_rs::http::party::update_party,
        party_rs::http::party::deactivate_party,
        party_rs::http::party::search_parties,
        // Contacts
        party_rs::http::contacts::create_contact,
        party_rs::http::contacts::list_contacts,
        party_rs::http::contacts::get_contact,
        party_rs::http::contacts::update_contact,
        party_rs::http::contacts::delete_contact,
        party_rs::http::contacts::set_primary,
        party_rs::http::contacts::primary_contacts,
        // Addresses
        party_rs::http::addresses::create_address,
        party_rs::http::addresses::list_addresses,
        party_rs::http::addresses::get_address,
        party_rs::http::addresses::update_address,
        party_rs::http::addresses::delete_address,
    ),
    components(schemas(
        // Party types
        Party, PartyCompany, PartyIndividual, ExternalRef, PartyView,
        CreateCompanyRequest, CreateIndividualRequest, UpdatePartyRequest,
        ListPartiesQuery, SearchQuery,
        // Contact types
        Contact, CreateContactRequest, UpdateContactRequest,
        SetPrimaryRequest, PrimaryContactEntry,
        // Address types
        Address, CreateAddressRequest, UpdateAddressRequest,
        // Shared envelopes
        ApiError, PaginatedResponse<Party>, PaginationMeta,
        DataResponse<Contact>, DataResponse<Address>, DataResponse<PrimaryContactEntry>,
    )),
    security(
        ("bearer" = [])
    ),
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
        eprintln!("Party service cannot start without valid configuration.");
        std::process::exit(1);
    });

    tracing::info!(
        "Party: config loaded: bus_type={:?}, host={}, port={}",
        config.bus_type,
        config.host,
        config.port
    );

    let pool = db::resolve_pool(&config.database_url)
        .await
        .expect("Party: failed to connect to Postgres");

    let shutdown_pool = pool.clone();

    sqlx::migrate!("./db/migrations")
        .run(&pool)
        .await
        .expect("Party: failed to run database migrations");

    tracing::info!("Party: database migrations applied");

    let party_metrics =
        Arc::new(metrics::PartyMetrics::new().expect("Party: failed to create metrics"));
    tracing::info!("Party: metrics initialized");

    let app_state = Arc::new(AppState {
        pool: pool.clone(),
        metrics: party_metrics,
    });

    // Optional JWT verifier for claims extraction (requires JWT_PUBLIC_KEY env var).
    let maybe_verifier = JwtVerifier::from_env_with_overlap().map(Arc::new);

    let app = http::router(app_state)
        .route("/api/openapi.json", get(openapi_json))
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

    tracing::info!("Party service listening on {addr}");

    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .expect("Party: failed to bind address");

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .expect("Party: failed to start server");

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
