# Security Audit Report — 2026-02-24 (Full Investigation)

## Executive Summary

Comprehensive security audit of the 7D-Solutions Platform. The platform has solid foundations — Argon2id password hashing, RS256 JWT with key rotation, PII redaction utilities, rate limiting, and tenant isolation testing. However, **three critical issues** require immediate attention: the AR module lacks JWT verification middleware entirely, a legacy authz stub is deployed across most services without doing real token validation, and six modules ship with `allow_origin(Any)` CORS.

---

## CRITICAL FINDINGS

### C1. AR Module: No JWT Verification Middleware (CRITICAL)

**Severity: CRITICAL**

The AR module (`modules/ar/src/main.rs`) does **not** wire up `optional_claims_mw` or `ClaimsLayer`. It only mounts the legacy `AuthzLayer::from_env()` stub, which **never validates tokens** (see C2 below).

Consequence: `RequirePermissionsLayer` on mutation routes will always return 401 (because no `VerifiedClaims` are ever injected into request extensions). All **read routes have zero authentication** — and use a hardcoded `"test-app"` string for tenant scoping across 79 TODO locations in 15 files.

**Impact:** Any network-reachable client can read all AR data (customers, invoices, subscriptions, disputes, refunds, charges, payment methods, aging reports, events) without authentication. Data is not tenant-scoped.

**Modules with `optional_claims_mw` properly wired (safe):** GL, inventory, payments, subscriptions, notifications, reporting, consolidation, timekeeping, treasury, AP, fixed-assets, maintenance, pdf-editor.

**Modules WITHOUT `optional_claims_mw` (vulnerable like AR):** AR, TTP, party.

**Fix:** Add `JwtVerifier::from_env_with_overlap()` and `optional_claims_mw` to AR, TTP, and party `main.rs`, matching the pattern used by GL and other secured modules.

### C2. Legacy AuthzLayer is a No-Op Stub (CRITICAL)

**Severity: CRITICAL**

`platform/security/src/authz.rs` line 141:
```rust
let status = AuthzStatus::Unauthenticated;
```

The comment says "Phase 35 will add token validation here." Every service mounts `AuthzLayer::from_env()`, but it **never validates any token**. It just marks every request as `Unauthenticated`. With `AUTHZ_STRICT` not set (the default), all requests pass through.

This layer provides **zero security value** and gives a false sense of protection. The real JWT validation is in `authz_middleware.rs` (`ClaimsLayer` + `RequirePermissionsLayer`), but that only works when `optional_claims_mw` is wired up in `main.rs`.

**Fix:** Either implement token validation in `AuthzLayer` or remove it entirely to prevent confusion. Ensure all services use `optional_claims_mw` from `authz_middleware.rs`.

### C3. Six Modules Ship with `allow_origin(Any)` CORS (HIGH)

**Severity: HIGH**

These modules allow requests from any origin:

| Module | File |
|--------|------|
| notifications | `modules/notifications/src/main.rs:129` |
| payments | `modules/payments/src/main.rs:164` |
| subscriptions | `modules/subscriptions/src/main.rs:133` |
| maintenance | `modules/maintenance/src/main.rs:196` |
| GL | `modules/gl/src/main.rs:234` |
| inventory | `modules/inventory/src/main.rs:176` |

These all use `tower_http::cors::Any` for origin, methods, AND headers. Combined with `allow_credentials(true)` (if set), this makes the services vulnerable to cross-origin attacks from any website.

**Properly configured (safe):** AR (localhost origins only), pdf-editor (configurable via `CORS_ORIGINS` env var), consolidation, timekeeping, TTP, treasury, integrations (all use specific origins or restrictive configs).

**Fix:** Replace `allow_origin(Any)` with explicit allowed origins in all six modules, or adopt the pdf-editor pattern of configurable `CORS_ORIGINS` env var.

---

## HIGH-SEVERITY FINDINGS

### H1. AR Module: 79 Hardcoded Tenant IDs (HIGH)

All AR route handlers use hardcoded `"test-app"` or `"default-tenant"` as `app_id` instead of extracting tenant identity from JWT claims. This means even if JWT auth were wired up, **all tenants would share the same data scope**.

79 TODO occurrences across 15 files. Every CRUD route is affected: customers, invoices, subscriptions, charges, refunds, disputes, payment methods, aging, usage, events, allocation, reconciliation, webhooks, and idempotency.

### H2. SQL Injection via Dynamic Table Names (MEDIUM-HIGH)

Several files construct SQL using `format!()` with table/column names:

| File | Risk | Input Source |
|------|------|-------------|
| `platform/projections/src/admin.rs:198` | **HIGH** — `req.projection_name` comes from HTTP request body | User input via API |
| `platform/projections/src/digest.rs:42,63` | MEDIUM — `table_name` param from internal callers | Internal callers only |
| `tools/projection-rebuild/src/main.rs` | LOW — CLI tool, not web-facing | CLI args |
| `modules/reporting/src/` | MEDIUM — dynamic table names in DELETE/SELECT | Mixed |
| `e2e-tests/tests/` | NONE — test-only | N/A |

