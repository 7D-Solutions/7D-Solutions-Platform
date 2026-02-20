use axum::{extract::DefaultBodyLimit, http::Method, routing::{get, post, put}, Extension, Router};
use security::{optional_claims_mw, permissions, JwtVerifier, RequirePermissionsLayer};
use event_bus::{EventBus, InMemoryBus, NatsBus};
use security::middleware::{
    default_rate_limiter, rate_limit_middleware, timeout_middleware, DEFAULT_BODY_LIMIT,
};
use std::net::SocketAddr;
use std::sync::Arc;
use tower_http::cors::CorsLayer;
use tracing_subscriber::EnvFilter;

use ap::{config::Config, db, http, metrics, outbox, AppState};

#[tokio::main]
async fn main() {
    dotenvy::dotenv().ok();

    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();

    let config = Config::from_env().unwrap_or_else(|err| {
        eprintln!("Configuration error: {}", err);
        eprintln!("AP service cannot start without valid configuration.");
        std::process::exit(1);
    });

    tracing::info!(
        "AP: config loaded: bus_type={:?}, host={}, port={}",
        config.bus_type,
        config.host,
        config.port
    );

    // Resolve DB pool through the app_id-scoped resolver seam.
    // DATABASE_URL must name the database following the ap_{app_id}_db convention.
    let pool = db::resolve_pool(&config.database_url)
        .await
        .expect("AP: failed to connect to Postgres");

    sqlx::migrate!("./db/migrations")
        .run(&pool)
        .await
        .expect("AP: failed to run database migrations");

    tracing::info!("AP: database migrations applied");

    // Initialize event bus
    let event_bus: Arc<dyn EventBus> = match config.bus_type {
        ap::config::BusType::Nats => {
            let nats_url = config
                .nats_url
                .as_ref()
                .expect("NATS_URL must be set when BUS_TYPE=nats");
            tracing::info!("AP: connecting to NATS at {}", nats_url);
            let client = async_nats::connect(nats_url)
                .await
                .expect("AP: failed to connect to NATS");
            Arc::new(NatsBus::new(client))
        }
        ap::config::BusType::InMemory => {
            tracing::info!("AP: using in-memory event bus");
            Arc::new(InMemoryBus::new())
        }
    };

    // Spawn outbox publisher loop
    let publisher_pool = pool.clone();
    let publisher_bus = event_bus.clone();
    tokio::spawn(async move {
        outbox::run_publisher_task(publisher_pool, publisher_bus).await;
    });
    tracing::info!("AP: outbox publisher task started");

    // Metrics
    let ap_metrics = Arc::new(
        metrics::ApMetrics::new().expect("AP: failed to create metrics"),
    );
    tracing::info!("AP: metrics initialized");

    let app_state = Arc::new(AppState {
        pool: pool.clone(),
        metrics: ap_metrics,
    });

    let cors = CorsLayer::new()
        .allow_origin([
            "http://localhost:5173".parse().unwrap(),
            "http://localhost:3000".parse().unwrap(),
        ])
        .allow_methods([Method::GET, Method::POST, Method::PUT, Method::DELETE])
        .allow_headers([
            axum::http::header::CONTENT_TYPE,
            axum::http::header::AUTHORIZATION,
        ])
        .allow_credentials(true);

    let maybe_verifier = JwtVerifier::from_env().map(Arc::new);

    let ap_mutations = Router::new()
        // Vendors — write
        .route("/api/ap/vendors", post(http::vendors::create_vendor))
        .route("/api/ap/vendors/{vendor_id}", put(http::vendors::update_vendor))
        .route("/api/ap/vendors/{vendor_id}/deactivate", post(http::vendors::deactivate_vendor))
        // Purchase orders — write
        .route("/api/ap/pos", post(http::purchase_orders::create_po))
        .route("/api/ap/pos/{po_id}/lines", put(http::purchase_orders::update_po_lines))
        .route("/api/ap/pos/{po_id}/approve", post(http::purchase_orders::approve_po))
        // Bills — write
        .route("/api/ap/bills", post(http::bills::create_bill))
        .route("/api/ap/bills/{bill_id}/match", post(http::bills::match_bill))
        .route("/api/ap/bills/{bill_id}/approve", post(http::bills::approve_bill))
        .route("/api/ap/bills/{bill_id}/void", post(http::bills::void_bill))
        .route("/api/ap/bills/{bill_id}/tax-quote", post(http::bills::quote_bill_tax))
        // Bill allocations — write
        .route(
            "/api/ap/bills/{bill_id}/allocations",
            post(http::allocations::create_allocation),
        )
        // Payment runs — write
        .route("/api/ap/payment-runs", post(http::payment_runs::create_run))
        .route("/api/ap/payment-runs/{run_id}/execute", post(http::payment_runs::execute_run))
        .route_layer(RequirePermissionsLayer::new(&[permissions::AP_MUTATE]))
        .with_state(app_state.clone());

    let app = Router::new()
        .route("/api/health", get(http::health))
        .route("/api/ready", get(http::ready))
        .route("/api/version", get(http::version))
        .route("/metrics", get(metrics::metrics_handler))
        // Vendors — read
        .route("/api/ap/vendors", get(http::vendors::list_vendors))
        .route("/api/ap/vendors/{vendor_id}", get(http::vendors::get_vendor))
        // Purchase orders — read
        .route("/api/ap/pos", get(http::purchase_orders::list_pos))
        .route("/api/ap/pos/{po_id}", get(http::purchase_orders::get_po))
        // Bills — read
        .route("/api/ap/bills", get(http::bills::list_bills))
        .route("/api/ap/bills/{bill_id}", get(http::bills::get_bill))
        // Bill allocations — read
        .route("/api/ap/bills/{bill_id}/allocations", get(http::allocations::list_allocations))
        .route("/api/ap/bills/{bill_id}/balance", get(http::allocations::get_balance))
        // Payment runs — read
        .route("/api/ap/payment-runs/{run_id}", get(http::payment_runs::get_run))
        // Reports — read
        .route("/api/ap/aging", get(http::reports::aging_report))
        .route("/api/ap/tax/reports/summary", get(http::tax_reports::tax_report_summary))
        .route("/api/ap/tax/reports/export", get(http::tax_reports::tax_report_export))
        .with_state(app_state)
        .merge(ap_mutations)
        .merge(http::admin::admin_router(pool))
        .layer(DefaultBodyLimit::max(DEFAULT_BODY_LIMIT))
        .layer(axum::middleware::from_fn(security::tracing::tracing_context_middleware))
        .layer(axum::middleware::from_fn(timeout_middleware))
        .layer(axum::middleware::from_fn(rate_limit_middleware))
        .layer(Extension(default_rate_limiter()))
        .layer(axum::middleware::from_fn_with_state(maybe_verifier, optional_claims_mw))
        .layer(security::AuthzLayer::from_env())
        .layer(cors)
        .into_make_service_with_connect_info::<SocketAddr>();

    let addr: SocketAddr = format!("{}:{}", config.host, config.port)
        .parse()
        .expect("Invalid HOST:PORT");

    tracing::info!("AP service listening on {addr}");

    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .expect("AP: failed to bind address");

    axum::serve(listener, app)
        .await
        .expect("AP: failed to start server");
}
