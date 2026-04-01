//! Two-phase startup: Phase A (infrastructure) then Phase B (HTTP stack).

use std::net::SocketAddr;
use std::sync::Arc;

use axum::extract::DefaultBodyLimit;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::get;
use axum::{Extension, Json, Router};
use event_bus::EventBus;
use security::middleware::rate_limit_middleware;
use security::ratelimit::{RateLimitConfig, RateLimiter};
use security::{optional_claims_mw, JwtVerifier};
use tracing_subscriber::EnvFilter;

use crate::startup_helpers::{build_cors_layer, parse_body_limit, parse_duration_str, shutdown_signal};

use crate::consumer::ConsumerHandles;
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

    // Step 4: DB pool — sizes from manifest [database] section (defaults: min=5, max=20)
    let pool_max = manifest.database.as_ref().map_or(20, |db| db.pool_max);
    let pool_min = manifest.database.as_ref().map_or(5, |db| db.pool_min);
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(pool_max)
        .min_connections(pool_min)
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

    let mut outbox_handle: Option<tokio::task::JoinHandle<()>> = None;

    if let Some(ref table) = outbox_table {
        if let Some(ref bus) = bus {
            let pub_pool = pool.clone();
            let pub_bus = bus.clone();
            let pub_table = table.clone();
            let pub_module = manifest.module.name.clone();
            outbox_handle = Some(tokio::spawn(async move {
                publisher::run_outbox_publisher(pub_pool, pub_bus, &pub_table, &pub_module).await;
            }));
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

    // Step 8: JWT verifier — JWKS URL (manifest) or env var fallback
    let jwt_verifier = if let Some(ref auth) = manifest.auth {
        if !auth.enabled {
            None
        } else if let Some(ref jwks_url) = auth.jwks_url {
            let interval = parse_duration_str(&auth.refresh_interval);
            match JwtVerifier::from_jwks_url(jwks_url, interval, auth.fallback_to_env).await {
                Ok(v) => {
                    tracing::info!(
                        module = %manifest.module.name,
                        jwks_url = %jwks_url,
                        "JWT verifier initialized from JWKS"
                    );
                    Some(Arc::new(v))
                }
                Err(e) => {
                    return Err(StartupError::Config(format!("JWKS auth failed: {e}")));
                }
            }
        } else {
            JwtVerifier::from_env_with_overlap().map(Arc::new)
        }
    } else {
        JwtVerifier::from_env_with_overlap().map(Arc::new)
    };
    if jwt_verifier.is_none() {
        tracing::warn!(
            module = %manifest.module.name,
            "running without JWT verification"
        );
    }

    // Step 9: rate limiter — from manifest [rate_limit] section or platform defaults
    let rate_limiter = if let Some(ref rl) = manifest.rate_limit {
        let window = std::time::Duration::from_secs(1);
        Arc::new(RateLimiter::with_configs(
            RateLimitConfig::new(rl.burst, window),
            RateLimitConfig::new((rl.requests_per_second / 10).max(1), window),
        ))
    } else {
        security::middleware::default_rate_limiter()
    };

    Ok(PhaseAOutput {
        pool,
        bus,
        jwt_verifier,
        rate_limiter,
        outbox_handle,
    })
}

pub(crate) struct PhaseAOutput {
    pub pool: sqlx::PgPool,
    pub bus: Option<Arc<dyn EventBus>>,
    pub jwt_verifier: Option<Arc<JwtVerifier>>,
    pub rate_limiter: Arc<security::ratelimit::RateLimiter>,
    pub outbox_handle: Option<tokio::task::JoinHandle<()>>,
}

/// Phase B: HTTP stack assembly and server start.
pub(crate) async fn phase_b(
    manifest: &Manifest,
    phase_a: PhaseAOutput,
    module_routes: Router,
    migrator: Option<&sqlx::migrate::Migrator>,
    consumer_handles: ConsumerHandles,
    ctx: ModuleContext,
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

    let shutdown_pool = phase_a.pool.clone();

    // Determine which dependencies to probe from manifest [health] section.
    let health_deps: Vec<String> = manifest
        .health
        .as_ref()
        .map(|h| h.dependencies.clone())
        .unwrap_or_default();
    let probe_nats = health_deps.iter().any(|d| d == "nats");

    // Warn if manifest declares nats dependency but no bus is configured.
    if probe_nats && phase_a.bus.is_none() {
        tracing::warn!(
            module = %module_name,
            "health.dependencies includes 'nats' but no event bus is configured — \
             NATS will report as down in health checks"
        );
    }

    // Health routes
    let health_name = module_name.clone();
    let health_version = version.clone();
    let health_pool = phase_a.pool.clone();
    let health_bus = if probe_nats { phase_a.bus.clone() } else { None };

    let health_routes = Router::new()
        .route("/healthz", get(health::healthz))
        .route(
            "/api/health",
            get(move || async move {
                let mut checks = Vec::new();

                // Postgres probe
                let start = std::time::Instant::now();
                let err = sqlx::query("SELECT 1")
                    .execute(&health_pool)
                    .await
                    .err()
                    .map(|e| e.to_string());
                let latency = start.elapsed().as_millis() as u64;
                checks.push(health::db_check(latency, err));

                // NATS probe (only if declared in health.dependencies)
                if let Some(ref bus) = health_bus {
                    let nats_start = std::time::Instant::now();
                    let connected = bus.health_check().await;
                    let nats_latency = nats_start.elapsed().as_millis() as u64;
                    checks.push(health::nats_check(connected, nats_latency));
                }

                let resp = health::build_ready_response(&health_name, &health_version, checks);
                health::ready_response_to_axum(resp)
            }),
        )
        .route(
            "/api/ready",
            get({
                let ready_name = module_name.clone();
                let ready_version = version.clone();
                let ready_pool = phase_a.pool.clone();
                let ready_bus = if probe_nats { phase_a.bus.clone() } else { None };
                move || async move {
                    let mut checks = Vec::new();

                    // Postgres probe
                    let start = std::time::Instant::now();
                    let err = sqlx::query("SELECT 1")
                        .execute(&ready_pool)
                        .await
                        .err()
                        .map(|e| e.to_string());
                    let latency = start.elapsed().as_millis() as u64;
                    checks.push(health::db_check(latency, err));

                    // NATS probe (only if declared in health.dependencies)
                    if let Some(ref bus) = ready_bus {
                        let nats_start = std::time::Instant::now();
                        let connected = bus.health_check().await;
                        let nats_latency = nats_start.elapsed().as_millis() as u64;
                        checks.push(health::nats_check(connected, nats_latency));
                    }

                    let resp =
                        health::build_ready_response(&ready_name, &ready_version, checks);
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

    // Body limit from manifest [server] section (default: "2mb")
    let body_limit = parse_body_limit(&manifest.server.body_limit);

    // Request timeout from manifest [server] section (default: "30s")
    let request_timeout = parse_duration_str(&manifest.server.request_timeout);

    // Assemble the full app: module routes + health + metrics + middleware
    let app = module_routes
        .merge(health_routes)
        .merge(metrics_route)
        .layer(Extension(ctx))
        .layer(DefaultBodyLimit::max(body_limit))
        .layer(axum::middleware::from_fn(
            security::tracing::tracing_context_middleware,
        ))
        .layer(axum::middleware::from_fn(move |req, next: axum::middleware::Next| async move {
            match tokio::time::timeout(request_timeout, next.run(req)).await {
                Ok(response) => response,
                Err(_) => (StatusCode::REQUEST_TIMEOUT, "Request timeout\n").into_response(),
            }
        }))
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

    tracing::info!(module = %module_name, "server stopped — draining consumers");
    consumer_handles.shutdown().await;
    if let Some(handle) = phase_a.outbox_handle {
        tracing::info!(module = %module_name, "stopping outbox publisher");
        handle.abort();
        let _ = handle.await;
    }
    tracing::info!(module = %module_name, "closing resources");
    shutdown_pool.close().await;
    tracing::info!(module = %module_name, "shutdown complete");

    Ok(())
}

/// Phase B variant: serve module routes as-is without SDK middleware.
///
/// Used when a module provides its own middleware stack (CORS, JWT, health,
/// metrics). The SDK still handles migrations, graceful shutdown, and
/// consumer draining.
pub(crate) async fn phase_b_raw(
    manifest: &Manifest,
    phase_a: PhaseAOutput,
    module_routes: Router,
    migrator: Option<&sqlx::migrate::Migrator>,
    consumer_handles: ConsumerHandles,
) -> Result<(), StartupError> {
    let module_name = &manifest.module.name;

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

    let shutdown_pool = phase_a.pool.clone();

    // Env-based overrides for host/port
    let host = std::env::var("HOST").unwrap_or_else(|_| manifest.server.host.clone());
    let port: u16 = std::env::var("PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(manifest.server.port);

    let app = module_routes.into_make_service_with_connect_info::<SocketAddr>();

    let addr: SocketAddr = format!("{}:{}", host, port)
        .parse()
        .map_err(|_| StartupError::Config(format!("invalid address: {}:{}", host, port)))?;

    tracing::info!(module = %module_name, %addr, "listening (raw mode)");

    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .map_err(|e| StartupError::Bind { addr, source: e })?;

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .map_err(|e| StartupError::Serve(e.to_string()))?;

    tracing::info!(module = %module_name, "server stopped — draining consumers");
    consumer_handles.shutdown().await;
    if let Some(handle) = phase_a.outbox_handle {
        tracing::info!(module = %module_name, "stopping outbox publisher");
        handle.abort();
        let _ = handle.await;
    }
    tracing::info!(module = %module_name, "closing resources");
    shutdown_pool.close().await;
    tracing::info!(module = %module_name, "shutdown complete");

    Ok(())
}
