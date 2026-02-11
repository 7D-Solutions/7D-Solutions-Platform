mod config;
mod db;
mod middleware;
mod routes;
mod auth;
mod events;
mod metrics;
mod rate_limit;
mod jetstream_setup;

use axum::{middleware::from_fn_with_state, routing::get, Router};
use std::sync::Arc;
use tower_http::trace::TraceLayer;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

// TODO: Re-enable when tower_governor compatibility with axum 0.7 is fixed
// use tower_governor::{governor::GovernorConfigBuilder, GovernorLayer};

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
    let metrics = metrics::Metrics::new();

    // DB + migrations (fail-fast)
    let pool = db::create_pool(&cfg.database_url).await?;
    db::run_migrations(&pool).await?;
    metrics.dep_up.with_label_values(&["db"]).set(1);

    // NATS (fail-fast)
    let nats = async_nats::connect(&cfg.nats_url).await?;
    metrics.dep_up.with_label_values(&["nats"]).set(1);

    // JetStream streams (events + DLQ)
    jetstream_setup::ensure_streams(nats.clone()).await?;

    // Load schemas
    let schema_dir = std::env::var("SCHEMA_DIR").unwrap_or_else(|_| "/app/schemas".into());
    crate::events::validate::load_schemas_from_dir(&schema_dir)?;

    // JWT keys
    let jwt = crate::auth::jwt::JwtKeys::from_pem(
        &cfg.jwt_private_key_pem.replace("\\n", "\n"),
        &cfg.jwt_public_key_pem.replace("\\n", "\n"),
        cfg.jwt_kid.clone(),
    )?;

    // Password policy
    let pwd = crate::auth::password::PasswordPolicy {
        memory_kb: cfg.argon_memory_kb,
        iterations: cfg.argon_iterations,
        parallelism: cfg.argon_parallelism,
    };

    // Event publisher
    let producer = format!("auth-rs@{}", env!("CARGO_PKG_VERSION"));
    let events = crate::events::publisher::EventPublisher::new(nats.clone());

    // Keyed limiters
    let keyed_limits = rate_limit::KeyedLimiters::new();

    // Hash concurrency limiter
    let hash_limiter = crate::auth::concurrency::HashConcurrencyLimiter::new(
        cfg.max_concurrent_hashes,
        cfg.hash_acquire_timeout_ms,
    );

    let auth_state = Arc::new(crate::auth::handlers::AuthState {
        db: pool.clone(),
        jwt: jwt.clone(),
        pwd,
        access_ttl_minutes: cfg.access_token_ttl_minutes,
        refresh_ttl_days: cfg.refresh_token_ttl_days,
        events,
        producer,
        metrics: metrics.clone(),
        keyed_limits,
        hash_limiter,
        lockout_threshold: cfg.lockout_threshold,
        lockout_minutes: cfg.lockout_minutes,
        login_per_min_per_email: cfg.login_per_min_per_email,
        register_per_min_per_email: cfg.register_per_min_per_email,
        refresh_per_min_per_token: cfg.refresh_per_min_per_token,
    });

    // Health + Metrics states
    let health_state = Arc::new(routes::health::HealthState {
        db: pool,
        nats,
        metrics: metrics.clone(),
    });

    let metrics_state = Arc::new(routes::metrics::MetricsState {
        metrics: metrics.clone(),
    });

    let metrics_mw_state = Arc::new(middleware::metrics::MetricsMiddlewareState {
        metrics: metrics.clone(),
    });

    // JWKS state
    let jwks_state = Arc::new(routes::jwks::JwksState { jwt });

    // Per-IP governor (global safety net)
    // NOTE: Temporarily disabled due to tower_governor/axum compatibility issues
    // TODO: Re-enable with working configuration or alternative rate limiting approach
    // let governor_conf = GovernorConfigBuilder::default()
    //     .per_second(cfg.ip_rl_per_second as u64)
    //     .burst_size(cfg.ip_rl_burst as u32)
    //     .finish()
    //     .unwrap();
    // let governor_layer = GovernorLayer::new(governor_conf);

    // Build routers separately then merge
    let health_router = Router::new()
        .route("/health/live", get(routes::health::health_live))
        .route("/health/ready", get(routes::health::health_ready))
        .with_state(health_state);

    let metrics_router = Router::new()
        .route("/metrics", get(routes::metrics::metrics))
        .with_state(metrics_state);

    let jwks_router = Router::new()
        .route("/.well-known/jwks.json", get(routes::jwks::jwks_handler))
        .with_state(jwks_state);

    let auth_router = routes::auth::router(auth_state);

    let app = Router::new()
        .merge(health_router)
        .merge(metrics_router)
        .merge(jwks_router)
        .merge(auth_router)
        // request duration metrics
        .layer(from_fn_with_state(
            metrics_mw_state,
            middleware::metrics::metrics_middleware,
        ))
        // client IP and user agent extraction
        .layer(axum::middleware::from_fn(middleware::client_ip::client_meta_middleware))
        // trace id propagation
        .layer(axum::middleware::from_fn(middleware::tracing::trace_id_middleware))
        // per-IP limiter - TODO: re-enable when tower_governor works with axum 0.7
        // .layer(governor_layer)
        .layer(TraceLayer::new_for_http());

    let addr = format!("{}:{}", cfg.host, cfg.port);
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    tracing::info!(%addr, "listening");
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
    .await?;

    Ok(())
}
