# SDK Auth for Verticals: Investigation Finding

**Bead:** bd-47mzv
**Investigator:** DarkCrane
**Date:** 2026-04-02

## Executive Summary

The SDK auth **does work** for verticals and should be the recommended path. The JWKS flow (`[auth] jwks_url`) is implemented, tested in fallback paths, and structurally sound. However, **no module has ever used it**, and `skip_default_middleware()` is the real reason verticals bypass SDK auth: it's all-or-nothing, so any vertical needing custom CORS or health endpoints loses SDK auth as collateral damage.

## Finding: SDK Auth Works, But skip_default_middleware Forces Bypass

### What the SDK provides

1. **`[auth]` manifest section** — `jwks_url`, `refresh_interval` (default 5m), `fallback_to_env` (default true), `enabled` (default true)
2. **`JwtVerifier::from_jwks_url()`** — fetches JWKS, spawns background refresh, falls back to `JWT_PUBLIC_KEY` env var if JWKS endpoint is down
3. **`optional_claims_mw`** — permissive middleware: extracts Bearer token, verifies, inserts `VerifiedClaims` into request extensions. Passes through if no token.
4. **`RequirePermissionsLayer`** — enforces permission strings on protected routes. Returns 401 if no claims, 403 if permissions missing.

### How platform modules use it today

**No module uses `[auth]` in module.toml.** All 20 modules rely on the implicit fallback: when no `[auth]` section exists, `startup.rs:194` calls `JwtVerifier::from_env_with_overlap()`, reading `JWT_PUBLIC_KEY` from environment. This works because platform modules share the deployment environment with identity-auth.

**customer-portal** is the only module with its own JWT system (portal-specific tokens for end-customers), but it still uses the SDK's Phase B middleware stack — it doesn't call `skip_default_middleware()`.

### Why verticals bypass it

| Vertical | Lines of custom auth | Root cause |
|----------|---------------------|------------|
| Fireproof | 1,874 | Separate deployable; needs JWKS discovery. Also has vertical-specific RBAC scopes, audit logging, CSRF. |
| TrashTech | 429 | Uses `skip_default_middleware()` for custom CORS + health endpoints. Auth is collateral damage. |

**The all-or-nothing problem:** `skip_default_middleware()` disables the entire Phase B stack:
- JWT auth (`optional_claims_mw`)
- Rate limiting
- Request timeout
- CORS
- Body limit
- Tracing context

If a vertical needs custom CORS or custom health routes, it must skip everything and reimplement auth on its own. This is `phase_b_raw` in `startup.rs:356-441` — it serves module routes + observability routes with zero middleware.

### What a vertical needs to use SDK auth

A vertical that does NOT call `skip_default_middleware()` gets SDK auth automatically:

```toml
# module.toml
[auth]
jwks_url = "http://7d-auth-lb:8080/.well-known/jwks.json"
refresh_interval = "5m"
fallback_to_env = true
```

The SDK fetches JWKS keys at startup, refreshes every 5 minutes, and wires `optional_claims_mw` + the full middleware stack. The vertical uses `RequirePermissionsLayer` on protected routes.

### Proof: integration test

See `platform/platform-sdk/tests/sdk_auth_vertical.rs` — tests the full JWKS endpoint -> JwtVerifier -> optional_claims_mw -> RequirePermissionsLayer chain against a real local JWKS server.

## Gaps

### Gap 1: skip_default_middleware is all-or-nothing (BLOCKING)

**Impact:** Any vertical needing custom CORS, health, or metrics must bypass SDK auth entirely.

**Fix:** Replace the boolean `skip_default_middleware` with granular flags:
```rust
.skip_cors()          // Use own CORS
.skip_health()        // Use own health/ready endpoints
.skip_rate_limit()    // Use own rate limiting
.skip_auth()          // Use own JWT verification (rare)
```

Or alternatively, keep SDK auth always-on and let verticals merge their custom routes alongside SDK middleware:
```rust
.custom_cors(my_cors_layer)   // Override SDK CORS
.custom_health(my_health_fn)  // Override SDK health endpoints
```

### Gap 2: JWKS path never tested against real endpoint

**Impact:** `from_jwks_url()` works in code review and fallback tests, but no test exercises a successful JWKS fetch + token verification.

**Fix:** Integration test added in this bead (see `sdk_auth_vertical.rs`).

### Gap 3: No vertical has ever deployed with [auth] jwks_url

**Impact:** The happy path is unproven in production. Could have issues with JWKS endpoint availability, background refresh timing, key rotation across identity-auth restarts.

**Fix:** Run the JWKS path in staging before recommending it for Fireproof migration.

### Gap 4: Vertical-specific RBAC scopes

**Impact:** Fireproof has 21 gauge-specific scopes (`gauges:read`, `calibrations:create`). The SDK's `RequirePermissionsLayer` checks `VerifiedClaims.perms`, which comes from the JWT. Verticals need their own scope constants, but identity-auth must issue tokens with those scopes. This is a provisioning/configuration concern, not an SDK gap.

**Fix:** Document that vertical scopes go in the vertical's permission constants module, and identity-auth must be configured to issue tokens with those scopes. `RequirePermissionsLayer` already handles arbitrary permission strings.

## Recommendations

1. **Create a bead** to split `skip_default_middleware()` into granular skip flags. This is the single change that would let TrashTech use SDK auth.
2. **Create a bead** to run the JWKS auth path in staging against live identity-auth.
3. **Document** in CG-AUTH.md that verticals SHOULD use `[auth] jwks_url` rather than implementing their own JWT middleware.
4. **Do not create** a migration guide yet. The granular skip flags (recommendation 1) must land first, otherwise the guide would say "use SDK auth but also rewrite your CORS and health endpoints."

## Answer to Each Verification Question

1. **Write a test vertical with [auth] jwks_url pointing at identity-auth:** Done — integration test exercises the full chain (see `sdk_auth_vertical.rs`). In production, set `jwks_url = "http://7d-auth-lb:8080/.well-known/jwks.json"` in module.toml.

2. **Send a request with a valid JWT — does VerifiedClaims appear in extensions?** YES. `optional_claims_mw` extracts the Bearer token, calls `JwtVerifier::verify()`, and inserts `VerifiedClaims` into request extensions. Handlers access it via `Extension(claims): Extension<VerifiedClaims>`.

3. **Send a request without JWT — does it get 401?** DEPENDS ON THE ROUTE. `optional_claims_mw` is permissive — requests without JWT pass through with no claims. Routes protected by `RequirePermissionsLayer` return 401 when no claims are present. Unprotected routes (health, public endpoints) work normally. This is the correct behavior for a vertical with mixed public/protected routes.