The `admin.rs` endpoint is the highest risk because `req.projection_name` comes directly from an HTTP request body. An attacker could inject SQL like `"users; DROP TABLE credentials; --"`.

**Fix:** Add a strict allowlist check: validate table names against `^[a-z_]+$` regex and a known list of valid projection tables before interpolation.

---

## MEDIUM-SEVERITY FINDINGS

### M1. Service-to-Service Auth Uses Symmetric HMAC (MEDIUM)

`platform/security/src/service_auth.rs` uses HMAC-SHA256 with a shared `SERVICE_AUTH_SECRET` env var. All services that can read this secret can impersonate any other service. Consider migrating to asymmetric signing (like the JWT infrastructure) for better isolation.

### M2. Password Policy Parameters Not Documented (MEDIUM)

`PasswordPolicy` in `platform/identity-auth/src/auth/password.rs` uses configurable Argon2id parameters (`memory_kb`, `iterations`, `parallelism`) but the actual production values are set in service startup code, not in a central config. Ensure production uses OWASP-recommended minimums: memory ≥ 47 MiB, iterations ≥ 1, parallelism ≥ 1.

### M3. No `deny(unsafe_code)` Workspace Lint (MEDIUM)

No `#![deny(unsafe_code)]` is set at the workspace level. Currently no unsafe code exists in production paths, but there's no guardrail preventing future additions.

---

## LOW-SEVERITY FINDINGS

### L1. Hardcoded Secrets in Tests (LOW)

Test webhook secrets like `whsec_test_secret` appear in 24 locations across test code. All are within `#[cfg(test)]` blocks — no production risk, but consider using constants or test fixtures for consistency.

### L2. `.env` Contains Development RSA Private Key (LOW)

Root `.env` has a full RSA private key for local JWT signing. Properly gitignored, but should be rotated if ever leaked.

### L3. TLS Termination (INFORMATIONAL)

All Rust services bind plain HTTP on localhost. TLS is terminated at nginx (confirmed by `ssh_bootstrap.sh` opening port 443 with UFW). This is standard practice for containerized deployments behind a reverse proxy.

---

## POSITIVE FINDINGS (What's Working Well)

| Area | Assessment |
|------|-----------|
| **Password hashing** | Argon2id with configurable parameters, CPU-limited via semaphore |
| **JWT verification** | RS256, proper exp/iss/aud validation, zero-downtime key rotation support |
| **PII redaction** | `Redacted<T>` wrapper prevents accidental logging of sensitive fields |
| **Rate limiting** | IP-based rate limiting (200 req/min), per-email login limits, hash concurrency limiter |
| **Account lockout** | Failed login counting with temporary lock-until mechanism |
| **Webhook verification** | HMAC-SHA256 with constant-time comparison and replay protection (timestamp tolerance) |
| **Tenant isolation testing** | Comprehensive 12-assertion production isolation check script |
| **Secrets management** | Production secrets validator checks ownership, permissions, and placeholders |
| **Session management** | DB-backed seat leases with advisory locks for atomic enforcement |
| **Sensitive data in logs** | No instances of passwords/secrets/tokens logged in production code |
| **Password reset** | Token hash stored (not raw token), expiry enforced |

---

## Priority Action Items

| # | Priority | Finding | Action |
|---|----------|---------|--------|
| 1 | **CRITICAL** | C1: AR/TTP/party have no JWT middleware | Wire up `optional_claims_mw` in `main.rs` for AR, TTP, and party modules |
| 2 | **CRITICAL** | C2: Legacy `AuthzLayer` is a no-op | Remove or implement real validation; audit all `AuthzLayer::from_env()` callers |
| 3 | **HIGH** | C3: 6 modules use `allow_origin(Any)` | Replace with specific origins or configurable `CORS_ORIGINS` env var |
| 4 | **HIGH** | H1: 79 hardcoded tenant IDs in AR | Extract `tenant_id` from `VerifiedClaims` in all AR route handlers |
| 5 | **MEDIUM-HIGH** | H2: SQL injection in projections admin | Add table name allowlist validation before `format!()` interpolation |
| 6 | **MEDIUM** | M1: Symmetric service auth | Consider asymmetric signing for service-to-service tokens |
| 7 | **MEDIUM** | M3: No `deny(unsafe_code)` | Add `#![deny(unsafe_code)]` to workspace `Cargo.toml` |
| 8 | **CI** | Cargo audit not in pipeline | Add `cargo audit` to CI |
| 9 | **CI** | npm audit not in pipeline | Add `npm audit` to CI for TCP UI |
