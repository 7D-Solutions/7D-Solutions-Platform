/// Control-Plane HTTP Service
///
/// Exposes the tenant lifecycle control API:
///   POST /api/control/tenants                       — Provision a new tenant
///   GET  /api/control/tenants/:tenant_id/summary    — Tenant readiness summary
///
/// Connects to the tenant-registry database.
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

    // Run migrations from tenant-registry migrations directory
    sqlx::migrate::Migrator::new(std::path::Path::new(
        "./platform/tenant-registry/db/migrations",
    ))
    .await?
    .run(&pool)
    .await?;

    tracing::info!("Migrations applied");

    let app_state = Arc::new(state::AppState::new(pool.clone()));
    let summary_state = Arc::new(SummaryState::new_local(pool));

    let app = routes::build_router(app_state, summary_state);

    let port = std::env::var("PORT").unwrap_or_else(|_| "8092".to_string());
    let addr = format!("0.0.0.0:{port}");

    tracing::info!("Control-plane listening on {}", addr);
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}
