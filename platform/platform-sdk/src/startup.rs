//! Two-phase startup sequence for platform modules.
//!
//! **Phase A** — infrastructure: dotenv, tracing, config, DB pool, migrations,
//! JWT verifier, rate limiter, health probes.
//!
//! **Phase B** — HTTP: middleware stack, health routes, module routes, TCP bind,
//! graceful shutdown.

use std::net::SocketAddr;
use std::sync::Arc;

use axum::extract::DefaultBodyLimit;
use axum::routing::get;
use axum::{Extension, Json, Router};
use security::middleware::{
    default_rate_limiter, rate_limit_middleware, timeout_middleware, DEFAULT_BODY_LIMIT,
};
use security::{optional_claims_mw, JwtVerifier};
use tower_http::cors::{AllowOrigin, CorsLayer};
use tracing_subscriber::EnvFilter;

use crate::context::ModuleContext;
use crate::manifest::Manifest;

/// Errors that can occur during startup.
#[derive(Debug, thiserror::Error)]
pub enum StartupError {
    #[error("configuration error: {0}")]
    Config(String),

    #[error("database connection failed: {0}")]
    Database(String),

    #[error("database migration failed: {0}")]
    Migration(String),

    #[error("failed to bind {addr}: {source}")]
    Bind {
        addr: SocketAddr,
        source: std::io::Error,
    },

    #[error("server error: {0}")]
    Serve(String),

