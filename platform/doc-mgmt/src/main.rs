use axum::{routing::get, Router};
use event_bus::{EventBus, NatsBus};
use security::{optional_claims_mw, JwtVerifier};
use std::net::SocketAddr;
use std::sync::Arc;
use tracing_subscriber::EnvFilter;

mod attachments;
mod config;
mod db;
mod distribution;
mod handlers;
mod models;
mod outbox_relay;
mod render;
mod retention;
mod routes;
mod template_engine;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    dotenvy::dotenv().ok();

    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();

    let cfg = config::Config::from_env()?;

    // DB + migrations (fail-fast)
    let pool = db::create_pool(&cfg.database_url).await?;
    let shutdown_pool = pool.clone();
    db::run_migrations(&pool).await?;
    tracing::info!("database migrations applied");

    // NATS (fail-fast)
    let nats_client = event_bus::connect_nats(&cfg.nats_url).await?;
    let bus: Arc<dyn EventBus> = Arc::new(NatsBus::new(nats_client));
    tracing::info!(url = %cfg.nats_url, "connected to NATS");

    // Spawn outbox relay
    tokio::spawn(outbox_relay::start_outbox_relay(pool.clone(), bus));

    // Blob storage (fail-fast — required for attachment endpoints)
    let blob_config = blob_storage::BlobStorageConfig::from_env()
        .map_err(|e| format!("blob storage config error: {e}"))?;
    let blob_client = blob_storage::BlobStorageClient::new(blob_config)
        .await
        .map_err(|e| format!("blob storage init error: {e}"))?;
    blob_client
        .ensure_bucket_exists()
        .await
        .map_err(|e| format!("blob storage bucket check failed: {e}"))?;
    tracing::info!("blob storage initialized");

    // App state
    let app_state = Arc::new(handlers::AppState {
        db: pool.clone(),
        blob: Arc::new(blob_client),
    });

    // JWT verifier (optional — permissive if not configured)
    let maybe_verifier = JwtVerifier::from_env_with_overlap().map(Arc::new);

    // Health endpoints (no auth)
    let health_pool = pool.clone();
    let health_router = Router::new().route("/healthz", get(health::healthz)).route(
        "/api/ready",
        get(move || {
            let db = health_pool.clone();
            async move {
                let start = std::time::Instant::now();
                let check = sqlx::query("SELECT 1").execute(&db).await;
                let latency = start.elapsed().as_millis() as u64;
                let db_check = health::db_check(latency, check.err().map(|e| e.to_string()));
                let resp = health::build_ready_response(
                    "doc-mgmt",
                    env!("CARGO_PKG_VERSION"),
                    vec![db_check],
                );
                health::ready_response_to_axum(resp)
            }
        }),
    );

    // API router (with authz at route level in routes::api_router)
    let api = routes::api_router(app_state);

    let app =
        Router::new()
            .merge(health_router)
            .merge(api)
            .layer(axum::middleware::from_fn_with_state(
                maybe_verifier,
                optional_claims_mw,
            ));

    let addr: SocketAddr = format!("{}:{}", cfg.host, cfg.port).parse()?;
    tracing::info!(%addr, "doc-mgmt listening");

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown_signal())
    .await?;

    tracing::info!("Server stopped — closing resources");
    shutdown_pool.close().await;
    tracing::info!("Shutdown complete");

    Ok(())
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
