//! Two-phase startup: Phase A (infrastructure) then Phase B (HTTP stack).

use std::net::SocketAddr;
use std::sync::Arc;

use axum::extract::{DefaultBodyLimit, Query, Request};
use axum::http::StatusCode;
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Extension, Json, Router};
use event_bus::EventBus;
use security::claims::VerifiedClaims;
use security::middleware::{rate_limit_middleware, tiered_rate_limit_middleware};
use security::ratelimit::{RateLimitConfig, RateLimiter, TieredRateLimiter};
use security::{optional_claims_mw, JwtVerifier};
use tracing::Instrument;
use tracing_subscriber::EnvFilter;
use uuid::Uuid;

use crate::startup_helpers::{
    build_cors_layer, parse_body_limit, parse_duration_str, shutdown_signal,
};

use crate::consumer::ConsumerHandles;
use crate::context::ModuleContext;

// ── Distributed tracing task-local ───────────────────────────────────────────

tokio::task_local! {
    /// The trace ID for the current request, set by [`platform_trace_middleware`].
    /// Used by [`crate::http_client::PlatformClient`] to propagate trace context
    /// on every outbound HTTP call without requiring callers to thread the ID manually.
    pub(crate) static CURRENT_TRACE_ID: String;
}

/// Parse a W3C Trace Context `traceparent` header and extract the trace ID as a UUID string.
///
/// Format: `{version}-{trace_id_32hex}-{parent_id_16hex}-{flags}`
/// Example: `00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01`
fn parse_traceparent(header: &str) -> Option<String> {
    let parts: Vec<&str> = header.splitn(4, '-').collect();
    if parts.len() < 4 {
        return None;
    }
    // parts[0] = version ("00"), parts[1] = trace_id (32 hex chars)
    let trace_hex = parts[1];
    if trace_hex.len() != 32 || !trace_hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    // Reformat as UUID: 8-4-4-4-12
    Some(format!(
        "{}-{}-{}-{}-{}",
        &trace_hex[0..8],
        &trace_hex[8..12],
        &trace_hex[12..16],
        &trace_hex[16..20],
        &trace_hex[20..32]
    ))
}

fn header_str(headers: &axum::http::HeaderMap, name: &str) -> Option<String> {
    headers
        .get(name)
        .and_then(|v| v.to_str().ok())
        .filter(|s| !s.trim().is_empty())
        .map(|s| s.to_string())
}

