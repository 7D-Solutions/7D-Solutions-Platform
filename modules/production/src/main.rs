use axum::{extract::DefaultBodyLimit, routing::{get, post}, Extension, Json, Router};
use security::{
    middleware::{
        default_rate_limiter, rate_limit_middleware, timeout_middleware, DEFAULT_BODY_LIMIT,
    },
    optional_claims_mw, permissions, JwtVerifier, RequirePermissionsLayer,
};
use std::net::SocketAddr;
use std::sync::Arc;
use tower_http::cors::{AllowOrigin, CorsLayer};
use tracing_subscriber::EnvFilter;
use utoipa::OpenApi;

use production_rs::{
    db::resolver::resolve_pool,
    http::health::{health as health_fn, ready, version},
    http::component_issue,
    http::downtime,
    http::fg_receipt,
    http::operations,
    http::routings,
    http::time_entries,
    http::work_orders,
    http::workcenters,
    metrics::{metrics_handler, ProductionMetrics},
    AppState, Config,
    domain::component_issue::{RequestComponentIssueRequest, ComponentIssueItemInput},
    domain::downtime::{WorkcenterDowntime, StartDowntimeRequest, EndDowntimeRequest},
    domain::fg_receipt::RequestFgReceiptRequest,
    domain::operations::OperationInstance,
    domain::routings::{
        RoutingTemplate, RoutingStep, CreateRoutingRequest, UpdateRoutingRequest,
        AddRoutingStepRequest,
    },
    domain::time_entries::{TimeEntry, StartTimerRequest, StopTimerRequest, ManualEntryRequest},
    domain::work_orders::{WorkOrder, WorkOrderStatus, CreateWorkOrderRequest},
    domain::workcenters::{Workcenter, CreateWorkcenterRequest, UpdateWorkcenterRequest},
    http::pagination::PaginationQuery,
    http::routings::ItemDateQuery,
};
use platform_http_contracts::{ApiError, PaginatedResponse, PaginationMeta};

