# Security Audit Report -- 7D Solutions Platform

**Date:** 2026-03-06
**Scope:** Full codebase (~40 Rust crates, infrastructure, CI/CD)
**Method:** 5 parallel deep-dive agents across Auth, Database, API, Infrastructure, and Crypto domains

---

## Executive Summary

The platform demonstrates **strong security fundamentals** for a first-customer aerospace/defense deployment. Rust's type system and the consistent use of sqlx parameterized queries eliminate entire vulnerability classes (SQL injection, buffer overflows). JWT handling, password hashing (Argon2id), and multi-tenant isolation are well-architected.

However, **3 critical and 7 high-severity findings** require attention before production hardening is complete.

---

## Findings by Severity

### CRITICAL (3)

| # | Finding | Location | Description |
|---|---------|----------|-------------|
| C1 | **Secrets committed to .env** | `.env` (lines 2-48) | JWT RSA private key, Tilled API keys, NATS token in plaintext. `.gitignore` excludes `.env` but file exists in git history. |
| C2 | **CORS wildcard fallback in PDF Editor** | `docker-compose.modules.yml:698` | `CORS_ORIGINS: ${PDF_EDITOR_CORS_ORIGINS:-*}` falls back to `*`, allowing any origin to access the API. |
| C3 | **No envelope validation on consume** | `platform/event-consumer/` | Envelope validation runs at publish time only. A compromised NATS client can publish forged events directly to JetStream, bypassing all guards. |

### HIGH (7)

| # | Finding | Location | Description |
|---|---------|----------|-------------|
| H1 | **NATS subject injection risk** | `modules/ar/src/events/publisher.rs:56-68` | Event type used directly in subject construction without rejecting wildcard characters (`*`, `>`). |
| H2 | **DLQ stores full sensitive payloads** | `platform/event-consumer/src/dlq.rs:44-50` | Failed events (including financial data, PII) written unredacted to dead letter queue. |
| H3 | **Database errors leaked in responses** | `modules/ar/src/idempotency.rs:76`, `modules/subscriptions/src/http.rs:73`, `platform/control-plane/src/routes.rs:96` | `e.to_string()` returned to clients, exposing table names, constraints, connection details. |
| H4 | **Hardcoded temp password** | `modules/customer-portal/src/http/admin.rs:72` | `"TempPassw0rd!temp"` hardcoded for portal invitations. All invited users share the same default password. |
| H5 | **Global IP rate limiter disabled** | `platform/identity-auth/src/main.rs:139-147` | tower_governor commented out due to axum 0.7 incompatibility. Per-endpoint keyed limiters still active but no global fallback. |
| H6 | **Missing Nginx security headers** | `nginx/gateway.conf` | No HSTS, X-Frame-Options, X-Content-Type-Options, or CSP headers configured. |
| H7 | **CORS config unverified in identity-auth** | `platform/identity-auth/` | No CorsLayer or CORS_ORIGINS handling found. May rely on gateway, but needs explicit verification. |

### MEDIUM (12)