/// Platform-aware request tracing middleware.
///
/// Extends the security tracing context with `tenant_id` and `actor_id` from JWT claims,
/// making these fields available in every structured log line emitted during request
/// processing. Runs AFTER JWT auth so claims are already in request extensions.
///
/// Additionally:
/// - Reads a W3C `traceparent` header (falling back to `X-Trace-Id`) so that upstream
///   services can propagate their trace context.
/// - Reads `X-Request-Id` (or generates a fresh UUID) and records it as `request_id`
///   in the span, satisfying the platform logging standard.
/// - Stores the trace ID in [`CURRENT_TRACE_ID`] task-local so outbound
///   [`crate::http_client::PlatformClient`] calls can propagate it automatically.
/// - Echoes `X-Request-Id`, `X-Trace-Id`, and `X-Correlation-Id` in the response.
pub async fn platform_trace_middleware(request: Request, next: Next) -> Response {
    // Resolve trace_id: W3C traceparent takes precedence, then X-Trace-Id, then fresh UUID.
    let trace_id = header_str(request.headers(), "traceparent")
        .and_then(|tp| parse_traceparent(&tp))
        .or_else(|| header_str(request.headers(), "x-trace-id"))
        .unwrap_or_else(|| Uuid::new_v4().to_string());

    let correlation_id =
        header_str(request.headers(), "x-correlation-id").unwrap_or_else(|| trace_id.clone());

    // Claims are already injected by the JWT auth middleware (which runs before tracing).
    let claims = request.extensions().get::<VerifiedClaims>().cloned();
    let tenant_id = claims
        .as_ref()
        .map(|c| c.tenant_id.to_string())
        .unwrap_or_default();
    let actor_id = claims
        .as_ref()
        .map(|c| c.user_id.to_string())
        .unwrap_or_default();

    // request_id: use the X-Request-Id header if present, else alias to trace_id.
    let request_id =
        header_str(request.headers(), "x-request-id").unwrap_or_else(|| trace_id.clone());

    let method = request.method().clone();
    let uri = request.uri().path().to_string();

    let span = tracing::info_span!(
        "request",
        trace_id = %trace_id,
        request_id = %request_id,
        correlation_id = %correlation_id,
        tenant_id = %tenant_id,
        actor_id = %actor_id,
        method = %method,
        path = %uri,
    );

    // Also inject TracingContext extension so consumers/handlers that read it still work.
    let tracing_ctx = event_bus::TracingContext::new()
        .with_trace_id(trace_id.clone())
        .with_correlation_id(correlation_id.clone());
    let tracing_ctx = if let Some(ref c) = claims {
        tracing_ctx.with_actor(c.user_id, "User".to_string())
    } else {
        tracing_ctx
    };

    let mut request = request;
    request.extensions_mut().insert(tracing_ctx);

    let trace_id_local = trace_id.clone();
    let mut response = CURRENT_TRACE_ID
        .scope(trace_id_local, next.run(request).instrument(span))
        .await;

    // Echo tracing IDs in response headers so callers can correlate.
    // request_id is the client-visible ID (echoes the X-Request-Id the caller sent, or the
    // generated trace_id when none was provided).  trace_id tracks the distributed trace.
    if let Ok(val) = request_id.parse() {
        response.headers_mut().insert("x-request-id", val);
    }
    if let Ok(val) = trace_id.parse() {
        response.headers_mut().insert("x-trace-id", val);
    }
    if let Ok(val) = correlation_id.parse() {
        response.headers_mut().insert("x-correlation-id", val);
    }

    response
}
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

