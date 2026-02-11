mod config;
mod db;
mod middleware;
mod routes;
mod auth;
mod events;

use axum::{middleware::from_fn, routing::get, Router};
use std::sync::Arc;
use tower_http::trace::TraceLayer;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,auth_rs=debug".into()),
        )
        .with(tracing_subscriber::fmt::layer().json())
        .init();

    let cfg = config::Config::from_env()?;
    tracing::info!("config loaded");

    let pool = db::create_pool(&cfg.database_url).await?;
    db::run_migrations(&pool).await?;
    tracing::info!("db connected + migrations applied");

    let nats = async_nats::connect(&cfg.nats_url).await?;
    tracing::info!("nats connected");

    let schema_dir = std::env::var("SCHEMA_DIR").unwrap_or_else(|_| "/app/schemas".into());
    crate::events::validate::load_schemas_from_dir(&schema_dir)?;
    tracing::info!(%schema_dir, "schemas loaded");

    let jwt = crate::auth::jwt::JwtKeys::from_pem(
        &cfg.jwt_private_key_pem.replace("\\n", "\n"),
        &cfg.jwt_public_key_pem.replace("\\n", "\n"),
        cfg.jwt_kid.clone(),
    ).map_err(|e| format!("JWT key error: {e}"))?;

    let pwd = crate::auth::password::PasswordPolicy {
        memory_kb: cfg.argon_memory_kb,
        iterations: cfg.argon_iterations,
        parallelism: cfg.argon_parallelism,
    };

    let producer = format!("auth-rs@{}", env!("CARGO_PKG_VERSION"));
    let events = crate::events::publisher::EventPublisher::new(nats.clone());

    let auth_state = Arc::new(crate::auth::handlers::AuthState {
        db: pool.clone(),
        jwt,
        pwd,
        access_ttl_minutes: cfg.access_token_ttl_minutes,
        refresh_ttl_days: cfg.refresh_token_ttl_days,
        events,
        producer,
    });

    let health_state = Arc::new(routes::health::HealthState { db: pool, nats });

    let app = Router::new()
        .route("/health/live", get(routes::health::health_live))
        .route("/health/ready", get(routes::health::health_ready))
        .with_state(health_state)
        .merge(routes::auth::router(auth_state))
        .layer(from_fn(middleware::tracing::trace_id_middleware))
        .layer(TraceLayer::new_for_http());

    let addr = format!("{}:{}", cfg.host, cfg.port);
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    tracing::info!(%addr, "listening");
    axum::serve(listener, app).await?;

    Ok(())
}
