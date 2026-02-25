use axum::{extract::DefaultBodyLimit, routing::{get, post, put}, Extension, Router};
use ::event_bus::{EventBus, InMemoryBus, NatsBus};
use http::HeaderName;
use pdf_editor_rs::{config, config::Config, db, event_bus, metrics, routes};
use security::{
    middleware::{default_rate_limiter, rate_limit_middleware, timeout_middleware, DEFAULT_BODY_LIMIT},
    optional_claims_mw, permissions, JwtVerifier, RequirePermissionsLayer,
};
use std::net::SocketAddr;
use std::sync::Arc;
use tower_http::cors::{AllowOrigin, CorsLayer};
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
            let nats_client = async_nats::connect(nats_url)
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

    // Mutation routes — require pdf_editor.mutate permission.
    let mutations = Router::new()
        // Form templates — write
        .route("/api/pdf/forms/templates", post(routes::templates::create_template))
        .route("/api/pdf/forms/templates/{id}", put(routes::templates::update_template))
        // Form fields — write
        .route("/api/pdf/forms/templates/{id}/fields", post(routes::fields::create_field))
        .route("/api/pdf/forms/templates/{tid}/fields/{fid}", put(routes::fields::update_field))
        .route("/api/pdf/forms/templates/{id}/fields/reorder", post(routes::fields::reorder_fields))
        // Form submissions — write
        .route("/api/pdf/forms/submissions", post(routes::submissions::create_submission))
        .route("/api/pdf/forms/submissions/{id}", put(routes::submissions::autosave_submission))
        .route("/api/pdf/forms/submissions/{id}/submit", post(routes::submissions::submit_submission))
        .route("/api/pdf/forms/submissions/{id}/generate", post(routes::generate::generate_pdf))
        .route_layer(RequirePermissionsLayer::new(&[permissions::PDF_EDITOR_MUTATE]))
        .with_state(db.clone());

    // Read routes — no permission gate.
    let reads = Router::new()
        .route("/healthz", get(health::healthz))
        .route("/api/health", get(routes::health::health))
        .route("/api/ready", get(routes::health::ready))
        .route("/api/version", get(routes::health::version))
        .route("/metrics", get(metrics::metrics_handler))
        // Form templates — read
        .route("/api/pdf/forms/templates", get(routes::templates::list_templates))
        .route("/api/pdf/forms/templates/{id}", get(routes::templates::get_template))
        // Form fields — read
        .route("/api/pdf/forms/templates/{id}/fields", get(routes::fields::list_fields))
        // Form submissions — read
        .route("/api/pdf/forms/submissions", get(routes::submissions::list_submissions))
        .route("/api/pdf/forms/submissions/{id}", get(routes::submissions::get_submission))
        .with_state(db.clone());

    let app = Router::new()
        .merge(mutations)
        .merge(reads)
        // PDF processing — stateless, no DB state needed.
        // Nested router with its own 50 MB body limit for PDF uploads.
        .merge(
            Router::new()
                .route("/api/pdf/render-annotations", post(routes::annotations::render_annotations))
                .route_layer(RequirePermissionsLayer::new(&[permissions::PDF_EDITOR_MUTATE]))
                .layer(DefaultBodyLimit::max(52_428_800))
        )
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
    tracing::info!("PDF Editor module listening on {}", addr);

    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .expect("Failed to bind");

    axum::serve(listener, app)
        .await
        .expect("Server failed to start");
}

/// Build CORS layer from configuration.
///
/// - `CORS_ORIGINS=*` → allow any origin
/// - `CORS_ORIGINS=http://localhost:3000,https://app.example.com` → specific origins
///
/// Always allows `If-Match` header for optimistic concurrency on document updates.
fn build_cors_layer(config: &Config) -> CorsLayer {
    let is_wildcard = config.cors_origins.len() == 1 && config.cors_origins[0] == "*";

    if is_wildcard && config.env != "development" {
        tracing::warn!("CORS_ORIGINS is set to wildcard — restrict to specific origins in production");
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
        .allow_headers([
            http::header::CONTENT_TYPE,
            http::header::AUTHORIZATION,
            HeaderName::from_static("if-match"),
            HeaderName::from_static("if-none-match"),
        ])
        .expose_headers([http::header::ETAG])
}
