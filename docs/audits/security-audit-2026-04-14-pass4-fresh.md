# Security Audit — Pass 4 (Audit-only “Convergence Read”): 7D-Solutions Platform

Date: 2026-04-14  
Goal: Re-check the **highest-risk prior finding** with a stricter “would this actually ship insecurely?” lens — **no code changes**, audit documentation only.

## What Pass 4 is (in audit mode)

This mirrors the **Verify / fresh re-read** step from `multi-pass-bug-hunting`, but **without** the fix/rescan/test loop: we only tighten exploitability reasoning and identify **deployment preconditions** that turn a “warning” into an incident.

## Re-read target: default CORS fallback behavior

### Prior concern (from Pass 2)

`platform-sdk` builds a permissive CORS layer when `CORS_ORIGINS` is unset (defaults to `*`) and only warns when `ENV != development`.

### Convergence read (tighter exploitability model)

```35:107:/Users/james/Projects/7D-Solutions Platform/platform/platform-sdk/src/startup_helpers.rs
pub(crate) fn build_cors_layer(manifest: &Manifest) -> CorsLayer {
    let env_val = std::env::var("ENV").unwrap_or_else(|_| "development".to_string());
    // ...
    let cors_env = std::env::var("CORS_ORIGINS").unwrap_or_else(|_| "*".to_string());
    // ...
    if is_wildcard && env_val != "development" {
        tracing::warn!(/* ... */);
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
```

### Updated assessment

- **Severity (audit): Medium → “Medium (misconfiguration-dependent)”**
- **Why not automatically Critical:** `allow_credentials(false)` reduces the classic “any origin + credentialed cookies” browser exploit class.
- **What still makes it real:** Wildcard origins broaden **browser-based attack surface** for APIs that rely on other controls (cookies not involved, but CSRF-like cross-site posting patterns, token exfiltration via XSS elsewhere, etc.). Also, operational mistakes (`ENV` unset in prod) can silently widen exposure.

### Concrete verification questions (for a security engineer agent)

- Is `ENV` guaranteed set in all production manifests (not just “usually”)?
- Are any browser clients relying on credentialed flows despite `allow_credentials(false)` being false (double-check any custom header auth patterns)?
- Is there an org-wide policy test that fails CI if `CORS_ORIGINS` is `*` outside dev?

## Pass 4 output

- **No new criticals found** on this convergence read; the primary risk remains **misconfiguration + missing guardrails**, not an obvious “always exploitable” code bug from this snippet alone.
