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

use treasury::{config::Config, db, http, metrics, outbox, AppState};

#[tokio::main]
async fn main() {
    dotenvy::dotenv().ok();

    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();

    let config = Config::from_env().unwrap_or_else(|err| {
        eprintln!("Configuration error: {}", err);
        eprintln!("Treasury service cannot start without valid configuration.");
        std::process::exit(1);
    });

    tracing::info!(
        "Treasury: config loaded: bus_type={:?}, host={}, port={}",
        config.bus_type,
        config.host,
        config.port
    );

    // Resolve DB pool through the app_id-scoped resolver seam.
    // DATABASE_URL must name the database following the treasury_{app_id}_db convention.
    let pool = db::resolve_pool(&config.database_url)
        .await
        .expect("Treasury: failed to connect to Postgres");

    sqlx::migrate!("./db/migrations")
        .run(&pool)
        .await
        .expect("Treasury: failed to run database migrations");

    tracing::info!("Treasury: database migrations applied");

    // Initialize event bus
    let event_bus: Arc<dyn EventBus> = match config.bus_type {
        treasury::config::BusType::Nats => {
            let nats_url = config
                .nats_url
                .as_ref()
                .expect("NATS_URL must be set when BUS_TYPE=nats");
            tracing::info!("Treasury: connecting to NATS at {}", nats_url);
            let client = async_nats::connect(nats_url)
                .await
                .expect("Treasury: failed to connect to NATS");
            Arc::new(NatsBus::new(client))
        }
        treasury::config::BusType::InMemory => {
            tracing::info!("Treasury: using in-memory event bus");
            Arc::new(InMemoryBus::new())
        }
    };

    // Spawn outbox publisher loop
    let publisher_pool = pool.clone();
    let publisher_bus = event_bus.clone();
    tokio::spawn(async move {
        outbox::run_publisher_task(publisher_pool, publisher_bus).await;
    });
    tracing::info!("Treasury: outbox publisher task started");

    // Metrics
    let treasury_metrics = Arc::new(
        metrics::TreasuryMetrics::new().expect("Treasury: failed to create metrics"),
    );
    tracing::info!("Treasury: metrics initialized");

    let app_state = Arc::new(AppState {
        pool: pool.clone(),
        metrics: treasury_metrics,
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

    let treasury_mutations = Router::new()
        // Accounts — write
        .route("/api/treasury/accounts/bank", post(http::accounts::create_bank_account))
        .route(
            "/api/treasury/accounts/credit-card",
            post(http::accounts::create_credit_card_account),
        )
        .route("/api/treasury/accounts/:id", put(http::accounts::update_account))
        .route("/api/treasury/accounts/:id/deactivate", post(http::accounts::deactivate_account))
        // Reconciliation — write
        .route("/api/treasury/recon/auto-match", post(http::recon::auto_match))
        .route("/api/treasury/recon/manual-match", post(http::recon::manual_match))
        .route("/api/treasury/recon/gl-link", post(http::recon_gl::link_to_gl))
        .route("/api/treasury/recon/gl-unmatched-entries", post(http::recon_gl::unmatched_gl_entries))
        // Statement import — write
        .route("/api/treasury/statements/import", post(http::import::import_statement))
        .route_layer(RequirePermissionsLayer::new(&[permissions::TREASURY_MUTATE]))
        .with_state(app_state.clone());

    let app = Router::new()
        .route("/api/health", get(http::health))
        .route("/api/ready", get(http::ready))
        .route("/api/version", get(http::version))
        .route("/metrics", get(metrics::metrics_handler))
        // Accounts — read
        .route("/api/treasury/accounts", get(http::accounts::list_accounts))
        .route("/api/treasury/accounts/:id", get(http::accounts::get_account))
        // Reports — read
        .route("/api/treasury/cash-position", get(http::reports::cash_position))
        .route("/api/treasury/forecast", get(http::reports::forecast))
        // Reconciliation — read
        .route("/api/treasury/recon/matches", get(http::recon::list_matches))
        .route("/api/treasury/recon/unmatched", get(http::recon::list_unmatched))
        .route("/api/treasury/recon/gl-unmatched-txns", get(http::recon_gl::unmatched_bank_txns))
        .layer(axum::middleware::from_fn_with_state(
            app_state.clone(),
            metrics::latency_layer,
        ))
        .with_state(app_state)
        .merge(treasury_mutations)
        .merge(http::admin::admin_router(pool.clone()))
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

    tracing::info!("Treasury service listening on {addr}");

    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .expect("Treasury: failed to bind address");

    axum::serve(listener, app)
        .await
        .expect("Treasury: failed to start server");
}