| # | Finding | Location | Description |
|---|---------|----------|-------------|
| M1 | **No PostgreSQL Row-Level Security** | All migrations | Tenant isolation relies solely on application WHERE clauses. No database-level RLS policies as defense-in-depth. |
| M2 | **Weak RNG for refresh tokens** | `modules/customer-portal/src/auth.rs:74` | Uses `thread_rng()` instead of `OsRng` for refresh token generation. |
| M3 | **No `deny_unknown_fields` on DTOs** | `platform/doc-mgmt/src/models.rs`, `platform/identity-auth/src/auth/handlers.rs` | Request structs accept and silently ignore extra JSON fields. |
| M4 | **No request body size limits** | All services | No explicit `DefaultBodyLimit` layer. Relies on axum's default 2MB, undocumented. |
| M5 | **Fixed DB connection pool sizes** | `modules/treasury/src/db/mod.rs:15`, multiple others | Hardcoded `max_connections(10)` with no env configuration, timeout, or circuit breaker. |
| M6 | **Webhook secrets silently optional** | `modules/payments/src/config.rs:147` | `.ok()` swallows missing TILLED_WEBHOOK_SECRET; service may accept unsigned webhooks. |
| M7 | **No NATS subject-level ACLs** | `infra/nats/nats.conf:10-13` | Single `platform` user for all modules. Compromised module can publish to any subject. |
| M8 | **No NATS TLS encryption** | Infrastructure | Plaintext protocol between services and NATS. Docker network isolation is only protection. |
| M9 | **Inconsistent monetary amount validation** | AR validates negative amounts; subscriptions, payments do not consistently | Negative or zero amounts possible in some modules. |
| M10 | **Unvalidated serde_json::Value fields** | `platform/doc-mgmt/src/models.rs:41,46` | Arbitrary JSON accepted without depth/size limits. |
| M11 | **CI workflow permissions not restricted** | `.github/workflows/` | No `permissions:` directive; GITHUB_TOKEN has default broad access. |
| M12 | **Production Postgres TLS mode unclear** | `docs/POSTGRES-TLS.md:100` | Docs recommend `verify-full` for prod but status unconfirmed. Dev uses `require` (no CA validation). |

### LOW (5)

| # | Finding | Location | Description |
|---|---------|----------|-------------|
| L1 | **No string length validation on inputs** | `platform/doc-mgmt/src/handlers.rs:52-57` | Only `is_empty()` checks, no max length. |
| L2 | **Inconsistent error response format** | Various modules | doc-mgmt, AR, subscriptions, control-plane all use different error structures. |
| L3 | **No timing-safe password comparison test** | `platform/identity-auth/tests/` | Argon2 crate is constant-time by design, but no explicit test validates this. |
| L4 | **Dockerfile uses `rust:latest`** | `platform/identity-auth/deploy/Dockerfile:1` | Floating tag breaks reproducibility. |
| L5 | **cargo audit not yet active in CI** | `.github/workflows/hardening.yml:36-43` | Commented out, planned for future bead. |

---

## What's Working Well

These areas showed strong security posture across all audits:

- **Zero SQL injection** -- 100% parameterized queries via sqlx across all modules
- **Zero unsafe blocks** -- entire codebase uses safe Rust abstractions
- **JWT architecture** -- RS256 pinned, proper validation (exp/iss/aud), zero-downtime key rotation, JWKS endpoint
- **Password hashing** -- Argon2id with OsRng salts, configurable parameters
- **Session management** -- 256-bit random tokens, hash-only storage, single-use enforcement, replay detection, concurrent login limits via advisory locks
- **RBAC** -- exhaustive permission matrix, two-layer middleware (claims + permissions), proper audit trail
- **Multi-tenant isolation** -- enforced at schema (UNIQUE constraints), query (WHERE tenant_id), and API level (tenant_id from JWT, never request body)
- **Password reset** -- user enumeration protection (constant 200), multi-dimension rate limiting, token hash-only storage, atomic single-use claim
- **Webhook verification** -- HMAC-SHA256 with constant-time comparison, replay protection, rotation support
- **PII redaction** -- `Redacted<T>` wrapper type prevents Display/Debug leakage, structured logging
- **Event envelope validation** -- comprehensive field validation, merchant context enforcement for financial events
- **Container hardening** -- non-root user, cap_drop ALL, read-only filesystem, localhost-only port binding
- **Nginx rate limiting** -- per-endpoint policies (5/min auth, 120/min API), tenant+IP keying
- **Production secrets** -- Docker secrets with file-based mounts, root-owned mode 0600
- **Template rendering** -- safe string replacement, no expression language or code execution
- **Fail-closed patterns** -- entitlement checks, tenant lifecycle gating deny on error

---

## Remediation Priority

### Immediate (before production)

