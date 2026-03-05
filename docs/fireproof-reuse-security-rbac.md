# Fireproof ERP Reuse: Identity/RBAC Client + Security Infrastructure

**Investigator:** SageDesert
**Bead:** bd-1h5cg
**Date:** 2026-03-05

## Executive Summary

Fireproof's identity_auth/ and security/ modules are **client-side consumers** of the 7D Platform's identity-auth service. The platform already has superior implementations for JWT verification, RBAC enforcement, and rate limiting. Most Fireproof code should **stay in the vertical** with only two patterns worth extracting: the JWKS-based middleware pattern (for verticals that can't use PEM keys) and the structured security audit log.

**Overall verdict:** Mostly SKIP. Two ADAPT-PATTERN candidates. One EXTRACT candidate (audit_log).

---

## Module-by-Module Assessment

### 1. identity_auth/client.rs (211 LOC) — ADAPT-PATTERN

**What it does:** Typed HTTP client for the identity-auth service with retry logic (exponential backoff on 5xx/network errors), generic `get_json<T>()` and `post_json<B,T>()` methods, and a `ClientError` enum.

**Platform comparison:** Platform has no generic typed HTTP client pattern. Each module makes raw reqwest calls or uses service-specific clients.

**Assessment:** The retry pattern (exponential backoff on 5xx, immediate fail on 4xx, configurable max_retries) is solid and generic. However, this belongs in the **platform clients investigation** (bd-3ashx), not here. The pattern is not identity-specific — it's a reusable HTTP client template.

**Recommendation:** ADAPT-PATTERN — defer to bd-3ashx for the platform SDK client investigation. The retry logic pattern is the extractable piece, not this specific client.

### 2. identity_auth/context.rs (217 LOC) — SKIP

**What it does:** `RequestContext` Axum extractor built from `VerifiedClaims`. Provides `tenant_id`, `user_id`, `roles`, `scopes`, `actor_type`, plus convenience methods `has_role()` and `has_scope()`.

**Platform comparison:** Platform's `VerifiedClaims` (claims.rs:47-58) already carries `user_id: Uuid`, `tenant_id: Uuid`, `roles: Vec<String>`, `perms: Vec<String>`, `actor_type: ActorType`. It has typed UUIDs (superior to Fireproof's raw strings), typed `ActorType` enum (superior to Fireproof's string), and includes `issued_at`, `expires_at`, `token_id` fields that Fireproof lacks.

**Gap:** Fireproof's `RequestContext` adds `role_snapshot_id` and `session_id` fields for audit correlation. Platform's `VerifiedClaims` doesn't have these.

**Assessment:** Platform's `VerifiedClaims` is strictly more capable. The `role_snapshot_id` and `session_id` are Fireproof-specific audit fields that should stay in the vertical. Fireproof's string-based IDs are a step backward from platform's `Uuid` types.

**Recommendation:** SKIP. Platform already has this, and it's better.

### 3. identity_auth/jwks.rs (229 LOC) — ADAPT-PATTERN

**What it does:** JWKS cache with TTL-based expiry. Fetches public keys from identity-auth's `/.well-known/jwks.json` endpoint, caches them with RwLock, double-checked locking on refresh, prefetch at startup. Supports key rotation automatically (tries all keys).

**Platform comparison:** Platform's `JwtVerifier` (claims.rs:83-203) uses PEM keys loaded from environment variables (`JWT_PUBLIC_KEY`, `JWT_PUBLIC_KEY_PREV`). Supports zero-downtime rotation via a two-key overlap window. Does NOT support JWKS endpoint discovery.

**Assessment:** These are two different key distribution strategies:
- **Platform (PEM env vars):** Works for services that share the deployment environment. Simpler, no network dependency for key fetching.
- **Fireproof (JWKS endpoint):** Works for verticals that are deployed separately from identity-auth. More dynamic, handles key rotation without redeployment.

Both are legitimate. As Fireproof (and future verticals) are separate deployables, they need JWKS-based key discovery. The platform shouldn't adopt JWKS internally — its PEM approach is faster and has fewer moving parts. But the JWKS pattern should be documented as the **recommended approach for verticals**.

**Recommendation:** ADAPT-PATTERN — document JWKS cache as the reference implementation for vertical JWT verification. Don't extract into platform — it's a client pattern.

### 4. identity_auth/middleware.rs (274 LOC) — SKIP

**What it does:** `require_auth` Axum middleware. Extracts Bearer token, validates via JWKS cache, tries all keys (rotation support), inserts `VerifiedClaims` + `AuthToken` into extensions. Returns 401 with JSON body on failure.

**Platform comparison:** Platform's `ClaimsLayer` / `ClaimsMiddleware` (authz_middleware.rs:49-125) does the same thing but as a proper Tower Layer (composable, strict/permissive modes). `RequirePermissionsLayer` (authz_middleware.rs:181-260) adds per-route permission guards. The `optional_claims_mw` function (authz_middleware.rs:160-171) handles the "JWT not yet configured" dev scenario.

**Assessment:** Platform's implementation is architecturally superior:
- Tower Layer pattern vs raw `middleware::from_fn` — more composable
- Strict/permissive modes for flexible deployment
- Proper `RequirePermissionsLayer` separation

Fireproof's middleware is functionally equivalent but tightly coupled to its JWKS cache. Not extractable.

**Recommendation:** SKIP. Platform has better architecture here.

### 5. identity_auth/rbac.rs (648 LOC) — SKIP (gauge-specific)

**What it does:** `AuthzGate` with scope-based authorization. 21 gauge-specific scope constants. Legacy 8-permission-to-platform-scope mapping. `require()` and `require_role()` middleware factories. Security event logging on denial.

**Platform comparison:** Platform has:
- `permissions.rs` (329 LOC): Module-scoped permission constants (`ar.mutate`, `gl.post`, etc.) — 44 constants across 22 modules
- `rbac.rs` (316 LOC): Role-based operation authorization (Admin/Operator/Auditor for operational tasks)
- `RequirePermissionsLayer`: Per-route permission enforcement

**Overlap analysis:**

| Aspect | Fireproof | Platform |
|--------|-----------|----------|
| Scope constants | 21 gauge-specific (`gauges:read`, `calibrations:create`) | 44 module-scoped (`ar.mutate`, `gl.post`) |
| Naming convention | `entity:action` (colon) | `module.action` (dot) |
| Superuser | `admin:all` implies everything | Admin role has all permissions |
| Legacy compat | 8 legacy permission → scope mapping | None needed |
| Enforcement | `AuthzGate::require()` middleware | `RequirePermissionsLayer` Tower layer |
| Audit on denial | `security_event()` tracing log | `tracing::warn!()` |

**Assessment:** Fireproof's scope constants are entirely gauge-specific. The platform uses a different naming convention (`module.action` vs `entity:action`). The `AuthzGate` pattern duplicates `RequirePermissionsLayer` but with worse composability. The legacy mapping is Fireproof-only baggage.

The one interesting piece: Fireproof logs security events on denial via a dedicated `security_event()` function, while the platform just uses `tracing::warn!()`. See audit_log assessment below.

**Recommendation:** SKIP. Scopes should stay in verticals. Platform has the enforcement layer.

### 6. security/audit_log.rs (127 LOC) — EXTRACT

**What it does:** Structured security event logging. `SecurityOutcome` enum (Success/Denied/RateLimited). `security_event()` function that emits structured tracing events with: event_type, tenant_id, user_id, ip, request_id, outcome, metadata. Uses `target: "security_event"` for filtering.

**Platform comparison:** No equivalent. Platform's security module has:
- `tracing::warn!()` calls in authz_middleware.rs for denied requests
- No structured security event format
- No filterable target for security events
- No dedicated audit log abstraction

**Assessment:** This is a genuine gap. The platform should have a standardized way to emit security events that can be filtered, forwarded to SIEM systems, and audited. The Fireproof implementation is clean, minimal, and generic.

**Extraction plan:**
- Add `security_event.rs` to `platform/security/src/`
- Re-export from `platform/security/src/lib.rs`
- ~60 LOC of production code (the rest is tests)
- No Fireproof-specific dependencies
- Wire into existing authz_middleware.rs denial paths

**Recommendation:** EXTRACT into `platform/security/`. Direct lift with minimal changes.

### 7. security/csrf.rs (283 LOC) — SKIP

**What it does:** Double-submit cookie CSRF protection. `GET /api/csrf` generates a random token, sets `fp_csrf` cookie (SameSite=Strict, Secure, NOT httpOnly), returns token in body. Middleware validates `x-csrf-token` header matches cookie on POST/PUT/PATCH/DELETE. Uses constant-time comparison.

**Platform comparison:** No CSRF module in platform.

**Assessment:** CSRF protection is a frontend concern. The 7D Platform is backend-only — verticals build their own frontends. Each vertical's CSRF implementation will differ based on their cookie naming, SameSite policy, and frontend framework. The `fp_csrf` cookie name is Fireproof-specific.

Additionally, modern SPA frontends typically use token-based auth (Bearer tokens in headers), which is inherently CSRF-immune. CSRF only matters for cookie-based session auth.

**Recommendation:** SKIP. Frontend concern, vertical-specific. If we ever need it, the pattern is documented in Fireproof as a reference.

### 8. security/hibp.rs (132 LOC) — SKIP

**What it does:** Have I Been Pwned k-anonymity password check. SHA-1 hashes the password, sends first 5 chars to HIBP range API, matches suffixes locally.

**Platform comparison:** Platform's identity-auth already has password policy enforcement. The HIBP check is not in the platform crate.

**Assessment:** Password checks are the identity-auth service's responsibility, not a platform-wide concern. If HIBP integration is needed, it should be added to `platform/identity-auth/`, not `platform/security/`. But identity-auth already has password policy (password_policy.rs). The HIBP check is an enhancement to identity-auth, not a platform security primitive.

**Recommendation:** SKIP. Enhancement to identity-auth if needed, not platform/security.

### 9. security/rate_limit.rs (660 LOC) — SKIP

**What it does:** Token-bucket rate limiter keyed by IP+email (login), IP (logout), or IP+tenant (API). Three middleware functions for different endpoint categories. Lazy cleanup when map exceeds 10K entries.

**Platform comparison:** Platform's `ratelimit.rs` (461 LOC) has:
- Token bucket implementation (DashMap-based, more concurrent than Fireproof's Mutex)
- Tenant+path keyed limiting
- Normal/fallback tiers
- Prometheus metrics integration
- `WebhookRateLimiter` for IP-based webhook limiting

Platform's `middleware.rs` (169 LOC) has:
- `rate_limit_middleware` using ConnectInfo for IP extraction
- `timeout_middleware` for request timeouts

**Assessment:** Platform's rate limiter is more mature:

| Feature | Fireproof | Platform |
|---------|-----------|----------|
| Concurrency | `Mutex<HashMap>` | `DashMap` (lock-free) |
| Key strategy | IP+email, IP, IP+tenant | Tenant+path, IP (webhook) |
| Metrics | None | Prometheus `CounterVec` |
| Cleanup | Manual threshold (10K entries) | None needed (DashMap) |
| Middleware | Three separate middlewares | One generic middleware |
| Tiers | Login/logout/API | Normal/fallback |
| Retry-After | Yes (seconds until next token) | No |

The one thing Fireproof has that platform lacks: the `Retry-After` header in the 429 response. Platform's rate_limit_middleware returns a plain text "Rate limit exceeded\n" without Retry-After. This is a minor enhancement, not an extraction.

**Recommendation:** SKIP. Platform has a better implementation. Could file a small bead to add `Retry-After` header to platform's rate limit response.

### 10. error_registry.rs (1,159 LOC) — SKIP (gauge-specific)

**What it does:** `ApiError` struct implementing Axum's `IntoResponse`. Standard `{success, code, message, details}` JSON envelope. `status_for_code()` maps ~50 domain error codes to HTTP statuses. `GaugeOperation` field rejection rules. Convenience constructors (`bad_request()`, `not_found()`, `conflict()`).

**Platform comparison:** No centralized error type in platform. Each module defines its own error handling:
- Module handlers return `Result<impl IntoResponse, StatusCode>` or module-specific error types
- No standardized error JSON envelope

**Assessment:** The `ApiError` struct and convenience constructors are generic and useful. The `status_for_code()` registry and `GaugeOperation` field rejection are entirely gauge-specific.

However, creating a platform-wide error type is a significant architecture decision. Each module currently owns its error shape. Forcing a shared `ApiError` would create a dependency from every module on `platform/security` and impose a specific JSON envelope format. This is the kind of decision that should go through the design lock process, not be extracted from a vertical.

The generic pieces (~100 LOC): `ApiError` struct, `IntoResponse` impl, convenience constructors. The gauge-specific pieces (~1,059 LOC): error code registry, field rejection rules, test coverage.

**Recommendation:** SKIP for now. The pattern is documented as a reference. A platform-wide API error type is a design decision, not a code extraction.

---

## Security Gaps in the Platform

While comparing the two codebases, I identified these gaps:

### Gap 1: No structured security event logging
**Severity:** Medium
**Impact:** Security events (auth failures, RBAC denials, rate limiting) are logged as generic tracing events. No filterable target, no standardized schema. Makes SIEM integration harder.
**Fix:** Extract Fireproof's `audit_log.rs` (see assessment above).

### Gap 2: No Retry-After header on rate limit responses
**Severity:** Low
**Impact:** Clients don't know when to retry after being rate-limited.
**Fix:** Small enhancement to `platform/security/src/middleware.rs` — add `Retry-After` header.

### Gap 3: No JWKS endpoint discovery for verticals
**Severity:** Low (documentation gap, not code gap)
**Impact:** Verticals need to implement their own JWKS caching. Fireproof has a solid reference implementation.
**Fix:** Document Fireproof's `jwks.rs` as the reference pattern for vertical JWT verification. Consider a `platform-client-auth` crate if multiple verticals need this.

---

## Mapping to Manufacturing Roadmap

| Roadmap Phase | Relevance | Notes |
|--------------|-----------|-------|
| Phase 0 (Design Lock) | None | Security infrastructure is not a Phase 0 deliverable |
| Phase A (Inventory + BOM) | Low | BOM module will use existing `RequirePermissionsLayer` with `bom.mutate`/`bom.read` perms (already in permissions.rs) |
| Phase B (Production) | Low | Same — production perms would be added to permissions.rs |
| Phase C (Inspection) | Low | Same — `quality_inspection.mutate`/`.read` already exist |
| Phase D (ECO) | None | |
| Phase E (Maintenance) | None | Maintenance perms already exist |

Security infrastructure does not gate any manufacturing phase. The platform's existing auth/RBAC/rate-limiting stack is sufficient.

---

## Recommended Beads

### Bead 1: Extract security audit log to platform (estimated ~60 LOC)
- Lift `security_event()` and `SecurityOutcome` from Fireproof to `platform/security/src/security_event.rs`
- Wire into `authz_middleware.rs` denial paths
- Re-export from `platform/security/src/lib.rs`

### Bead 2: Add Retry-After header to rate limit response (estimated ~10 LOC)
- Modify `platform/security/src/middleware.rs:rate_limit_middleware` to include `Retry-After` header
- Update the rate limiter API to return remaining wait time

---

## Summary Table

| Fireproof Module | LOC | Recommendation | Reason |
|-----------------|-----|----------------|--------|
| identity_auth/client.rs | 211 | ADAPT-PATTERN | Retry logic pattern useful; defer to bd-3ashx (platform clients) |
| identity_auth/context.rs | 217 | SKIP | Platform `VerifiedClaims` is strictly better (typed UUIDs, ActorType enum) |
| identity_auth/jwks.rs | 229 | ADAPT-PATTERN | Document as reference for vertical JWT verification |
| identity_auth/middleware.rs | 274 | SKIP | Platform's Tower Layer pattern is architecturally superior |
| identity_auth/rbac.rs | 648 | SKIP | Gauge-specific scopes and legacy mappings; platform has `RequirePermissionsLayer` |
| security/audit_log.rs | 127 | EXTRACT | Genuine platform gap — structured security event logging |
| security/csrf.rs | 283 | SKIP | Frontend concern, vertical-specific |
| security/hibp.rs | 132 | SKIP | Identity-auth enhancement, not platform security |
| security/rate_limit.rs | 660 | SKIP | Platform has better implementation (DashMap, Prometheus metrics) |
| error_registry.rs | 1,159 | SKIP | ~95% gauge-specific; platform error type is a design decision |
| **Total** | **3,949** | **~60 LOC extractable** | |
