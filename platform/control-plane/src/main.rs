/// Control-Plane HTTP Service
///
/// Exposes the tenant lifecycle control API:
///   POST /api/control/tenants                       — Provision a new tenant
///   GET  /api/control/tenants/:tenant_id/summary    — Tenant readiness summary
///   POST /api/control/platform-billing-runs         — Platform billing cycle runner
///
/// Connects to the tenant-registry database (required) and the AR database (optional).
/// Runs SQLx migrations on startup.
use control_plane::routes;
use control_plane::state;

use sqlx::postgres::PgPoolOptions;
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
    let ar_db_url = std::env::var("AR_DATABASE_URL").unwrap_or_else(|_| {
        "postgresql://ar_user:ar_pass@localhost:5434/ar_db".to_string()
    });
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
            tracing::warn!("AR database unavailable — platform billing runs will be disabled: {}", e);
            None
        }
    };

    let app_state = Arc::new(state::AppState::new(pool.clone(), ar_pool));
    let summary_state = Arc::new(SummaryState::new_local(pool));

    let app = routes::build_router(app_state, summary_state);

    let port = std::env::var("PORT").unwrap_or_else(|_| "8092".to_string());
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