/// Initialise a [`BlobStorageClient`] from the manifest `[blob]` section and env vars.
///
/// The bucket name is read from `[blob].bucket` in the manifest.  All credentials
/// and the optional endpoint come from environment variables (loaded in phase A).
///
/// Returns a config error and aborts startup if the `[blob]` section is absent or
/// any required env var is missing.
///
/// [`BlobStorageClient`]: blob_storage::BlobStorageClient
pub(crate) async fn init_blob_storage(
    manifest: &Manifest,
) -> Result<blob_storage::BlobStorageClient, StartupError> {
    let blob_section = manifest.blob.as_ref().ok_or_else(|| {
        StartupError::Config(
            ".blob_storage() called but [blob] section is missing from module.toml \
             — add [blob] bucket = \"<name>\" to enable blob storage"
                .into(),
        )
    })?;

    let require_env = |name: &str| -> Result<String, StartupError> {
        std::env::var(name).map_err(|_| {
            StartupError::Config(format!(
                "{name} environment variable is required when [blob] is configured"
            ))
        })
    };

    let config = blob_storage::BlobStorageConfig {
        provider: std::env::var("BLOB_PROVIDER").unwrap_or_else(|_| "s3".to_string()),
        region: require_env("BLOB_REGION")?,
        endpoint: std::env::var("BLOB_ENDPOINT").ok(),
        bucket: blob_section.bucket.clone(),
        access_key_id: require_env("BLOB_ACCESS_KEY_ID")?,
        secret_access_key: require_env("BLOB_SECRET_ACCESS_KEY")?,
        presign_ttl_seconds: std::env::var("BLOB_PRESIGN_TTL_SECONDS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(900),
        max_upload_bytes: std::env::var("BLOB_MAX_UPLOAD_BYTES")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(26_214_400),
    };

    blob_storage::BlobStorageClient::new(config)
        .await
        .map_err(|e| StartupError::Config(format!("blob storage client init failed: {e}")))
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
/// 10. Enforce auth-required (fail-closed unless explicitly opted out)
pub(crate) async fn phase_a(
    manifest: &Manifest,
    skip_outbox: bool,
    skip_auth: bool,
    pool_resolver: Option<Arc<dyn crate::context::TenantPoolResolver>>,
    builder_tiers: Vec<security::ratelimit::TierDef>,
) -> Result<PhaseAOutput, StartupError> {
    // Step 1: dotenv
    dotenvy::dotenv().ok();

    // Step 2: tracing — LOG_FORMAT=json enables structured JSON output
    //
    // OTLP export: when OTEL_EXPORTER_OTLP_ENDPOINT is set, spans are intended for
    // export to an OpenTelemetry-compatible backend (Jaeger, Tempo, etc.). The SDK
    // logs a startup notice here; full gRPC OTLP export is wired via the
    // opentelemetry-otlp crate when that integration is enabled.
    let log_format = std::env::var("LOG_FORMAT").unwrap_or_default();
    let env_filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into());
    if log_format.eq_ignore_ascii_case("json") {
        tracing_subscriber::fmt()
            .json()
            .with_env_filter(env_filter)
            .init();
    } else {
        tracing_subscriber::fmt().with_env_filter(env_filter).init();
    }

    if let Ok(otlp_endpoint) = std::env::var("OTEL_EXPORTER_OTLP_ENDPOINT") {
        tracing::info!(
            module = %manifest.module.name,
            otlp_endpoint = %otlp_endpoint,
            "OTLP endpoint configured — spans are logged as structured JSON; \
             wire opentelemetry-otlp for gRPC export"
        );
    }

    tracing::info!(
        module = %manifest.module.name,
        version = ?manifest.module.version,
        "starting module"
    );

    // Step 3: DATABASE_URL from environment
    let database_url = std::env::var("DATABASE_URL")
        .map_err(|_| StartupError::Config("DATABASE_URL is required but not set".into()))?;

    // Step 4: DB pool — sizes from manifest [database] section
    let pool_max = manifest.database.as_ref().map_or(12, |db| db.pool_max);
    let pool_min = manifest.database.as_ref().map_or(2, |db| db.pool_min);
    let acquire_timeout_secs = manifest
        .database
        .as_ref()
        .map_or(5, |db| db.pool_acquire_timeout_secs);
    let idle_timeout_secs = manifest
        .database
        .as_ref()
        .map_or(300, |db| db.pool_idle_timeout_secs);
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(pool_max)
        .min_connections(pool_min)
        .acquire_timeout(std::time::Duration::from_secs(acquire_timeout_secs))
        .idle_timeout(std::time::Duration::from_secs(idle_timeout_secs))
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

    let mut nats_client: Option<async_nats::Client> = None;

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

            // Ensure all platform JetStream streams exist with the correct dedup windows.
            // This is idempotent — safe to run on every startup. Non-fatal: a failure here
            // means streams may use the default 2-minute window rather than the configured
            // class window, but the module can still function.
            match event_bus::ensure_platform_streams(client.clone()).await {
                Ok(()) => {
                    tracing::info!(module = %manifest.module.name, "JetStream stream dedup windows applied");
                }
                Err(e) => {
                    tracing::warn!(
                        module = %manifest.module.name,
                        error = %e,
                        "JetStream stream setup failed — streams may use default 2-minute dedup window"
                    );
                }
            }

            nats_client = Some(client.clone());
            Some(Arc::new(event_bus::NatsBus::new(client)))
        }
        "inmemory" => {
            tracing::info!(module = %manifest.module.name, "in-memory event bus created");
            Some(Arc::new(event_bus::InMemoryBus::new()))
        }
        _ => None, // "none", missing section
    };

    // Step 7: outbox publisher / undeclared outbox detection
    let publish_section = manifest.events.as_ref().and_then(|e| e.publish.as_ref());
    let outbox_table = publish_section.map(|p| p.outbox_table.clone());
    let subject_prefix = publish_section.and_then(|p| p.subject_prefix.clone());

    let mut outbox_handle: Option<tokio::task::JoinHandle<()>> = None;
    let mut outbox_shutdown_tx: Option<tokio::sync::watch::Sender<bool>> = None;

    // Auto-create the outbox table when requested.
    if let Some(ref ps) = publish_section {
        if ps.auto_create {
            publisher::ensure_outbox_table(&pool, &ps.outbox_table).await?;
        }
    }

    if let Some(ref table) = outbox_table {
        if skip_outbox {
            tracing::info!(
                module = %manifest.module.name,
                outbox_table = %table,
                "outbox publisher skipped — module manages its own publishing"
            );
        } else if let Some(ref bus) = bus {
            let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
            outbox_shutdown_tx = Some(shutdown_tx);

            if let Some(ref resolver) = pool_resolver {
                // Multi-tenant: publish from every tenant database
                let pub_resolver = resolver.clone();
                let pub_bus = bus.clone();
                let pub_table = table.clone();
                let pub_module = manifest.module.name.clone();
                let pub_prefix = subject_prefix.clone();
                outbox_handle = Some(tokio::spawn(async move {
                    publisher::run_multi_tenant_outbox_publisher(
                        pub_resolver,
                        pub_bus,
                        &pub_table,
                        &pub_module,
                        pub_prefix.as_deref(),
                        shutdown_rx,
                    )
                    .await;
                }));
                tracing::info!(
                    module = %manifest.module.name,
                    outbox_table = %table,
                    "multi-tenant outbox publisher task spawned"
                );
            } else {
                // Single-DB: publish from the default pool
                let pub_pool = pool.clone();
                let pub_bus = bus.clone();
                let pub_table = table.clone();
                let pub_module = manifest.module.name.clone();
                let pub_prefix = subject_prefix.clone();
                outbox_handle = Some(tokio::spawn(async move {
                    publisher::run_outbox_publisher(
                        pub_pool,
                        pub_bus,
                        &pub_table,
                        &pub_module,
                        pub_prefix.as_deref(),
                        shutdown_rx,
                    )
                    .await;
                }));
                tracing::info!(
                    module = %manifest.module.name,
                    outbox_table = %table,
                    "outbox publisher task spawned"
                );
            }
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

    // Step 10: enforce auth-required (fail-closed by default).
    //
    // When [auth] is absent, required defaults to true — modules must
    // explicitly opt out with `[auth] required = false`.  The builder's
    // `.skip_auth()` also bypasses this check (the module handles its own
    // authentication).
    let auth_required = manifest.auth.as_ref().map_or(true, |a| a.required);
    if auth_required && !skip_auth && jwt_verifier.is_none() {
        return Err(StartupError::Config(
            "authentication is required but no JWT verifier could be created — \
             set auth.jwks_url or the JWT_PUBLIC_KEY env var, \
             or add [auth] required = false to module.toml"
                .into(),
        ));
    }

    // Step 9: rate limiter — tiered if any tiers are configured, simple otherwise.
    let rate_limiter = if !builder_tiers.is_empty() {
        tracing::info!(
            module = %manifest.module.name,
            tiers = builder_tiers.len(),
            "tiered rate limiter active"
        );
        RateLimiterKind::Tiered(Arc::new(TieredRateLimiter::from_defs(builder_tiers)))
    } else if let Some(ref rl) = manifest.rate_limit {
        let window = std::time::Duration::from_secs(1);
        RateLimiterKind::Simple(Arc::new(RateLimiter::with_configs(
            RateLimitConfig::new(rl.burst, window),
            RateLimitConfig::new((rl.requests_per_second / 10).max(1), window),
        )))
    } else {
        RateLimiterKind::Simple(security::middleware::default_rate_limiter())
    };

    Ok(PhaseAOutput {
        pool,
        bus,
        nats_client,
        jwt_verifier,
        rate_limiter,
        outbox_handle,
        outbox_shutdown_tx,
    })
}

/// Which rate limiter variant is active for this module.
pub(crate) enum RateLimiterKind {
    /// Single-tier (backwards-compatible, no tier config).
    Simple(Arc<RateLimiter>),
    /// Multi-tier with per-tier token buckets and route-based dispatch.
    Tiered(Arc<TieredRateLimiter>),
}

pub(crate) struct PhaseAOutput {
    pub pool: sqlx::PgPool,
    pub bus: Option<Arc<dyn EventBus>>,
    pub nats_client: Option<async_nats::Client>,
    pub jwt_verifier: Option<Arc<JwtVerifier>>,
    pub rate_limiter: RateLimiterKind,
    pub outbox_handle: Option<tokio::task::JoinHandle<()>>,
    /// Shutdown sender for the outbox publisher. Send `true` to request stop.
    pub outbox_shutdown_tx: Option<tokio::sync::watch::Sender<bool>>,
}

/// Granular middleware opt-out flags for [`phase_b`].
///
/// Each flag disables one layer of the SDK's default middleware stack
/// without touching the others. The zero-value (all `false`) is
/// equivalent to the full default stack.
pub(crate) struct MiddlewareFlags {
    /// Skip the CORS layer (e.g. reverse proxy already adds headers).
    pub skip_cors: bool,
    /// Skip the rate-limiting layer (e.g. throttling is at the gateway).
    pub skip_rate_limit: bool,
    /// Skip the JWT authentication layer (e.g. internal-only traffic).
    pub skip_auth: bool,
    /// Skip the tracing context middleware (e.g. proxy already injects trace IDs).
    pub skip_tracing: bool,
    /// Enable CSRF protection (double-submit cookie pattern).
    /// Off by default — enable for browser-facing verticals.
    pub csrf_protection: bool,
    /// Optional route-level permission enforcement configuration.
    /// When `Some`, an AuthzGate middleware layer is inserted after JWT auth.
    pub authz_gate: Option<Arc<crate::authz_gate::AuthzGateConfig>>,
    /// Optional tenant readiness checker wired into `GET /api/ready?tenant_id=`.
    /// When `None`, the `?tenant_id=` parameter is silently ignored.
    pub tenant_readiness: Option<Arc<dyn health::TenantReadinessCheck>>,
    /// Optional custom vitals provider for the `extended` field of `GET /api/vitals`.
    /// `GET /api/vitals` is always wired regardless of whether this is set.
    pub vitals_provider: Option<Arc<dyn crate::vitals::VitalsProvider>>,
    /// Outbox table name for vitals pending-count queries.
    /// Defaults to `"events_outbox"` if not set.
    pub vitals_outbox_table: Option<String>,
}

/// Phase B: HTTP stack assembly and server start.
pub(crate) async fn phase_b(
    manifest: &Manifest,
    phase_a: PhaseAOutput,
    module_routes: Router,
    migrator: Option<&sqlx::migrate::Migrator>,
    consumer_handles: ConsumerHandles,
    ctx: ModuleContext,
    flags: MiddlewareFlags,
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

    // Observability routes (health, ready, version, metrics) — always served.
    let health_deps: Vec<String> = manifest
        .health
        .as_ref()
        .map(|h| h.dependencies.clone())
        .unwrap_or_default();
    let probe_nats = health_deps.iter().any(|d| d == "nats");
    let obs_routes = build_observability_routes(
        module_name.clone(),
        version.clone(),
        phase_a.pool.clone(),
        phase_a.bus.clone(),
        probe_nats,
        flags.tenant_readiness.clone(),
        flags.vitals_provider.clone(),
        flags.vitals_outbox_table.clone(),
    );

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

    // Assemble the full app: module routes + observability + middleware.
    // Body limit and timeout are applied unconditionally. Tracing and each
    // optional layer is applied only when its flag is not set.
    let app = module_routes
        .merge(obs_routes)
        .layer(Extension(ctx))
        .layer(DefaultBodyLimit::max(body_limit))
        .layer(axum::middleware::from_fn(
            move |req, next: axum::middleware::Next| async move {
                match tokio::time::timeout(request_timeout, next.run(req)).await {
                    Ok(response) => response,
                    Err(_) => (StatusCode::REQUEST_TIMEOUT, "Request timeout\n").into_response(),
                }
            },
        ));

    let app = if !flags.skip_tracing {
        // platform_trace_middleware runs AFTER JWT auth (more outer layers run first),
        // so VerifiedClaims are available and enrich the span with tenant_id and actor_id.
        app.layer(axum::middleware::from_fn(platform_trace_middleware))
    } else {
        tracing::info!(module = %module_name, "tracing context middleware disabled");
        app
    };

    let app = if !flags.skip_rate_limit {
        match phase_a.rate_limiter {
            RateLimiterKind::Simple(limiter) => app
                .layer(axum::middleware::from_fn(rate_limit_middleware))
                .layer(Extension(limiter)),
            RateLimiterKind::Tiered(limiter) => app
                .layer(axum::middleware::from_fn(tiered_rate_limit_middleware))
                .layer(Extension(limiter)),
        }
    } else {
        tracing::info!(module = %module_name, "rate-limit middleware disabled");
        app
    };

    // AuthzGate is applied BEFORE auth in the builder chain so that auth
    // (the outer wrapper) runs first and injects VerifiedClaims, which
    // AuthzGate then reads.  Execution order: auth → authz_gate → ...
    let app = if let Some(config) = flags.authz_gate {
        tracing::info!(module = %module_name, "authz gate middleware enabled");
        app.layer(axum::middleware::from_fn_with_state(
            config,
            crate::authz_gate::authz_gate_middleware,
        ))
    } else {
        app
    };

    let app = if !flags.skip_auth {
        app.layer(axum::middleware::from_fn_with_state(
            phase_a.jwt_verifier,
            optional_claims_mw,
        ))
    } else {
        tracing::info!(module = %module_name, "auth middleware disabled");
        app
    };

    let app = if flags.csrf_protection {
        let csrf_config = std::sync::Arc::new(crate::csrf::CsrfConfig::from_env());
        tracing::info!(
            module = %module_name,
            secure = %csrf_config.secure,
            "CSRF double-submit middleware enabled"
        );
        app.layer(axum::middleware::from_fn_with_state(
            csrf_config,
            crate::csrf::csrf_middleware,
        ))
    } else {
        app
    };

    let app = if !flags.skip_cors {
        app.layer(build_cors_layer(manifest)?)
    } else {
        tracing::info!(module = %module_name, "CORS middleware disabled");
        app
    };

    let app = app.into_make_service_with_connect_info::<SocketAddr>();

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
        if let Some(tx) = phase_a.outbox_shutdown_tx {
            let _ = tx.send(true);
        }
        let _ = handle.await;
    }
    tracing::info!(module = %module_name, "closing resources");
    shutdown_pool.close().await;
    tracing::info!(module = %module_name, "shutdown complete");

    Ok(())
}

