use ::event_bus::{EventBus, InMemoryBus, NatsBus};
use axum::{
    extract::DefaultBodyLimit,
    routing::{get, post, put},
    Extension, Router,
};
use pdf_editor_rs::{config, config::Config, cors::build_cors_layer, db, event_bus, http as handlers, metrics};
use security::{
    middleware::{
        default_rate_limiter, rate_limit_middleware, timeout_middleware, DEFAULT_BODY_LIMIT,
    },
    optional_claims_mw, permissions, JwtVerifier, RequirePermissionsLayer,
};
use std::net::SocketAddr;
use std::sync::Arc;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() {
    dotenvy::dotenv().ok();

    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();

    let config = Config::from_env().unwrap_or_else(|err| {
        eprintln!("Configuration error: {}", err);
        eprintln!("PDF Editor service cannot start without valid configuration.");
        std::process::exit(1);
    });

    tracing::info!(
        "Configuration loaded: bus_type={:?}, host={}, port={}",
        config.bus_type,
        config.host,
        config.port
    );

    // Database
    let db = db::resolver::resolve_pool(&config.database_url)
        .await
        .expect("Failed to connect to Postgres");

    let shutdown_pool = db.clone();

    // Run migrations
    sqlx::migrate!("./db/migrations")
        .run(&db)
        .await
        .expect("Failed to run database migrations");

    tracing::info!("Database migrations applied successfully");

    // Event bus
    let bus: Arc<dyn EventBus> = match config.bus_type {
        config::BusType::Nats => {
            let nats_url = config
                .nats_url
                .as_ref()
                .expect("NATS_URL must be set when BUS_TYPE=nats");
            tracing::info!("Connecting to NATS at {}", nats_url);
            let nats_client = ::event_bus::connect_nats(nats_url)
                .await
                .expect("Failed to connect to NATS");
            Arc::new(NatsBus::new(nats_client))
        }
        config::BusType::InMemory => {
            tracing::info!("Using InMemoryBus for event messaging");
            Arc::new(InMemoryBus::new())
        }
    };

    // Spawn outbox publisher task
    tokio::spawn(event_bus::start_outbox_publisher(db.clone(), bus.clone()));

    // Build CORS layer from config
    let cors = build_cors_layer(&config);

    // HTTP server
    let maybe_verifier = JwtVerifier::from_env_with_overlap().map(Arc::new);

    // Mutation handlers — require pdf_editor.mutate permission.
    let mutations = Router::new()
        // Form templates — write
        .route(
            "/api/pdf/forms/templates",
            post(handlers::templates::create_template),
        )
        .route(
            "/api/pdf/forms/templates/{id}",
            put(handlers::templates::update_template),
        )
        // Form fields — write
        .route(
            "/api/pdf/forms/templates/{id}/fields",
            post(handlers::fields::create_field),
        )
        .route(
            "/api/pdf/forms/templates/{tid}/fields/{fid}",
            put(handlers::fields::update_field),
        )
        .route(
            "/api/pdf/forms/templates/{id}/fields/reorder",
            post(handlers::fields::reorder_fields),
        )
        // Form submissions — write
        .route(
            "/api/pdf/forms/submissions",
            post(handlers::submissions::create_submission),
        )
        .route(
            "/api/pdf/forms/submissions/{id}",
            put(handlers::submissions::autosave_submission),
        )
        .route(
            "/api/pdf/forms/submissions/{id}/submit",
            post(handlers::submissions::submit_submission),
        )
        .route(
            "/api/pdf/forms/submissions/{id}/generate",
            post(handlers::generate::generate_pdf),
        )
        .route_layer(RequirePermissionsLayer::new(&[
            permissions::PDF_EDITOR_MUTATE,
        ]))
        .with_state(db.clone());

    // Read handlers — require pdf_editor.read permission.
    let reads = Router::new()
        // Form templates — read
        .route(
            "/api/pdf/forms/templates",
            get(handlers::templates::list_templates),
        )
        .route(
            "/api/pdf/forms/templates/{id}",
            get(handlers::templates::get_template),
        )
        // Form fields — read
        .route(
            "/api/pdf/forms/templates/{id}/fields",
            get(handlers::fields::list_fields),
        )
        // Form submissions — read
        .route(
            "/api/pdf/forms/submissions",
            get(handlers::submissions::list_submissions),
        )
        .route(
            "/api/pdf/forms/submissions/{id}",
            get(handlers::submissions::get_submission),
        )
        .route_layer(RequirePermissionsLayer::new(&[
            permissions::PDF_EDITOR_READ,
        ]))
        .with_state(db.clone());

    // Health/metrics — public, no auth required.
    let health_routes = Router::new()
        .route("/healthz", get(health::healthz))
        .route("/api/health", get(handlers::health::health))
        .route("/api/ready", get(handlers::health::ready))
        .route("/api/version", get(handlers::health::version))
        .route("/metrics", get(metrics::metrics_handler))
        .with_state(db.clone());

    let app = Router::new()
        .merge(mutations)
        .merge(reads)
        .merge(health_routes)
        // PDF processing — stateless, no DB state needed.
        // Nested router with its own 50 MB body limit for PDF uploads.
        .merge(
            Router::new()
                .route(
                    "/api/pdf/render-annotations",
                    post(handlers::annotations::render_annotations),
                )
                .route_layer(RequirePermissionsLayer::new(&[
                    permissions::PDF_EDITOR_MUTATE,
                ]))
                .layer(DefaultBodyLimit::max(52_428_800)),
        )
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
        .layer(cors)
        .into_make_service_with_connect_info::<SocketAddr>();

    let addr: SocketAddr = format!("{}:{}", config.host, config.port)
        .parse()
        .expect("Invalid HOST:PORT");
    tracing::info!("PDF Editor module listening on {}", addr);

    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .expect("Failed to bind");

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
