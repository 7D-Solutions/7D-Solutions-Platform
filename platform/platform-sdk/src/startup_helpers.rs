//! Helper functions for the startup module.

use tower_http::cors::{AllowOrigin, CorsLayer};

use crate::manifest::Manifest;

/// Parse a human-readable body-limit string (e.g. "2mb", "512kb") into bytes.
/// Falls back to 2 MiB on unrecognised input.
pub(crate) fn parse_body_limit(s: &str) -> usize {
    let s = s.trim().to_lowercase();
    if let Some(n) = s.strip_suffix("mb") {
        n.trim().parse::<usize>().unwrap_or(2) * 1024 * 1024
    } else if let Some(n) = s.strip_suffix("kb") {
        n.trim().parse::<usize>().unwrap_or(2048) * 1024
    } else if let Some(n) = s.strip_suffix("gb") {
        n.trim().parse::<usize>().unwrap_or(0) * 1024 * 1024 * 1024
    } else {
        s.parse::<usize>().unwrap_or(2 * 1024 * 1024)
    }
}

/// Parse a human-readable duration string (e.g. "30s", "5m") into `Duration`.
/// Falls back to 30 seconds on unrecognised input.
pub(crate) fn parse_duration_str(s: &str) -> std::time::Duration {
    let s = s.trim().to_lowercase();
    if let Some(n) = s.strip_suffix('s') {
        std::time::Duration::from_secs(n.trim().parse::<u64>().unwrap_or(30))
    } else if let Some(n) = s.strip_suffix('m') {
        std::time::Duration::from_secs(n.trim().parse::<u64>().unwrap_or(0) * 60)
    } else {
        std::time::Duration::from_secs(s.parse::<u64>().unwrap_or(30))
    }
}

/// CORS layer: manifest `[cors]` section takes priority, then `CORS_ORIGINS` env var fallback.
pub(crate) fn build_cors_layer(manifest: &Manifest) -> CorsLayer {
    let env_val = std::env::var("ENV").unwrap_or_else(|_| "development".to_string());

    // 1. Manifest cors.origin_pattern → regex predicate
    if let Some(ref pattern) = manifest.cors.as_ref().and_then(|c| c.origin_pattern.clone()) {
        let re = regex::Regex::new(pattern).expect("manifest validate() ensures valid regex");
        tracing::info!(
            module = %manifest.module.name,
            pattern = %pattern,
            "CORS origin_pattern from manifest"
        );
        return CorsLayer::new()
            .allow_origin(AllowOrigin::predicate(move |origin, _| {
                origin.to_str().map_or(false, |s| re.is_match(s))
            }))
            .allow_methods(tower_http::cors::Any)
            .allow_headers(tower_http::cors::Any)
            .allow_credentials(false);
    }

    // 2. Manifest cors.origins → explicit list
    if let Some(ref origins) = manifest.cors.as_ref().and_then(|c| c.origins.clone()) {
        tracing::info!(
            module = %manifest.module.name,
            count = origins.len(),
            "CORS origins from manifest"
        );
        let parsed: Vec<_> = origins.iter().filter_map(|o| o.parse().ok()).collect();
        return CorsLayer::new()
            .allow_origin(parsed)
            .allow_methods(tower_http::cors::Any)
            .allow_headers(tower_http::cors::Any)
            .allow_credentials(false);
    }

    // 3. Fallback: CORS_ORIGINS env var (existing behavior)
    let cors_env = std::env::var("CORS_ORIGINS").unwrap_or_else(|_| "*".to_string());
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

pub(crate) async fn shutdown_signal() {
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