1. **Rotate all secrets** exposed in `.env` -- JWT RSA keys, Tilled API keys, NATS token (C1)
2. **Remove .env from git history** -- `git filter-repo` or BFG Repo Cleaner (C1)
3. **Fix CORS wildcard fallback** -- remove `:-*` default in PDF editor compose config (C2)
4. **Add envelope validation on consume** -- call `validate_envelope_fields()` in event_consumer before routing (C3)
5. **Sanitize all database errors** in HTTP responses -- log full error server-side, return generic message (H3)
6. **Add Nginx security headers** -- HSTS, X-Frame-Options, X-Content-Type-Options, CSP (H6)

### Short-term (week 1-2)

7. **Add NATS subject allowlist validation** -- reject wildcard chars in event types before publish (H1)
8. **Implement DLQ payload redaction** for financial/PII fields (H2)
9. **Generate random temp passwords** per portal invitation instead of hardcoded (H4)
10. **Fix customer-portal RNG** -- change `thread_rng()` to `OsRng` for refresh tokens (M2)
11. **Re-enable or replace global rate limiter** -- test tower_governor 0.10+ or implement DashMap alternative (H5)
12. **Verify identity-auth CORS configuration** against platform standards (H7)
13. **Make webhook secrets required** -- change `.ok()` to `.expect()` or `?` (M6)

### Medium-term (phase 66+)

14. **Add `deny_unknown_fields`** to all request DTOs (M3)
15. **Add `DefaultBodyLimit`** layer to all services (M4)
16. **Make DB pool sizes configurable** via env vars, add timeouts (M5)
17. **Implement NATS subject-level ACLs** per module (M7)
18. **Enable NATS TLS** for message encryption in transit (M8)
19. **Standardize monetary validation** -- consistent negative/zero amount rejection (M9)
20. **Activate cargo audit** in CI pipeline (L5)
21. **Add CI workflow `permissions:` directives** (M11)
22. **Implement PostgreSQL RLS policies** as defense-in-depth (M1)
23. **Pin Dockerfile base images** to specific versions (L4)
24. **Confirm production Postgres uses `sslmode=verify-full`** (M12)

---

## Tested Attack Vectors (Code Review)

| Vector | Result |
|--------|--------|
| SQL injection | PROTECTED -- 100% parameterized queries |
| JWT alg:none | PROTECTED -- algorithm pinned to RS256 |
| Expired token acceptance | PROTECTED -- validate_exp enforced |
| Cross-service token reuse | PROTECTED -- issuer/audience validation |
| Password timing attacks | PROTECTED -- argon2 constant-time verify |
| Refresh token replay | PROTECTED -- single-use + revocation |
| Session fixation | PROTECTED -- new token on every refresh |
| User enumeration (password reset) | PROTECTED -- constant response |
| Cross-tenant data access | PROTECTED -- schema + query + API isolation |
| Privilege escalation | PROTECTED -- exhaustive RBAC match |
| Mass assignment (tenant_id, role) | PROTECTED -- extracted from JWT only |
| SSRF | PROTECTED -- no user-supplied URLs |
| Path traversal | PROTECTED -- UUID-based operations, no filesystem paths |
| Template injection | PROTECTED -- safe string replacement only |
| Buffer overflow | PROTECTED -- safe Rust, no unsafe blocks |

---

## Methodology

Five parallel analysis agents each performed independent deep-dive audits:

1. **Auth & Identity** -- JWT, passwords, RBAC, CORS, sessions, multi-tenancy
2. **SQL & Database** -- injection patterns, credentials, migrations, outbox, data exposure
3. **Input Validation & API** -- validation gaps, rate limiting, serialization, error handling, SSRF
4. **Infrastructure & NATS** -- event bus, Docker, Nginx, CI/CD, dependencies, secrets
5. **Crypto & Data Protection** -- algorithms, RNG, secrets in code, PII, logging, compliance

Each agent searched the full codebase (~40 crates) using pattern matching, file-by-file review of security-critical paths, and configuration analysis.
