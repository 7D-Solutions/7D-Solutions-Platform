//! Helper functions for the startup module.

use tower_http::cors::{AllowOrigin, CorsLayer};

use crate::manifest::Manifest;
use crate::startup::StartupError;

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

/// Returns `true` if `pattern` is broad enough to match any origin (wildcard).
///
/// Checks common literal wildcards first, then compiles and tests against a
/// deliberately diverse set of origins — if all match, the pattern is too broad.
fn is_wildcard_regex(pattern: &str) -> bool {
    let p = pattern.trim();
    if matches!(p, ".*" | "^.*$" | "^.+$" | ".+" | "^.*" | ".*$") {
        return true;
    }
    regex::Regex::new(pattern)
        .map(|re| {
            let diverse = [
                "https://evil.com",
                "ftp://other.net:9999",
                "http://localhost:1",
            ];
            diverse.iter().all(|o| re.is_match(o))
        })
        .unwrap_or(false)
}

/// Build a fail-closed CORS layer from the module manifest.
///
/// Policy (fail-closed):
///  1. Manifest `[cors]` section present:
///     - `origin_pattern` → compile regex; wildcard pattern → `StartupError::Config`.
///     - `origins` list → explicit allowlist; `"*"` entry → `StartupError::Config`;
///       empty list → layer that rejects all cross-origin requests (internal-only posture).
///  2. No manifest `[cors]` section → consult `CORS_ORIGINS` env var as operator override:
///     - Explicit list → build layer from it.
///     - `"*"` → `StartupError::Config`.
///     - Unset → `StartupError::Config` (no policy declared anywhere).
pub fn build_cors_layer(manifest: &Manifest) -> Result<CorsLayer, StartupError> {
    let module_name = &manifest.module.name;

    // 1. Manifest cors.origin_pattern takes priority.
    if let Some(ref pattern) = manifest
        .cors
        .as_ref()
        .and_then(|c| c.origin_pattern.clone())
    {
        if is_wildcard_regex(pattern) {
            return Err(StartupError::Config(format!(
                "module '{module_name}': manifest.cors.origin_pattern '{pattern}' matches all \
                 origins — use a specific domain pattern that restricts to your allowed origins"
            )));
        }
        let re = regex::Regex::new(pattern).expect("manifest validate() ensures valid regex");
        tracing::info!(
            module = %module_name,
            pattern = %pattern,
            "CORS origin_pattern from manifest"
        );
        return Ok(CorsLayer::new()
            .allow_origin(AllowOrigin::predicate(move |origin, _| {
                origin.to_str().map_or(false, |s| re.is_match(s))
            }))
            .allow_methods(tower_http::cors::Any)
            .allow_headers(tower_http::cors::Any)
            .allow_credentials(false));
    }

    // 2. Manifest cors.origins → explicit list (including empty = internal-only).
    if let Some(ref origins) = manifest.cors.as_ref().and_then(|c| c.origins.clone()) {
        if origins.iter().any(|o| o == "*") {
            return Err(StartupError::Config(format!(
                "module '{module_name}': manifest.cors.origins contains '*' — \
                 wildcard is not permitted; list explicit origins or use origins = [] \
                 for server-to-server modules"
            )));
        }
        tracing::info!(
            module = %module_name,
            count = origins.len(),
            "CORS origins from manifest"
        );
        let parsed: Vec<_> = origins.iter().filter_map(|o| o.parse().ok()).collect();
        return Ok(CorsLayer::new()
            .allow_origin(parsed)
            .allow_methods(tower_http::cors::Any)
            .allow_headers(tower_http::cors::Any)
            .allow_credentials(false));
    }

    // 3. No manifest [cors] section — check CORS_ORIGINS env var as operator override.
    match std::env::var("CORS_ORIGINS") {
        Ok(cors_env) => {
            let origins: Vec<String> = cors_env
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();
            if origins.len() == 1 && origins[0] == "*" {
                return Err(StartupError::Config(format!(
                    "module '{module_name}': CORS_ORIGINS is set to '*' — \
                     wildcard operator override is not permitted; \
                     add a [cors] section to module.toml with explicit origins"
                )));
            }
            tracing::info!(
                module = %module_name,
                count = origins.len(),
                "CORS origins from CORS_ORIGINS env override"
            );
            let parsed: Vec<_> = origins.iter().filter_map(|o| o.parse().ok()).collect();
            Ok(CorsLayer::new()
                .allow_origin(parsed)
                .allow_methods(tower_http::cors::Any)
                .allow_headers(tower_http::cors::Any)
                .allow_credentials(false))
        }
        Err(_) => Err(StartupError::Config(format!(
            "module '{module_name}' has no [cors] section in module.toml and CORS_ORIGINS \
             is not set — add a [cors] section: \
             origins = [] for server-to-server modules, \
             origins = [\"https://...\"] for browser-facing modules"
        ))),
    }
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