/// Query parameters for the `/api/ready` endpoint.
#[derive(serde::Deserialize)]
struct ReadyParams {
    /// When provided, runs the module's `TenantReadinessCheck` in addition to global checks.
    /// Returns a `tenant` object in the response body. Does not affect the HTTP status code.
    tenant_id: Option<uuid::Uuid>,
}

#[derive(serde::Deserialize)]
struct VitalsParams {
    /// When provided, scopes projection queries and tenant_ready to this tenant.
    tenant_id: Option<uuid::Uuid>,
}

/// Build observability routes that are always served regardless of middleware config.
///
/// These are not middleware — they are endpoints that orchestrators (k8s, Docker)
/// depend on. Stripping them when a module skips middleware would break health checks.
pub(crate) fn build_observability_routes(
    module_name: String,
    version: String,
    pool: sqlx::PgPool,
    bus: Option<Arc<dyn EventBus>>,
    probe_nats: bool,
    tenant_readiness: Option<Arc<dyn health::TenantReadinessCheck>>,
    vitals_provider: Option<Arc<dyn crate::vitals::VitalsProvider>>,
    vitals_outbox_table: Option<String>,
) -> Router {
    if probe_nats && bus.is_none() {
        tracing::warn!(
            module = %module_name,
            "health.dependencies includes 'nats' but no event bus is configured — \
             NATS will report as down in health checks"
        );
    }

    let health_name = module_name.clone();
    let health_version = version.clone();
    let health_pool = pool.clone();
    let health_bus = if probe_nats { bus.clone() } else { None };

    let health_routes = Router::new()
        .route("/healthz", get(health::healthz))
        .route(
            "/api/health",
            get(move || async move {
                let mut checks = Vec::new();

                let start = std::time::Instant::now();
                let err = sqlx::query("SELECT 1")
                    .execute(&health_pool)
                    .await
                    .err()
                    .map(|e| e.to_string());
                let latency = start.elapsed().as_millis() as u64;
                checks.push(health::db_check(latency, err));

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
                let ready_pool = pool.clone();
                let ready_bus = if probe_nats { bus.clone() } else { None };
                let ready_checker = tenant_readiness.clone();
                move |Query(params): Query<ReadyParams>| async move {
                    let mut checks = Vec::new();

                    let start = std::time::Instant::now();
                    let err = sqlx::query("SELECT 1")
                        .execute(&ready_pool)
                        .await
                        .err()
                        .map(|e| e.to_string());
                    let latency = start.elapsed().as_millis() as u64;
                    checks.push(health::db_check(latency, err));

                    if let Some(ref bus) = ready_bus {
                        let nats_start = std::time::Instant::now();
                        let connected = bus.health_check().await;
                        let nats_latency = nats_start.elapsed().as_millis() as u64;
                        checks.push(health::nats_check(connected, nats_latency));
                    }

                    let mut resp =
                        health::build_ready_response(&ready_name, &ready_version, checks);

                    // Tenant-scoped probe — only runs when ?tenant_id= is supplied.
                    // A non-ready tenant returns `status: "warming"` but does NOT
                    // change the global HTTP status code. If the probe times out or
                    // the checker is absent, the field is silently omitted.
                    if let (Some(tid), Some(checker)) = (params.tenant_id, ready_checker.as_ref()) {
                        let checker = Arc::clone(checker);
                        let tenant_ready = tokio::time::timeout(
                            std::time::Duration::from_millis(500),
                            async move { checker.is_ready(tid) },
                        )
                        .await
                        .unwrap_or(false);

                        resp.tenant = Some(health::TenantReadiness {
                            id: tid.to_string(),
                            status: if tenant_ready {
                                health::TenantReadyStatus::Up
                            } else {
                                health::TenantReadyStatus::Warming
                            },
                        });
                    }

                    health::ready_response_to_axum(resp)
                }
            }),
        )
        .route(
            "/api/vitals",
            get({
                let vitals_name = module_name.clone();
                let vitals_version = version.clone();
                let vitals_pool = pool.clone();
                let vitals_readiness = tenant_readiness.clone();
                let custom_provider = vitals_provider;
                let outbox_table = vitals_outbox_table;
                move |Query(params): Query<VitalsParams>| async move {
                    use crate::vitals::StandardVitalsProvider;

                    let provider = match outbox_table {
                        Some(ref t) => StandardVitalsProvider::with_outbox_table(t),
                        None => StandardVitalsProvider::new(),
                    };

                    let tenant_id = params.tenant_id;

                    let tenant_ready =
                        if let (Some(tid), Some(ref checker)) = (tenant_id, &vitals_readiness) {
                            Some(checker.is_ready(tid))
                        } else {
                            None
                        };

                    let extended = if let Some(ref prov) = custom_provider {
                        let val = prov.collect_extended(&vitals_pool, tenant_id).await;
                        if val.is_null() { None } else { Some(val) }
                    } else {
                        None
                    };

                    let resp = provider
                        .collect(
                            &vitals_pool,
                            &vitals_name,
                            &vitals_version,
                            tenant_id,
                            tenant_ready,
                            extended,
                        )
                        .await;

                    Json(resp)
                }
            }),
        )
        .route(
            "/api/version",
            get({
                let ver = version;
                let name = module_name;
                move || async move {
                    Json(serde_json::json!({
                        "module": name,
                        "version": ver,
                    }))
                }
            }),
        );

    let metrics_route = Router::new().route(
        "/metrics",
        get(|| async {
            let encoder = prometheus::TextEncoder::new();
            let families = prometheus::gather();
            match encoder.encode_to_string(&families) {
                Ok(body) => (
                    StatusCode::OK,
                    [(
                        axum::http::header::CONTENT_TYPE,
                        "text/plain; version=0.0.4",
                    )],
                    body,
                ),
                Err(e) => (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    [(axum::http::header::CONTENT_TYPE, "text/plain")],
                    format!("metrics encoding error: {e}"),
                ),
            }
        }),
    );

    health_routes.merge(metrics_route)
}
