use axum::{extract::DefaultBodyLimit, routing::get, Extension, Json, Router};
use event_bus::{EventBus, InMemoryBus, NatsBus};
use platform_http_contracts::{ApiError, PaginatedResponse, PaginationMeta};
use security::{
    middleware::{
        default_rate_limiter, rate_limit_middleware, timeout_middleware, DEFAULT_BODY_LIMIT,
    },
    optional_claims_mw, permissions, JwtVerifier, RequirePermissionsLayer,
};
use shipping_receiving_rs::{
    config::Config,
    db::inspection_routing_repo::InspectionRoutingRow,
    domain::{
        inspection_routing::RouteLineRequest,
        shipments::{Direction, Shipment},
    },
    http::shipments::types::{
        AddLineRequest, CreateShipmentRequest, ReceiveLineRequest,
        ShipLineQtyRequest, ShipmentLineRow, TransitionStatusRequest,
    },
    metrics, outbox, routes, AppState,
};
use std::net::SocketAddr;
use std::sync::Arc;
use tower_http::cors::{AllowOrigin, CorsLayer};
use tracing_subscriber::EnvFilter;
use utoipa::OpenApi;

#[derive(OpenApi)]
#[openapi(
    info(
        title = "Shipping & Receiving Service",
        version = "2.1.0",
        description = "Inbound and outbound shipment tracking with inventory integration.\n\n\
                        **Authentication:** Bearer JWT. Tenant identity derived from JWT claims.\n\n\
                        **Multi-tenancy:** All data scoped by tenant_id from JWT.",
    ),
    paths(
        shipping_receiving_rs::http::shipments::create_shipment,
        shipping_receiving_rs::http::shipments::get_shipment,
        shipping_receiving_rs::http::shipments::list_shipments,
        shipping_receiving_rs::http::shipments::transition_status,
        shipping_receiving_rs::http::shipments::add_line,
        shipping_receiving_rs::http::shipments::receive_line,
        shipping_receiving_rs::http::shipments::ship_line_qty,
        shipping_receiving_rs::http::shipments::close_shipment,
        shipping_receiving_rs::http::shipments::ship_shipment,
        shipping_receiving_rs::http::shipments::deliver_shipment,
        shipping_receiving_rs::http::shipments::accept_line,
        shipping_receiving_rs::http::inspection_routing::route_line,
        shipping_receiving_rs::http::inspection_routing::list_routings,
        shipping_receiving_rs::http::refs::shipments_by_po,
        shipping_receiving_rs::http::refs::lines_by_po_line,
        shipping_receiving_rs::http::refs::shipments_by_source_ref,
    ),
    components(schemas(
        Shipment, Direction,
        CreateShipmentRequest, TransitionStatusRequest,
        AddLineRequest, ReceiveLineRequest, ShipLineQtyRequest,
        ShipmentLineRow,
        RouteLineRequest, InspectionRoutingRow,
        ApiError, PaginatedResponse<Shipment>, PaginationMeta,
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
        eprintln!("Shipping-Receiving service cannot start without valid configuration.");
        std::process::exit(1);
    });

    tracing::info!(
        "Shipping-Receiving: config loaded: bus_type={:?}, host={}, port={}",
        config.bus_type,
        config.host,
        config.port
    );

    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(10)
        .connect(&config.database_url)
        .await
        .expect("Shipping-Receiving: failed to connect to Postgres");

    let shutdown_pool = pool.clone();

    sqlx::migrate!("./db/migrations")
        .run(&pool)
        .await
        .expect("Shipping-Receiving: failed to run database migrations");

    tracing::info!("Shipping-Receiving: database migrations applied");

    let event_bus: Arc<dyn EventBus> = match config.bus_type {
        shipping_receiving_rs::config::BusType::Nats => {
            let nats_url = config
                .nats_url
                .as_ref()
                .expect("NATS_URL must be set when BUS_TYPE=nats");
            tracing::info!("Shipping-Receiving: connecting to NATS at {}", nats_url);
            let client = event_bus::connect_nats(&nats_url)
                .await
                .expect("Shipping-Receiving: failed to connect to NATS");
            Arc::new(NatsBus::new(client))
        }
        shipping_receiving_rs::config::BusType::InMemory => {
            tracing::info!("Shipping-Receiving: using in-memory event bus");
            Arc::new(InMemoryBus::new())
        }
    };

    let publisher_pool = pool.clone();
    let publisher_bus = event_bus.clone();
    tokio::spawn(async move {
        outbox::run_publisher_task(publisher_pool, publisher_bus).await;
    });
    tracing::info!("Shipping-Receiving: outbox publisher task started");

    // Start event consumers
    shipping_receiving_rs::consumers::start_po_approved_consumer(event_bus.clone(), pool.clone())
        .await;
    shipping_receiving_rs::consumers::start_so_released_consumer(event_bus.clone(), pool.clone())
        .await;
    tracing::info!("Shipping-Receiving: event consumers started");

    let metrics = Arc::new(
        metrics::ShippingReceivingMetrics::new().expect("Failed to create metrics registry"),
    );

    let inventory = match &config.inventory_url {
        Some(url) => {
            tracing::info!(
                "Shipping-Receiving: inventory integration HTTP mode → {}",
                url
            );
            shipping_receiving_rs::InventoryIntegration::http(url)
        }
        None => {
            tracing::info!("Shipping-Receiving: inventory integration deterministic mode");
            shipping_receiving_rs::InventoryIntegration::deterministic()
        }
    };

    let app_state = Arc::new(AppState {
        pool,
        metrics,
        inventory,
    });

    let maybe_verifier = JwtVerifier::from_env_with_overlap().map(Arc::new);

    let app = Router::new()
        .route("/healthz", get(health::healthz))
        .route("/api/openapi.json", get(openapi_json))
        .route("/metrics", get(metrics::metrics_handler))
        .merge(routes::build_ops_router())
        .merge(routes::build_router())
        .merge(
            routes::build_mutation_router().route_layer(RequirePermissionsLayer::new(&[
                permissions::SHIPPING_RECEIVING_MUTATE,
            ])),
        )
        .with_state(app_state)
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

    tracing::info!("Shipping-Receiving module listening on {}", addr);

    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .expect("Shipping-Receiving: failed to bind address");

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .expect("Shipping-Receiving: failed to start server");

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
}

#[cfg(test)]
mod tests {
    use super::*;
    use shipping_receiving_rs::config::BusType;

    #[test]
    fn cors_wildcard_parses() {
        let config = Config {
            database_url: "postgresql://localhost/test".to_string(),
            bus_type: BusType::InMemory,
            nats_url: None,
            host: "0.0.0.0".to_string(),
            port: 8103,
            env: "development".to_string(),
            cors_origins: vec!["*".to_string()],
            inventory_url: None,
        };
        let _layer = build_cors_layer(&config);
    }

    #[test]
    fn cors_specific_origins_parse() {
        let config = Config {
            database_url: "postgresql://localhost/test".to_string(),
            bus_type: BusType::InMemory,
            nats_url: None,
            host: "0.0.0.0".to_string(),
            port: 8103,
            env: "development".to_string(),
            cors_origins: vec![
                "http://localhost:3000".to_string(),
                "https://app.example.com".to_string(),
            ],
            inventory_url: None,
        };
        let _layer = build_cors_layer(&config);
    }
}
