//! Two-phase startup sequence for platform modules.
//!
//! **Phase A** — infrastructure: dotenv, tracing, config, DB pool, migrations,
//! event bus, outbox publisher, JWT verifier, rate limiter.
//!
//! **Phase B** — HTTP: middleware stack, health routes, module routes, TCP bind,
//! graceful shutdown.

use std::net::SocketAddr;
use std::sync::Arc;

use axum::extract::DefaultBodyLimit;
use axum::routing::get;
use axum::{Extension, Json, Router};
use event_bus::EventBus;
use security::middleware::{
    default_rate_limiter, rate_limit_middleware, timeout_middleware, DEFAULT_BODY_LIMIT,
};
use security::{optional_claims_mw, JwtVerifier};
use tower_http::cors::{AllowOrigin, CorsLayer};
use tracing_subscriber::EnvFilter;

use crate::context::ModuleContext;
use crate::manifest::Manifest;
use crate::publisher;

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

    #[error("database has outbox table '{table}' but manifest does not declare [events.publish].outbox_table — add it or remove the table")]
    UndeclaredOutboxTable { table: String },
}

/// Phase A: infrastructure setup.
///
/// 1. Load .env
/// 2. Initialize tracing
/// 3. Parse DATABASE_URL from env
/// 4. Connect DB pool
/// 5. Log migration intent (actual run is in Phase B)
/// 6. Create EventBus (if bus_type != "none")
/// 7. Detect undeclared outbox tables / spawn outbox publisher
/// 8. Build JWT verifier (optional)
/// 9. Build rate limiter
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
            tracing::info!(
                module = %manifest.module.name,
                migrations = %db.migrations,
                "auto_migrate enabled — module must provide migrator via builder"
            );
        }
    }

    // Step 6: Create EventBus (if bus_type != "none")
    let bus_type = manifest
        .bus
        .as_ref()
        .map(|b| b.bus_type.to_lowercase())
        .unwrap_or_default();

    let bus: Option<Arc<dyn EventBus>> = match bus_type.as_str() {
        "nats" => {
            let nats_url = std::env::var("NATS_URL").map_err(|_| {
                StartupError::Config("NATS_URL is required when bus.type=nats".into())
            })?;
            tracing::info!(module = %manifest.module.name, url = %nats_url, "connecting to NATS");
            let client = event_bus::connect_nats(&nats_url)
                .await
                .map_err(|e| StartupError::Config(format!("NATS connection failed: {e}")))?;
            tracing::info!(module = %manifest.module.name, "NATS event bus connected");
            Some(Arc::new(event_bus::NatsBus::new(client)))
        }
        "inmemory" => {
            tracing::info!(module = %manifest.module.name, "in-memory event bus created");
            Some(Arc::new(event_bus::InMemoryBus::new()))
        }
        _ => None, // "none", missing section
    };

    // Step 7: outbox publisher / undeclared outbox detection
    let outbox_table = manifest
        .events
        .as_ref()
        .and_then(|e| e.publish.as_ref())
        .map(|p| p.outbox_table.clone());

    if let Some(ref table) = outbox_table {
        if let Some(ref bus) = bus {
            let pub_pool = pool.clone();
            let pub_bus = bus.clone();
            let pub_table = table.clone();
            let pub_module = manifest.module.name.clone();
            tokio::spawn(async move {
                publisher::run_outbox_publisher(pub_pool, pub_bus, &pub_table, &pub_module).await;
            });
            tracing::info!(
                module = %manifest.module.name,
                outbox_table = %table,
                "outbox publisher task spawned"
            );
        }
    } else if bus.is_some() {
        // No outbox_table declared — check if DB secretly has one.
        if let Some(found) = publisher::detect_outbox_table(&pool).await? {
            return Err(StartupError::UndeclaredOutboxTable { table: found });
        }
    }

    // Step 8: JWT verifier (optional — absent in dev)
    let jwt_verifier = JwtVerifier::from_env_with_overlap().map(Arc::new);
    if jwt_verifier.is_some() {
        tracing::info!(module = %manifest.module.name, "JWT verifier initialized");
    } else {
        tracing::warn!(
            module = %manifest.module.name,
            "no JWT_PUBLIC_KEY — running without token verification"
        );
    }

    // Step 9: rate limiter
    let rate_limiter = default_rate_limiter();

    Ok(PhaseAOutput {
        pool,
        bus,
        jwt_verifier,
        rate_limiter,
    })
}

pub(crate) struct PhaseAOutput {
    pub pool: sqlx::PgPool,
    pub bus: Option<Arc<dyn EventBus>>,
    pub jwt_verifier: Option<Arc<JwtVerifier>>,
    pub rate_limiter: Arc<security::ratelimit::RateLimiter>,
}

/// Phase B: HTTP stack assembly and server start.
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

    let ctx = ModuleContext::new(phase_a.pool.clone(), manifest.clone(), phase_a.bus.clone());
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
