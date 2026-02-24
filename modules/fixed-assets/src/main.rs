use axum::{extract::DefaultBodyLimit, http::Method, routing::{delete, get, post, put}, Extension, Router};
use event_bus::{EventBus, InMemoryBus, NatsBus};
use security::{
    middleware::{default_rate_limiter, rate_limit_middleware, timeout_middleware, DEFAULT_BODY_LIMIT},
    optional_claims_mw, permissions, JwtVerifier, RequirePermissionsLayer,
};
use std::net::SocketAddr;
use std::sync::Arc;
use tower_http::cors::CorsLayer;
use tracing_subscriber::EnvFilter;

use fixed_assets::{config::Config, consumers, db, http, metrics, outbox, AppState};

#[tokio::main]
async fn main() {
    dotenvy::dotenv().ok();

    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();

    let config = Config::from_env().unwrap_or_else(|err| {
        eprintln!("Configuration error: {}", err);
        eprintln!("Fixed Assets service cannot start without valid configuration.");
        std::process::exit(1);
    });

    tracing::info!(
        "Fixed Assets: config loaded: bus_type={:?}, host={}, port={}",
        config.bus_type,
        config.host,
        config.port
    );

    // Resolve DB pool through the app_id-scoped resolver seam.
    // DATABASE_URL must name the database following the fa_{app_id}_db convention.
    let pool = db::resolve_pool(&config.database_url)
        .await
        .expect("Fixed Assets: failed to connect to Postgres");

    sqlx::migrate!("./db/migrations")
        .run(&pool)
        .await
        .expect("Fixed Assets: failed to run database migrations");

    tracing::info!("Fixed Assets: database migrations applied");

    // Initialize event bus
    let event_bus: Arc<dyn EventBus> = match config.bus_type {
        fixed_assets::config::BusType::Nats => {
            let nats_url = config
                .nats_url
                .as_ref()
                .expect("NATS_URL must be set when BUS_TYPE=nats");
            tracing::info!("Fixed Assets: connecting to NATS at {}", nats_url);
            let client = async_nats::connect(nats_url)
                .await
                .expect("Fixed Assets: failed to connect to NATS");
            Arc::new(NatsBus::new(client))
        }
        fixed_assets::config::BusType::InMemory => {
            tracing::info!("Fixed Assets: using in-memory event bus");
            Arc::new(InMemoryBus::new())
        }
    };

    // Spawn outbox publisher loop
    let publisher_pool = pool.clone();
    let publisher_bus = event_bus.clone();
    tokio::spawn(async move {
        outbox::run_publisher_task(publisher_pool, publisher_bus).await;
    });
    tracing::info!("Fixed Assets: outbox publisher task started");

    // Spawn AP bill approved consumer (capitalization from AP)
    consumers::ap_bill_approved::start_ap_bill_approved_consumer(
        event_bus.clone(),
        pool.clone(),
    )
    .await;
    tracing::info!("Fixed Assets: AP bill approved consumer started");

    // Metrics
    let fa_metrics = Arc::new(
        metrics::FixedAssetsMetrics::new().expect("Fixed Assets: failed to create metrics"),
    );
    tracing::info!("Fixed Assets: metrics initialized");

    let app_state = Arc::new(AppState {
        pool: pool.clone(),
        metrics: fa_metrics,
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

    let maybe_verifier = JwtVerifier::from_env_with_overlap().map(Arc::new);

    let fa_mutations = Router::new()
        // Category CRUD — write
        .route("/api/fixed-assets/categories", post(http::assets::create_category))
        .route("/api/fixed-assets/categories/{tenant_id}/{id}", put(http::assets::update_category).delete(http::assets::deactivate_category))
        // Asset CRUD — write
        .route("/api/fixed-assets/assets", post(http::assets::create_asset))
        .route("/api/fixed-assets/assets/{tenant_id}/{id}", put(http::assets::update_asset).delete(http::assets::deactivate_asset))
        // Depreciation — write
        .route("/api/fixed-assets/depreciation/schedule", post(http::depreciation::generate_schedule))
        .route("/api/fixed-assets/depreciation/runs", post(http::depreciation::create_run))
        // Disposals — write
        .route("/api/fixed-assets/disposals", post(http::disposals::dispose_asset))
        .route_layer(RequirePermissionsLayer::new(&[permissions::FIXED_ASSETS_MUTATE]))
        .with_state(app_state.clone());

    let app = Router::new()
        .route("/healthz", get(health::healthz))
        .route("/api/health", get(http::health))
        .route("/api/ready", get(http::ready))
        .route("/api/version", get(http::version))
        .route("/metrics", get(metrics::metrics_handler))
        // Category CRUD — read
        .route("/api/fixed-assets/categories/{tenant_id}/{id}", get(http::assets::get_category))
        .route("/api/fixed-assets/categories/{tenant_id}", get(http::assets::list_categories))
        // Asset CRUD — read
        .route("/api/fixed-assets/assets/{tenant_id}/{id}", get(http::assets::get_asset))
        .route("/api/fixed-assets/assets/{tenant_id}", get(http::assets::list_assets))
        // Depreciation — read
        .route("/api/fixed-assets/depreciation/runs/{tenant_id}", get(http::depreciation::list_runs))
        .route("/api/fixed-assets/depreciation/runs/{tenant_id}/{id}", get(http::depreciation::get_run))
        // Disposals — read
        .route("/api/fixed-assets/disposals/{tenant_id}", get(http::disposals::list_disposals))
        .route("/api/fixed-assets/disposals/{tenant_id}/{id}", get(http::disposals::get_disposal))
        .with_state(app_state)
        .merge(fa_mutations)
        .merge(http::admin::admin_router(pool.clone()))
        .layer(DefaultBodyLimit::max(DEFAULT_BODY_LIMIT))
        .layer(axum::middleware::from_fn(security::tracing::tracing_context_middleware))
        .layer(axum::middleware::from_fn(timeout_middleware))
        .layer(axum::middleware::from_fn(rate_limit_middleware))
        .layer(Extension(default_rate_limiter()))
        .layer(axum::middleware::from_fn_with_state(maybe_verifier, optional_claims_mw))
        .layer(cors)
        .into_make_service_with_connect_info::<SocketAddr>();

    let addr: SocketAddr = format!("{}:{}", config.host, config.port)
        .parse()
        .expect("Invalid HOST:PORT");

    tracing::info!("Fixed Assets service listening on {addr}");

    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .expect("Fixed Assets: failed to bind address");

    axum::serve(listener, app)
        .await
        .expect("Fixed Assets: failed to start server");
}