    #[error("manifest error: {0}")]
    Manifest(#[from] crate::manifest::ManifestError),
}

/// Phase A: infrastructure setup.
///
/// 1. Load .env
/// 2. Initialize tracing
/// 3. Parse DATABASE_URL from env
/// 4. Connect DB pool
/// 5. Run migrations (if auto_migrate)
/// 6. Build JWT verifier (optional)
/// 7. Build rate limiter
pub(crate) async fn phase_a(manifest: &Manifest) -> Result<PhaseAOutput, StartupError> {
    // Step 1: dotenv
    dotenvy::dotenv().ok();

    // Step 2: tracing
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();

    tracing::info!(
        module = %manifest.module.name,
        version = ?manifest.module.version,
        "starting module"
    );

    // Step 3: DATABASE_URL from environment
    let database_url = std::env::var("DATABASE_URL").map_err(|_| {
        StartupError::Config("DATABASE_URL is required but not set".into())
    })?;

    // Step 4: DB pool
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(20)
        .connect(&database_url)
        .await
        .map_err(|e| StartupError::Database(e.to_string()))?;

    tracing::info!(module = %manifest.module.name, "database connected");

    // Step 5: migrations
    if let Some(ref db) = manifest.database {
        if db.auto_migrate {
            // sqlx::migrate! requires a compile-time path, so modules must
            // still embed their own migrations. The SDK just runs the migrator
            // they provide via the builder.
            tracing::info!(
                module = %manifest.module.name,
                migrations = %db.migrations,
                "auto_migrate enabled — module must provide migrator via builder"
            );
        }
    }

    // Step 6: JWT verifier (optional — absent in dev)
    let jwt_verifier = JwtVerifier::from_env_with_overlap().map(Arc::new);
    if jwt_verifier.is_some() {
        tracing::info!(module = %manifest.module.name, "JWT verifier initialized");
    } else {
        tracing::warn!(
            module = %manifest.module.name,
            "no JWT_PUBLIC_KEY — running without token verification"
        );
    }

    // Step 7: rate limiter
    let rate_limiter = default_rate_limiter();

    Ok(PhaseAOutput {
        pool,
        jwt_verifier,
        rate_limiter,
    })
}

pub(crate) struct PhaseAOutput {
    pub pool: sqlx::PgPool,
    pub jwt_verifier: Option<Arc<JwtVerifier>>,
    pub rate_limiter: Arc<security::ratelimit::RateLimiter>,
}

/// Phase B: HTTP stack assembly and server start.
///
/// Steps 9-11: health routes, metrics endpoint, module routes.
/// Steps 12-18: middleware stack, TCP bind, graceful shutdown.
pub(crate) async fn phase_b(
    manifest: &Manifest,
    phase_a: PhaseAOutput,
    module_routes: Router,
    migrator: Option<&sqlx::migrate::Migrator>,
) -> Result<(), StartupError> {
    let module_name = &manifest.module.name;
    let version = manifest
        .module
        .version
        .as_deref()
        .unwrap_or("0.0.0")
        .to_string();

    // Run migrations if a migrator was provided and auto_migrate is enabled.
    if let Some(migrator) = migrator {
        if manifest
            .database
            .as_ref()
            .map_or(false, |db| db.auto_migrate)
        {
            migrator
                .run(&phase_a.pool)
                .await
                .map_err(|e| StartupError::Migration(e.to_string()))?;
            tracing::info!(module = %module_name, "database migrations applied");
        }
    }

    let ctx = ModuleContext::new(phase_a.pool.clone(), manifest.clone());
    let shutdown_pool = phase_a.pool.clone();

    // Health routes
    let health_name = module_name.clone();
    let health_version = version.clone();
    let health_pool = phase_a.pool.clone();

    let health_routes = Router::new()
        .route("/healthz", get(health::healthz))
        .route(
            "/api/health",
            get(move || async move {
                let start = std::time::Instant::now();
                let err = sqlx::query("SELECT 1")
                    .execute(&health_pool)
                    .await
                    .err()
                    .map(|e| e.to_string());
                let latency = start.elapsed().as_millis() as u64;
                let check = health::db_check(latency, err);
                let resp = health::build_ready_response(&health_name, &health_version, vec![check]);
                health::ready_response_to_axum(resp)
            }),
        )
        .route(
            "/api/ready",
            get({
                let ready_name = module_name.clone();
                let ready_version = version.clone();
                let ready_pool = phase_a.pool.clone();
                move || async move {
                    let start = std::time::Instant::now();
                    let err = sqlx::query("SELECT 1")
                        .execute(&ready_pool)
                        .await
                        .err()
                        .map(|e| e.to_string());
                    let latency = start.elapsed().as_millis() as u64;
                    let check = health::db_check(latency, err);
                    let resp =
                        health::build_ready_response(&ready_name, &ready_version, vec![check]);
                    health::ready_response_to_axum(resp)
                }
            }),
        )
        .route(
            "/api/version",
            get({
                let ver = version.clone();
                let name = module_name.clone();
                move || async move {
                    Json(serde_json::json!({
                        "module": name,
                        "version": ver,
                    }))
                }
            }),
        );

    // Metrics endpoint
    let metrics_route = Router::new().route(
        "/metrics",
        get(|| async {
            let encoder = prometheus::TextEncoder::new();
            let families = prometheus::gather();
            match encoder.encode_to_string(&families) {
                Ok(body) => (
                    axum::http::StatusCode::OK,
                    [(axum::http::header::CONTENT_TYPE, "text/plain; version=0.0.4")],
                    body,
                ),
                Err(e) => (
                    axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                    [(axum::http::header::CONTENT_TYPE, "text/plain")],
                    format!("metrics encoding error: {e}"),
                ),
            }
        }),
    );

    // CORS
    let cors = build_cors_layer(manifest);

    // Env-based overrides for host/port
    let host = std::env::var("HOST").unwrap_or_else(|_| manifest.server.host.clone());
    let port: u16 = std::env::var("PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(manifest.server.port);

    // Assemble the full app: module routes + health + metrics + middleware
    let app = module_routes
        .merge(health_routes)
        .merge(metrics_route)
        .layer(Extension(ctx))
        .layer(DefaultBodyLimit::max(DEFAULT_BODY_LIMIT))
        .layer(axum::middleware::from_fn(
            security::tracing::tracing_context_middleware,
        ))
        .layer(axum::middleware::from_fn(timeout_middleware))
        .layer(axum::middleware::from_fn(rate_limit_middleware))
        .layer(Extension(phase_a.rate_limiter))
        .layer(axum::middleware::from_fn_with_state(
            phase_a.jwt_verifier,
            optional_claims_mw,
        ))
        .layer(cors)
        .into_make_service_with_connect_info::<SocketAddr>();

    let addr: SocketAddr = format!("{}:{}", host, port)
        .parse()
        .map_err(|_| StartupError::Config(format!("invalid address: {}:{}", host, port)))?;

    tracing::info!(module = %module_name, %addr, "listening");

    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .map_err(|e| StartupError::Bind { addr, source: e })?;

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .map_err(|e| StartupError::Serve(e.to_string()))?;

    tracing::info!(module = %module_name, "server stopped — closing resources");
    shutdown_pool.close().await;
    tracing::info!(module = %module_name, "shutdown complete");

    Ok(())
}

/// CORS layer copied from Party's working `build_cors_layer()` implementation.
fn build_cors_layer(manifest: &Manifest) -> CorsLayer {
    let cors_env = std::env::var("CORS_ORIGINS").unwrap_or_else(|_| "*".to_string());
    let env_val = std::env::var("ENV").unwrap_or_else(|_| "development".to_string());

    let origins: Vec<String> = cors_env
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();

    let is_wildcard = origins.len() == 1 && origins[0] == "*";

    if is_wildcard && env_val != "development" {
        tracing::warn!(
            module = %manifest.module.name,
            "CORS_ORIGINS is set to wildcard — restrict to specific origins in production"
        );
    }

    let layer = if is_wildcard {
        CorsLayer::new().allow_origin(AllowOrigin::any())
    } else {
        let parsed: Vec<_> = origins.iter().filter_map(|o| o.parse().ok()).collect();
        CorsLayer::new().allow_origin(parsed)
    };

    layer
        .allow_methods(tower_http::cors::Any)
        .allow_headers(tower_http::cors::Any)
        .allow_credentials(false)
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

    tracing::info!("shutdown signal received — draining in-flight requests");
}