#[derive(OpenApi)]
#[openapi(
    info(
        title = "Production Service",
        version = "2.1.0",
        description = "Production execution: work orders, operations, workcenters, routing, \
                        component issue/receipt workflows, time entries, downtime tracking.\n\n\
                        **Authentication:** Bearer JWT. Tenant identity derived from JWT claims \
                        (not headers). Permissions: `PRODUCTION_READ` for queries, \
                        `PRODUCTION_MUTATE` for writes.\n\n\
                        **Events:** All state mutations are published to the outbox table for \
                        downstream consumers.",
    ),
    paths(
        production_rs::http::workcenters::create_workcenter,
        production_rs::http::workcenters::get_workcenter,
        production_rs::http::workcenters::list_workcenters,
        production_rs::http::workcenters::update_workcenter,
        production_rs::http::workcenters::deactivate_workcenter,
        production_rs::http::work_orders::create_work_order,
        production_rs::http::work_orders::release_work_order,
        production_rs::http::work_orders::close_work_order,
        production_rs::http::work_orders::get_work_order,
        production_rs::http::operations::initialize_operations,
        production_rs::http::operations::start_operation,
        production_rs::http::operations::complete_operation,
        production_rs::http::operations::list_operations,
        production_rs::http::time_entries::start_timer,
        production_rs::http::time_entries::stop_timer,
        production_rs::http::time_entries::manual_entry,
        production_rs::http::time_entries::list_time_entries,
        production_rs::http::routings::create_routing,
        production_rs::http::routings::get_routing,
        production_rs::http::routings::list_routings,
        production_rs::http::routings::find_routings_by_item,
        production_rs::http::routings::update_routing,
        production_rs::http::routings::release_routing,
        production_rs::http::routings::add_routing_step,
        production_rs::http::routings::list_routing_steps,
        production_rs::http::downtime::start_downtime,
        production_rs::http::downtime::end_downtime,
        production_rs::http::downtime::list_active_downtime,
        production_rs::http::downtime::list_workcenter_downtime,
        production_rs::http::component_issue::post_component_issue,
        production_rs::http::fg_receipt::post_fg_receipt,
    ),
    components(schemas(
        Workcenter, CreateWorkcenterRequest, UpdateWorkcenterRequest,
        WorkOrder, WorkOrderStatus, CreateWorkOrderRequest,
        OperationInstance,
        TimeEntry, StartTimerRequest, StopTimerRequest, ManualEntryRequest,
        WorkcenterDowntime, StartDowntimeRequest, EndDowntimeRequest,
        RoutingTemplate, RoutingStep, CreateRoutingRequest, UpdateRoutingRequest,
        AddRoutingStepRequest,
        RequestComponentIssueRequest, ComponentIssueItemInput,
        RequestFgReceiptRequest,
        ApiError, PaginatedResponse<Workcenter>, PaginatedResponse<RoutingTemplate>,
        PaginatedResponse<WorkcenterDowntime>, PaginationMeta, PaginationQuery,
        ItemDateQuery,
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
    tracing::info!("Starting Production service...");

    let config = Config::from_env().unwrap_or_else(|err| {
        eprintln!("Configuration error: {}", err);
        std::process::exit(1);
    });
    tracing::info!("Configuration loaded: host={}, port={}", config.host, config.port);

    let pool = resolve_pool(&config.database_url).await.expect("Failed to connect to database");
    sqlx::migrate!("db/migrations").run(&pool).await.expect("Failed to run database migrations");

    let shutdown_pool = pool.clone();
    let metrics = Arc::new(ProductionMetrics::new().expect("Failed to create metrics registry"));
    let app_state = Arc::new(AppState { pool, metrics });
    let maybe_verifier = JwtVerifier::from_env_with_overlap().map(Arc::new);

    let prod_mutations = Router::new()
        .route("/api/production/workcenters", post(workcenters::create_workcenter))
        .route("/api/production/workcenters/{id}", axum::routing::put(workcenters::update_workcenter))
        .route("/api/production/workcenters/{id}/deactivate", post(workcenters::deactivate_workcenter))
        .route("/api/production/work-orders", post(work_orders::create_work_order))
        .route("/api/production/work-orders/{id}/release", post(work_orders::release_work_order))
        .route("/api/production/work-orders/{id}/close", post(work_orders::close_work_order))
        .route("/api/production/work-orders/{id}/component-issues", post(component_issue::post_component_issue))
        .route("/api/production/work-orders/{id}/fg-receipt", post(fg_receipt::post_fg_receipt))
        .route("/api/production/work-orders/{id}/operations/initialize", post(operations::initialize_operations))
        .route("/api/production/work-orders/{wo_id}/operations/{op_id}/start", post(operations::start_operation))
        .route("/api/production/work-orders/{wo_id}/operations/{op_id}/complete", post(operations::complete_operation))
        .route("/api/production/time-entries/start", post(time_entries::start_timer))
        .route("/api/production/time-entries/manual", post(time_entries::manual_entry))
        .route("/api/production/time-entries/{id}/stop", post(time_entries::stop_timer))
        .route("/api/production/routings", post(routings::create_routing))
        .route("/api/production/routings/{id}", axum::routing::put(routings::update_routing))
        .route("/api/production/routings/{id}/release", post(routings::release_routing))
        .route("/api/production/routings/{id}/steps", post(routings::add_routing_step))
        .route("/api/production/workcenters/{id}/downtime/start", post(downtime::start_downtime))
        .route("/api/production/downtime/{id}/end", post(downtime::end_downtime))
        .route_layer(RequirePermissionsLayer::new(&[permissions::PRODUCTION_MUTATE]))
        .with_state(app_state.clone());

    let prod_reads = Router::new()
        .route("/api/production/workcenters", get(workcenters::list_workcenters))
        .route("/api/production/workcenters/{id}", get(workcenters::get_workcenter))
        .route("/api/production/work-orders/{id}", get(work_orders::get_work_order))
        .route("/api/production/work-orders/{id}/time-entries", get(time_entries::list_time_entries))
        .route("/api/production/work-orders/{id}/operations", get(operations::list_operations))
        .route("/api/production/routings", get(routings::list_routings))
        .route("/api/production/routings/by-item", get(routings::find_routings_by_item))
        .route("/api/production/routings/{id}", get(routings::get_routing))
        .route("/api/production/routings/{id}/steps", get(routings::list_routing_steps))
        .route("/api/production/workcenters/{id}/downtime", get(downtime::list_workcenter_downtime))
        .route("/api/production/downtime/active", get(downtime::list_active_downtime))
        .route_layer(RequirePermissionsLayer::new(&[permissions::PRODUCTION_READ]))
        .with_state(app_state.clone());

    let app = Router::new()
        .route("/healthz", get(health::healthz))
        .route("/api/health", get(health_fn))
        .route("/api/ready", get(ready))
        .route("/api/version", get(version))
        .route("/api/openapi.json", get(openapi_json))
        .route("/metrics", get(metrics_handler))
        .with_state(app_state)
        .merge(prod_reads)
        .merge(prod_mutations)
        .layer(DefaultBodyLimit::max(DEFAULT_BODY_LIMIT))
        .layer(axum::middleware::from_fn(security::tracing::tracing_context_middleware))
        .layer(axum::middleware::from_fn(timeout_middleware))
        .layer(axum::middleware::from_fn(rate_limit_middleware))
        .layer(Extension(default_rate_limiter()))
        .layer(axum::middleware::from_fn_with_state(maybe_verifier, optional_claims_mw))
        .layer(build_cors_layer(&config))
        .into_make_service_with_connect_info::<SocketAddr>();

    let addr = SocketAddr::from(([0, 0, 0, 0], config.port));
    tracing::info!("Production service listening on {}", addr);
    let listener = tokio::net::TcpListener::bind(addr).await.expect("Failed to bind address");
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .expect("Server failed to start");

    tracing::info!("Server stopped — closing resources");
    shutdown_pool.close().await;
    tracing::info!("Shutdown complete");
}

async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c().await.expect("failed to install Ctrl+C handler");
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
    let layer = if is_wildcard {
        CorsLayer::new().allow_origin(AllowOrigin::any())
    } else {
        let origins: Vec<_> = config.cors_origins.iter().filter_map(|o| o.parse().ok()).collect();
        CorsLayer::new().allow_origin(origins)
    };
    layer
        .allow_methods(tower_http::cors::Any)
        .allow_headers(tower_http::cors::Any)
}
