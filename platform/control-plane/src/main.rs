/// Control-Plane HTTP Service
///
/// Exposes the tenant lifecycle control API:
///   POST /api/control/tenants                       — Provision a new tenant
///   GET  /api/control/tenants/:tenant_id/summary    — Tenant readiness summary
///   POST /api/control/platform-billing-runs         — Platform billing cycle runner
///
/// Connects to the tenant-registry database (required) and the AR database (optional).
/// Runs SQLx migrations on startup.
/// When NATS_URL is set, spawns an outbox relay that publishes provisioning events.
use axum::extract::DefaultBodyLimit;
use control_plane::outbox_relay;
use control_plane::provisioning;
use control_plane::routes;
use control_plane::state;
use event_bus::{EventBus, NatsBus};
use security::JwtVerifier;

use sqlx::postgres::PgPoolOptions;

/// Default maximum request body size: 2 MiB (matches platform security constant).
const DEFAULT_BODY_LIMIT: usize = 2 * 1024 * 1024;
use std::sync::Arc;
use tenant_registry::routes::SummaryState;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive(tracing::Level::INFO.into()),
        )
        .init();

    let db_url = std::env::var("DATABASE_URL").unwrap_or_else(|_| {
        "postgresql://tenant_registry_user:tenant_registry_pass@localhost:5441/tenant_registry_db"
            .to_string()
    });

    let pool = PgPoolOptions::new()
        .max_connections(10)
        .connect(&db_url)
        .await?;

    let shutdown_pool = pool.clone();

    // Run migrations from tenant-registry migrations directory
    sqlx::migrate::Migrator::new(std::path::Path::new(
        "./platform/tenant-registry/db/migrations",
    ))
    .await?
    .run(&pool)
    .await?;

    tracing::info!("Migrations applied");

    // AR pool is optional — billing runs will fail gracefully if unavailable.
    let ar_db_url = std::env::var("AR_DATABASE_URL")
        .unwrap_or_else(|_| "postgresql://ar_user:ar_pass@localhost:5434/ar_db".to_string());
    let ar_pool = match PgPoolOptions::new()
        .max_connections(5)
        .connect(&ar_db_url)
        .await
    {
        Ok(p) => {
            tracing::info!("AR database pool connected");
            Some(p)
        }
        Err(e) => {
            tracing::warn!(
                "AR database unavailable — platform billing runs will be disabled: {}",
                e
            );
            None
        }
    };

    // NATS outbox relay + provisioning consumer (optional — skipped if NATS_URL is not set)
    if let Ok(nats_url) = std::env::var("NATS_URL") {
        match event_bus::connect_nats(&nats_url).await {
            Ok(nats_client) => {
                let bus: Arc<dyn EventBus> = Arc::new(NatsBus::new(nats_client));
                tracing::info!(url = %nats_url, "connected to NATS");

                // Outbox relay
                tokio::spawn(outbox_relay::start_outbox_relay(pool.clone(), bus.clone()));

                // Provisioning orchestrator
                match provisioning::load_registry_from_db(&pool).await {
                    Ok(registry) => {
                        tokio::spawn(provisioning::start_provisioning_consumer(
                            pool.clone(),
                            bus,
                            registry,
                        ));
                    }
                    Err(e) => {
                        tracing::warn!(
                            error = %e,
                            "failed to load module registry — provisioning consumer disabled"
                        );
                    }
                }
            }
            Err(e) => {
                tracing::warn!(
                    url = %nats_url,
                    error = %e,
                    "NATS connection failed — provisioning outbox relay disabled"
                );
            }
        }
    } else {
        tracing::info!("NATS_URL not set — provisioning outbox relay disabled");
    }

    let jwt_verifier = JwtVerifier::from_env_with_overlap().map(Arc::new);
    if jwt_verifier.is_some() {
        tracing::info!("JWT verifier loaded — RBAC enforcement active");
    } else {
        tracing::warn!("JWT_PUBLIC_KEY not set — all RBAC-protected routes will return 401");
    }

    let mut raw_state = state::AppState::new(pool.clone(), ar_pool);
    if let Some(v) = jwt_verifier {
        raw_state = raw_state.with_verifier(v);
    }
    let app_state = Arc::new(raw_state);
    let summary_state = Arc::new(SummaryState::new_local(pool));

    let app = routes::build_router(app_state, summary_state)
        .layer(DefaultBodyLimit::max(DEFAULT_BODY_LIMIT));

    let port = std::env::var("PORT").unwrap_or_else(|_| "8091".to_string());
    let addr = format!("0.0.0.0:{port}");

    tracing::info!("Control-plane listening on {}", addr);
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    axum::serve(listener, app)
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
