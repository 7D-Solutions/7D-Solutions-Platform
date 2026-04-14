# security — Revision History

> **What this file is:** The complete record of every change to this module after it was proven. Agents modifying this module must add a row here before committing. Products adopting a new version read this file to understand what changed.
> **Standard:** See `docs/VERSIONING.md` for the rules governing this file.

## Revisions

| Version | Date | Bead | What Changed | Why | Breaking? |
|---------|------|------|-------------|-----|-----------|
| 1.8.0 | 2026-04-13 | bd-397ij | Add `TierDef` struct carrying `RateLimitKeyStrategy` + optional method filters per tier. `ratelimit.rs` supports composite (tenant+ip) and ip-only key strategies. `middleware.rs` dispatches by method + path prefix. Integration test verifies tier selection and 429 responses. | Tiered rate limiting required cross-module activation; builder-side `with_rate_limiting()` needs a richer TierDef than the previous tuple. | No |
| 1.2.0 | 2026-04-02 | bd-4lc6q | Store raw bearer token (RawBearerToken) in request extensions alongside VerifiedClaims | Verticals need raw JWT for proxying, webhooks, audit | No |
| 1.0.0 | 2026-03-28 | bd-3ctma | Initial proof. All tests passing. | Module build complete and core logic validated via tests. | No |
| 1.0.1 | 2026-03-28 | bd-29c9i.1 | Add PRODUCTION_MUTATE and PRODUCTION_READ permission constants. | Production module was the only module without permission constants — security audit finding. | No |
| 1.0.2 | 2026-03-28 | bd-29c9i.3 | Add CUSTOMER_PORTAL_ADMIN permission constant. | Customer portal admin routes incorrectly used party.mutate — separate privilege scope needed. | No |
| 1.0.3 | 2026-03-30 | bd-zbahz | `RequirePermissionsLayer` and `ClaimsMiddleware` (strict mode) now return JSON error bodies (`{error, message, request_id}`) on 401 Unauthorized and 403 Forbidden instead of empty responses. `request_id` populated from `TracingContext` when available. | Consumers parsing empty 401/403 bodies got deserialization errors instead of machine-readable error codes. All modules behind these layers are affected. | No |
| 1.0.4 | 2026-03-31 | bd-68y44 | `JwtVerifier::from_env()` and `from_env_with_overlap()` now fall back to `JWT_PUBLIC_KEY_PEM` when `JWT_PUBLIC_KEY` is not set. | `.env` uses `JWT_PUBLIC_KEY_PEM` but verifier only read `JWT_PUBLIC_KEY`, so all services ran without JWT verification enabled. | No |
| 1.7.0 | 2026-04-13 | bd-505dg | Add `PLATFORM_TENANTS_CREATE` permission constant (`platform.tenants.create`). Gates `POST /api/control/tenants` in the control-plane router via `RequirePermissionsLayer`. | Any valid JWT could previously create tenants — no RBAC gate existed on the provisioning endpoint. | No |
| 1.6.1 | 2026-04-11 | bd-s56d3 | Normalize newlines in service_auth.rs private key loading — precautionary fix while diagnosing nil-tenant propagation in PlatformClient bearer tokens | Hardening during bd-s56d3 e2e test debug; found while tracing why BOM→Inventory calls were arriving with tenant_id=00000000... | No |
| 1.6.0 | 2026-04-10 | bd-6sle9 | RateLimitKeyStrategy enum (Composite/IpOnly/TenantOnly). TieredRateLimiter::with_strategies() accepts per-tier strategy; TieredRateLimiter::new() defaults to Composite (backward compatible). 6 new integration tests. | Verticals need per-tier key strategy control — single Composite key was insufficient for all rate limit scenarios | No |
| 1.5.0 | 2026-04-10 | bd-615tl | `mint_service_jwt_with_context(tenant_id, actor_id)` — service JWTs now carry real caller context instead of nil UUIDs. Issuer/audience corrected to auth-rs/7d-platform. | Receiving services saw nil tenant_id on cross-service calls, breaking tenant isolation | No |
| 1.4.0 | 2026-04-09 | bd-qv6ov | get_service_token() now mints RSA-signed JWTs (RS256) using JWT_PRIVATE_KEY_PEM when available, instead of HMAC tokens. RSA tokens are compatible with ClaimsLayer/JwtVerifier — HMAC tokens were not, causing all service-to-service calls to fail with 'no claims present'. HMAC fallback retained for environments without the private key. | Service-to-service auth was fundamentally broken: HMAC tokens passed signature check but ClaimsLayer (RSA-only) could not decode them, so no claims were set and RequirePermissionsLayer rejected every call | No |
| 1.3.1 | 2026-04-09 | bd-rovrv | RequirePermissionsLayer now bypasses permission checks for service-to-service calls (claims containing service.internal perm) | Service calls between modules (e.g. shipping-receiving → inventory) were rejected with 401 because service claims lack module-specific permissions like INVENTORY_READ | No |
| 1.3.0 | 2026-04-09 | bd-h7zrn | Add TieredRateLimiter with per-route configurable rate limit tiers, IP+tenant key isolation. Extend ClaimsLayer middleware to support AuthzGate integration. | SDK multi-tier rate limiting — single global limiter insufficient for verticals with auth endpoints needing different limits | No |
| 1.1.0 | 2026-04-01 | bd-x2k12 | Add `JwtVerifier::from_jwks_url()` async constructor with `Arc<RwLock<Vec<DecodingKey>>>` key storage, background refresh loop, and env var fallback. Add `SecurityError::JwksUnavailable` variant. Internal `KeyStore` enum splits static (PEM) and dynamic (JWKS) key paths. | Verticals need JWKS URL support for identity-auth integration without hardcoding public keys. | No |

## How to read this table

- **Version:** The version in the package file (`Cargo.toml` or `package.json`) after this change.
- **Date:** When the change was committed.
- **Bead:** The bead ID that tracked this work.
- **What Changed:** A concrete description of the change. Name specific endpoints, fields, events, or behaviors affected. Do not write "various improvements" or "minor fixes."
- **Why:** The reason the change was necessary. Reference the problem it solves or the requirement it fulfills.
- **Breaking?:** `No` if existing consumers are unaffected. `YES` if any consumer must change code to handle this version. If YES, include a brief migration note or reference a migration guide.

## Rules

- Add a new row for every version bump. One row per version.
- Do not edit old rows. If a previous change is reversed, add a new row explaining the reversal.
- The commit that bumps the version in the package file must also add the row here. Same commit.
- If the change is breaking (MAJOR version bump), the "Breaking?" column must describe what consumers need to change.
