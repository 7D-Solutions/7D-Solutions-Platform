use axum::{extract::DefaultBodyLimit, routing::get, Extension, Router};
use security::{
    middleware::{
        default_rate_limiter, rate_limit_middleware, timeout_middleware, DEFAULT_BODY_LIMIT,
    },
    optional_claims_mw, permissions, JwtVerifier, RequirePermissionsLayer,
};
use std::net::SocketAddr;
use std::sync::Arc;
use tower_http::cors::{AllowOrigin, CorsLayer};
use tracing_subscriber::EnvFilter;

use bom_rs::{
    db::resolver::resolve_pool,
    http::{
        bom_routes::{
            delete_line, get_bom, get_bom_by_part_id, get_explosion, get_lines, get_where_used,
            list_boms, list_revisions, post_bom, post_effectivity, post_line, post_revision,
            put_line,
        },
        eco_routes::{
            get_bom_revision_links, get_doc_revision_links, get_eco, get_eco_audit,
            get_eco_history_for_part, post_apply, post_approve, post_eco, post_link_bom_revision,
            post_link_doc_revision, post_reject, post_submit,
        },
        health::{health as health_fn, ready, version},
    },
    metrics::{metrics_handler, BomMetrics},
    AppState, Config, NumberingClient,
};

#[tokio::main]
async fn main() {
    dotenvy::dotenv().ok();

    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();

    tracing::info!("Starting BOM service...");

    let config = Config::from_env().unwrap_or_else(|err| {
        eprintln!("Configuration error: {}", err);
        std::process::exit(1);
    });

    tracing::info!(
        "Configuration loaded: host={}, port={}",
        config.host,
        config.port
    );

    let pool = resolve_pool(&config.database_url)
        .await
        .expect("Failed to connect to database");

    sqlx::migrate!("./db/migrations")
        .run(&pool)
        .await
        .expect("BOM: failed to run database migrations");

    let shutdown_pool = pool.clone();
    let metrics = Arc::new(BomMetrics::new().expect("Failed to create metrics registry"));
    let numbering = NumberingClient::http(config.numbering_url.clone());
    let app_state = Arc::new(AppState {
        pool,
        metrics,
        numbering,
    });

    let maybe_verifier = JwtVerifier::from_env_with_overlap().map(Arc::new);

    let bom_mutations = Router::new()
        .route("/api/bom", axum::routing::post(post_bom))
        .route(
            "/api/bom/{bom_id}/revisions",
            axum::routing::post(post_revision),
        )
        .route(
            "/api/bom/revisions/{revision_id}/effectivity",
            axum::routing::post(post_effectivity),
        )
        .route(
            "/api/bom/revisions/{revision_id}/lines",
            axum::routing::post(post_line),
        )
        .route(
            "/api/bom/lines/{line_id}",
            axum::routing::put(put_line).delete(delete_line),
        )
        // ECO mutations
        .route("/api/eco", axum::routing::post(post_eco))
        .route("/api/eco/{eco_id}/submit", axum::routing::post(post_submit))
        .route(
            "/api/eco/{eco_id}/approve",
            axum::routing::post(post_approve),
        )
        .route("/api/eco/{eco_id}/reject", axum::routing::post(post_reject))
        .route("/api/eco/{eco_id}/apply", axum::routing::post(post_apply))
        .route(
            "/api/eco/{eco_id}/bom-revisions",
            axum::routing::post(post_link_bom_revision),
        )
        .route(
            "/api/eco/{eco_id}/doc-revisions",
            axum::routing::post(post_link_doc_revision),
        )
        .route_layer(RequirePermissionsLayer::new(&[permissions::BOM_MUTATE]))
        .with_state(app_state.clone());

    let bom_reads = Router::new()
        .route("/api/bom", axum::routing::get(list_boms))
        .route("/api/bom/{bom_id}", axum::routing::get(get_bom))
        .route(
            "/api/bom/by-part/{part_id}",
            axum::routing::get(get_bom_by_part_id),
        )
        .route(
            "/api/bom/{bom_id}/revisions",
            axum::routing::get(list_revisions),
        )
        .route(
            "/api/bom/revisions/{revision_id}/lines",
            axum::routing::get(get_lines),
        )
        .route(
            "/api/bom/{bom_id}/explosion",
            axum::routing::get(get_explosion),
        )
        .route(
            "/api/bom/where-used/{item_id}",
            axum::routing::get(get_where_used),
        )
        // ECO reads
        .route("/api/eco/{eco_id}", axum::routing::get(get_eco))
        .route(
            "/api/eco/{eco_id}/bom-revisions",
            axum::routing::get(get_bom_revision_links),
        )
        .route(
            "/api/eco/{eco_id}/doc-revisions",
            axum::routing::get(get_doc_revision_links),
        )
        .route("/api/eco/{eco_id}/audit", axum::routing::get(get_eco_audit))
        .route(
            "/api/eco/history/{part_id}",
            axum::routing::get(get_eco_history_for_part),
        )
        .route_layer(RequirePermissionsLayer::new(&[permissions::BOM_READ]))
        .with_state(app_state.clone());

    let app = Router::new()
        .route("/healthz", get(health::healthz))
        .route("/api/health", get(health_fn))
        .route("/api/ready", get(ready))
        .route("/api/version", get(version))
        .route("/metrics", get(metrics_handler))
        .with_state(app_state)
        .merge(bom_reads)
        .merge(bom_mutations)
        .layer(DefaultBodyLimit::max(DEFAULT_BODY_LIMIT))
        .layer(axum::middleware::from_fn(
            security::tracing::tracing_context_middleware,
        ))
        .layer(axum::middleware::from_fn(timeout_middleware))
        .layer(axum::middleware::from_fn(rate_limit_middleware))
        .layer(Extension(default_rate_limiter()))
        .layer(axum::middleware::from_fn_with_state(
            maybe_verifier,
            optional_claims_mw,
        ))
        .layer(build_cors_layer(&config))
        .into_make_service_with_connect_info::<SocketAddr>();

    let addr = SocketAddr::from(([0, 0, 0, 0], config.port));
    tracing::info!("BOM service listening on {}", addr);

    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .expect("Failed to bind address");

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .expect("Server failed to start");

    tracing::info!("Server stopped — closing resources");
    shutdown_pool.close().await;
    tracing::info!("Shutdown complete");
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

fn build_cors_layer(config: &Config) -> CorsLayer {
    let is_wildcard = config.cors_origins.len() == 1 && config.cors_origins[0] == "*";

    let layer = if is_wildcard {
        CorsLayer::new().allow_origin(AllowOrigin::any())
    } else {
        let origins: Vec<_> = config
            .cors_origins
            .iter()
            .filter_map(|o| o.parse().ok())
            .collect();
        CorsLayer::new().allow_origin(origins)
    };

    layer
        .allow_methods(tower_http::cors::Any)
        .allow_headers(tower_http::cors::Any)
}
