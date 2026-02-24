# Identity Auth Module — Product Vision & Technical Specification

**7D Solutions Platform**
**Status:** Vision Document + Technical Specification (v1.3.x)
**Last updated:** 2026-02-24

---

## Revision History

| Rev | Date | Changed By | Summary |
|-----|------|-----------|---------|
| 1.0 | 2026-02-24 | PurpleCliff (bd-15s1) | Initial vision doc — based on shipped v1.3.1 source, migrations, REVISIONS.md, and all handlers |
| 1.1 | 2026-02-24 | PurpleCliff (bd-1twb) | Review fixes: HMAC-SHA256→SHA-256 for refresh tokens (4 locations — code uses plain sha2::Sha256, no hmac crate), added `granted_at` to role_permissions key fields, corrected `pg_try_advisory_xact_lock`→`pg_advisory_xact_lock` (code uses blocking variant) |

---

## The Business Problem

Every multi-tenant SaaS platform needs a single, correct answer to the question: *"Who is this user, and what are they allowed to do?"*

Without a dedicated auth service, each module would implement its own token handling, password storage, session management, and access control — leading to inconsistent security posture, duplicated argon2 tuning mistakes, different token formats that don't compose, and no central revocation path when a user's account is compromised.

The problems the platform would face without a centralized auth service:

- **Credential sprawl:** Each module storing its own passwords with potentially inconsistent hashing.
- **No tenant lifecycle enforcement:** Suspended tenants could still authenticate against individual modules.
- **No concurrent session control:** A tenant on a 5-seat plan could have 50 active sessions with no enforcement.
- **No standard JWT:** Modules would need to agree on a token format out-of-band, or verify each other's tokens with no shared issuer.
- **No safe rotation:** Rotating a JWT signing key would immediately invalidate all in-flight sessions with no overlap window.

---

## What the Module Does

Identity Auth (`auth-rs`) is the **sole authentication and token issuing authority** for the 7D Solutions Platform. It is the only service that stores credential hashes, issues JWT access tokens, manages refresh token rotation, and enforces tenant-level concurrent session limits.

It answers six questions:

1. **Is this credential valid?** — Argon2id password verification against a stored hash, with lockout after repeated failures.
2. **What is this user allowed to do?** — RBAC role and permission resolution, embedded as claims in the JWT.
3. **Is this tenant allowed to authenticate?** — Lifecycle status gating (active/trial/past_due/suspended/deleted) fetched from the control-plane tenant-registry.
4. **How many concurrent sessions does this tenant have?** — DB-backed session lease enforcement against the tenant's entitlement limit.
5. **Is this refresh token still valid?** — Token hash lookup with replay detection and lease rotation on use.
6. **How do I reset my password?** — Single-use, time-limited reset token flow with NATS event carrying the raw token for delivery, and hard session revocation on completion.

---

## Who Uses This

### Platform Services (JWT Verifiers)
Every other platform module verifies JWTs issued by identity-auth. They consume the public key from `/.well-known/jwks.json` and validate RS256 tokens using the `security` crate's `JwtVerifier`. They never call identity-auth at runtime — they verify tokens independently using the public key.

### Frontend Applications / API Clients
Applications (TCP UI, TrashTech portal, third-party clients) call:
- `POST /api/auth/register` — create credentials for a new user
- `POST /api/auth/login` — authenticate and receive access + refresh tokens
- `POST /api/auth/refresh` — rotate tokens before expiry
- `POST /api/auth/logout` — revoke a session
- `POST /api/auth/forgot-password` — initiate password reset
- `POST /api/auth/reset-password` — complete password reset with token

### Platform Operators / Administration
RBAC data (permissions, roles, user-role bindings) is managed directly via DB — there are no HTTP management endpoints for RBAC in v1. This is intentional: RBAC mutation is an admin concern, managed via internal tooling or provisioning scripts.

### Notification Infrastructure
`auth.events.password_reset_requested` carries the raw reset token on the NATS bus. A downstream notification service (e.g., the platform Notifications module) subscribes and delivers the token to the user via email or other channel. **NATS must use TLS** when this event is active — the raw token is sensitive and must not travel over plaintext transport.

### Tenant Registry / Control Plane
Identity-auth calls the tenant-registry at login time (and optionally at refresh time) to fetch:
- `concurrent_user_limit` — the maximum number of active sessions the tenant's plan allows
- `status` — the tenant lifecycle state (trial, active, past_due, suspended, deleted)

---

## Design Principles

### Fail-Closed Security
When identity-auth cannot determine a security policy decision — entitlement unavailable, tenant status unknown — it denies the operation. There is no fallback to "probably allow." The only exception is the stale-cache grace period (5 minutes) during a tenant-registry outage, to prevent cascading failures from locking out healthy tenants.

### Credential Hashes Only, Never Plaintext
Passwords are stored as argon2id hashes. Refresh tokens are stored as SHA-256 hashes. Reset tokens are stored as SHA-256 hashes. Raw values are never written to the database or emitted in logs.

### DB-Backed Session Leases for Horizontal Scaling
Session seat limits are enforced via `session_leases` in PostgreSQL with an advisory transaction lock, not in-memory counters. This means enforcement is correct across multiple running instances of identity-auth. An in-memory semaphore would be incorrect under horizontal scaling.

### RBAC Claims are Embedded at Token Issuance
JWT access tokens carry `roles` and `perms` arrays resolved from the RBAC DB at login and refresh. Downstream services do not need to call identity-auth to check permissions — the token is self-contained and signed. This trades a small latency at login for zero runtime auth overhead at the module layer.

### Zero-Downtime Key Rotation
JWT signing keys can be rotated without forcing a logout of all users. During the overlap window, identity-auth accepts tokens signed by either the current or the previous key. The JWKS endpoint serves both public keys so remote verifiers fetch the new key ID before old tokens expire. Rotation is purely env-var-driven.

### Rate Limiting is Keyed, Not Global
Rate limits are applied per-email (login, register), per-email+IP (forgot-password), and per-IP (reset-password, global safety net). Global rate limits would block legitimate users during normal load spikes; keyed limits target abusive patterns at specific accounts or IPs.

### Hash Concurrency is Bounded
Argon2id is CPU-intensive by design. Without concurrency control, a burst of login requests could saturate all available CPU with hash operations. The `HashConcurrencyLimiter` semaphore caps simultaneous argon2 calls and returns `503 Service Unavailable` when the queue fills, protecting the service from CPU starvation.

---

## MVP Scope (v1.x)

### In Scope
- Credential registration with argon2id password hashing
- Login with password verification, lockout, RBAC claims resolution, seat limit enforcement
- Refresh token rotation with replay detection
- Logout with lease revocation
- JWT issuance: RS256, configurable TTL, RBAC claims (`roles`, `perms`, `actor_type`, `ver`)
- JWKS endpoint for remote key discovery
- Zero-downtime JWT key rotation (prev_key overlap window)
- RBAC data model: permissions (global), roles (tenant-scoped), user-role bindings with soft revocation
- `effective_permissions_for_user` query for JWT claims embedding
- DB-backed session leases for concurrent seat enforcement
- Tenant lifecycle gating (trial/active/past_due/suspended/deleted) via tenant-registry
- Entitlement enforcement (concurrent_user_limit) via tenant-registry with TTL cache and grace period
- Password policy enforcement (min 12 chars, upper/lower/digit required, denylist)
- Rate limiting keyed per-email, per-IP
- Hash concurrency limiter
- Login lockout (configurable threshold and lock duration)
- Password reset: forgot-password (always 200, NATS event with raw token) + reset-password (claim token, update hash, hard-revoke sessions)
- Health endpoints: `/healthz`, `/health/live`, `/health/ready`, `/api/ready`
- Prometheus metrics endpoint: `/metrics`
- Structured JSON tracing with trace ID propagation
- JetStream stream provisioning on startup
- JSON Schema event validation

### Explicitly Out of Scope for v1
- HTTP API for RBAC management (create/delete roles, assign permissions) — internal/DB only
- Multi-factor authentication (TOTP, WebAuthn, SMS)
- Social / OAuth2 provider login (Google, GitHub, etc.)
- API key management via HTTP (present as an internal concept only)
- Audit log queries (logs are emitted to NATS and structured tracing; no query API)
- User self-service profile management
- Email delivery (raw token goes on NATS; delivery is a downstream concern)
- Session listing or selective session revocation by the user

---

## Technology Summary

| Layer | Technology | Notes |
|-------|-----------|-------|
| Language | Rust | Platform standard |
| HTTP framework | Axum 0.8 | Async, tower-based |
| Database | PostgreSQL | SQLx for queries and migrations; dedicated DB |
| Event bus | NATS (async-nats 0.33) | JetStream streams; schema-validated events |
| JWT | jsonwebtoken 9 | RS256 (RSA 2048-bit), JWKS served |
| Password hashing | argon2 0.5 | argon2id, configurable memory/iterations/parallelism |
| Token hashing | sha2 0.10 | SHA-256 hex for refresh and reset tokens |
| Rate limiting | governor 0.6 + dashmap 6 | Keyed per-email/IP in-process limiters |
| Metrics | prometheus 0.13 | Custom registry; `/metrics` endpoint |
| Tracing | tracing + tracing-subscriber | JSON output, env-filter, trace-ID middleware |
| Crate | `auth-rs` | Located at `platform/identity-auth/` |

---

## Structural Decisions (The "Walls")

### 1. One service owns all credentials
No other module stores passwords or issues JWTs. Identity-auth is the single authentication authority. Cross-service token verification uses the JWKS public key — no module calls identity-auth's API at runtime to validate tokens.

### 2. Refresh tokens are hashed in the DB; the raw token is never stored
`refresh_tokens.token_hash` stores SHA-256 of the raw bearer token. The raw token is returned to the client on login/refresh but never persisted. This means a database compromise does not yield usable refresh tokens.

### 3. Session leases are the source of truth for concurrent seat counting
The `session_leases` table (one row per active refresh token) is the authoritative count of active sessions per tenant. The count is taken inside a PostgreSQL advisory transaction lock (`pg_advisory_xact_lock`) to prevent race conditions under concurrent logins. An in-memory counter would be wrong under horizontal scaling.

### 4. RBAC is embedded in tokens, not checked per-request
Roles and effective permissions are resolved from the DB at login and token refresh, then embedded as `roles`/`perms` claims in the signed JWT. Downstream services validate the JWT and read claims directly. The trade-off: permissions changes don't take effect until the next token refresh (configurable TTL). The benefit: zero runtime latency for authorization decisions in every other module.

### 5. Tenant status gating is fail-closed with a 5-minute grace period
If the tenant-registry is unreachable and no cached value exists, logins are denied. If a cached value exists but has passed its TTL (default 60s), the stale value is used for up to 5 additional minutes (grace period) before failing closed. This prevents a tenant-registry outage from cascading to a full auth outage for all tenants that have recently authenticated.

### 6. Password reset tokens are single-use and hash-only
`password_reset_tokens.token_hash` stores SHA-256 of the raw token. `claim_reset_token()` uses a single `UPDATE ... SET used_at = NOW() WHERE used_at IS NULL AND expires_at > NOW() RETURNING user_id` — atomic claim prevents concurrent double-use. The raw token is dispatched on NATS and never stored.

### 7. Session hard-revocation on password reset
On `POST /api/auth/reset-password`, all `session_leases` and `refresh_tokens` rows for the user are hard-deleted (not soft-revoked). Any failure to delete returns 500 — there is no best-effort behavior. A successful password reset guarantees that no prior sessions remain active.

### 8. No tower_governor global IP limiter in v1 (intentionally disabled)
The global per-IP token bucket limiter (`tower_governor`) is disabled due to an axum 0.7/0.8 compatibility issue. Keyed per-email and per-IP limits on individual endpoints provide targeted rate limiting in the interim. This is a known gap, tracked, not an oversight.

### 9. RBAC has no HTTP management API
Roles and permissions are managed directly in the DB. Exposing a management API introduces authorization bootstrapping complexity (who can manage the roles that define what you can do?). The current design defers that complexity to a future admin plane bead.

---

## Domain Authority

Identity Auth is the **source of truth** for:

| Domain Entity | Authority |
|---------------|-----------|
| **Credentials** | Email + argon2id password hash per tenant+user. Lockout state (failed_login_count, lock_until) owned here. |
| **Refresh Tokens** | Hashed bearer tokens with expiry and revocation state. Session continuity is token-based. |
| **Session Leases** | One row per active refresh token. Authoritative for concurrent session count per tenant. |
| **RBAC — Permissions** | Global permission strings (e.g., `gl.journal.create`). Defined at platform provisioning time. |
| **RBAC — Roles** | Tenant-scoped named role definitions. System roles marked `is_system = true` cannot be mutated. |
| **RBAC — User-Role Bindings** | Which roles a user holds in a tenant, with grant/revoke lifecycle. |
| **Password Reset Tokens** | SHA-256 hashes of single-use reset tokens with TTL. |
| **JWT Signing Keys** | RSA private key (never leaves this service). Public key served via JWKS. |

Identity Auth is **NOT** authoritative for:
- Tenant provisioning, entitlements, or billing status (control-plane / tenant-registry owns this)
- User profile data beyond email (name, avatar, preferences — owned by a user-profile service if/when built)
- Audit trail storage (audit module subscribes to NATS events)
- Email delivery (Notifications module subscribes and dispatches)

---

## Data Ownership

### Tables Owned by Identity Auth

All tables with `tenant_id` use it for multi-tenant isolation. Every query **MUST** filter by `tenant_id` where it is present.

| Table | Purpose | Key Fields |
|-------|---------|------------|
| **credentials** | Email/password storage with lockout state | `id`, `tenant_id`, `user_id`, `email`, `password_hash`, `is_active`, `failed_login_count`, `last_failed_login_at`, `lock_until` |
| **refresh_tokens** | Hashed refresh tokens with revocation | `id`, `tenant_id`, `user_id`, `token_hash` (SHA-256, UNIQUE), `expires_at`, `revoked_at`, `last_used_at` |
| **session_leases** | DB-backed concurrent seat tracking | `lease_id`, `tenant_id`, `user_id`, `session_id` (FK → refresh_tokens), `issued_at`, `last_seen_at`, `revoked_at` |
| **permissions** | Global permission string definitions | `id`, `key` (UNIQUE, e.g. `gl.journal.create`), `description` |
| **roles** | Tenant-scoped role definitions | `id`, `tenant_id`, `name`, `description`, `is_system` |
| **role_permissions** | Junction: which permissions a role grants | `role_id`, `permission_id` (composite PK), `granted_at` |
| **user_role_bindings** | User-to-role bindings with soft revocation | `id`, `tenant_id`, `user_id`, `role_id`, `granted_by`, `granted_at`, `revoked_at` |
| **password_reset_tokens** | Single-use reset token hashes | `id`, `user_id`, `token_hash` (SHA-256 hex, indexed), `expires_at`, `used_at` |

### Indexes of Note

- `idx_credentials_tenant` — fast login lookup by `tenant_id`
- `idx_credentials_lock_until` — fast lockout-state queries
- `idx_refresh_user` — refresh lookup by `(tenant_id, user_id)`
- `idx_refresh_expiry` — token cleanup scans
- `idx_session_leases_tenant_active` — fast active seat count: `(tenant_id, last_seen_at) WHERE revoked_at IS NULL`
- `idx_session_leases_session` — lease lookup by refresh token id (for rotate/revoke)
- `idx_prt_token_hash` — reset token claim by hash

### Data NOT Owned by Identity Auth

Identity Auth **MUST NOT** store:
- Tenant billing data, subscription tiers, or entitlement limits (tenant-registry owns this; auth-rs reads via HTTP)
- User profile data beyond email (name, avatar, timezone)
- Audit trails of authorization decisions (separate audit module)
- Email addresses or notification preferences beyond what's needed for `forgot-password`

---

## Events Produced

All events use the platform `EventEnvelope` and are published to NATS via `EventPublisher`.

| Event | Subject | Trigger | Key Payload Fields |
|-------|---------|---------|-------------------|
| `auth.user.registered` | `auth.events.user.registered` | Successful credential registration | `user_id`, `email` |
| `auth.user.logged_in` | `auth.events.user.logged_in` | Successful login | `user_id` |
| `auth.token.refreshed` | `auth.events.token.refreshed` | Successful token refresh | `user_id` |
| `auth.password_reset_requested` | `auth.events.password_reset_requested` | Successful `forgot-password` when user found | `user_id`, `email`, `raw_token` (**sensitive — TLS bus required**), `expires_at`, `correlation_id` |
| `auth.events.password_reset_completed` | `auth.events.password_reset_completed` | Successful `reset-password` | `user_id`, `correlation_id` |

**Note:** A logout event schema file (`auth.user.logged_out.v1.json`) exists in `src/events/schemas/` but the event is not currently published by the logout handler. It is reserved for a future implementation.

---

## Events Consumed

| Event | Source | Action |
|-------|--------|--------|
| *(None in v1)* | — | Identity-auth is event-producing only. It integrates with tenant-registry via synchronous HTTP, not NATS. |

---

## Integration Points

### Tenant Registry (Synchronous HTTP, Read-Only)
`TenantRegistryClient` makes HTTP GET calls to:
- `GET {TENANT_REGISTRY_URL}/api/tenants/{tenant_id}/entitlements` — fetches `concurrent_user_limit`
- `GET {TENANT_REGISTRY_URL}/api/tenants/{tenant_id}/status` — fetches lifecycle status

Both are cached in-process (DashMap) with a configurable TTL (default 60s) and a 5-minute grace period for outage tolerance. If `TENANT_REGISTRY_URL` is not set, a static `MAX_CONCURRENT_SESSIONS` env var is used as the seat limit fallback, and tenant status gating is disabled.

### Security Crate (Compile-Time Dependency)
The platform `security` crate provides `JwtVerifier`, `VerifiedClaims`, and `ActorType` for consumers. Identity-auth writes tokens in the format that `security` verifies. The JWT claim schema is defined in `src/auth/jwt.rs::AccessClaims`.

### Notifications Module (Event-Driven, One-Way)
The Notifications module (or any downstream subscriber) receives `auth.events.password_reset_requested` and is responsible for delivering the raw token to the user via email or other channel. **Identity-auth never delivers emails directly.**

### RBAC-Aware Consumers (Token Claims, No Runtime Call)
All platform modules that perform authorization read `roles` and `perms` from the verified JWT. There is no runtime call to identity-auth for permission checks. RBAC changes (adding/removing role bindings) take effect on the next token refresh.

---

## Invariants

1. **No plaintext credential storage.** Passwords, refresh tokens, and reset tokens are hashed before any DB write. Raw values are never logged.
2. **Seat limit is atomically enforced.** Active seat count is read and a new lease is created within a single advisory-locked transaction. Concurrent logins cannot both succeed when the last seat is taken.
3. **Token replay is detected.** Using a revoked refresh token logs a structured security warning. The attempt is rejected with 401.
4. **Password reset is single-use.** `claim_reset_token()` atomically sets `used_at` in a single `UPDATE...RETURNING`. No token can be claimed twice regardless of concurrency.
5. **Reset revokes all sessions.** A successful password reset hard-deletes all `session_leases` and `refresh_tokens` for the user. Any failure to delete returns 500.
6. **Tenant lifecycle is enforced.** Suspended/deleted tenants cannot authenticate. Past-due tenants cannot start new sessions (but may refresh existing ones during grace).
7. **Fail-closed on unavailable policy.** If entitlements or tenant status cannot be determined and no usable cache exists, auth is denied.
8. **RBAC is per-tenant.** Roles are scoped to `tenant_id`. A user's roles in one tenant do not transfer to another.
9. **Lockout is enforced before password hash.** Account lockout is checked before acquiring the hash concurrency semaphore, avoiding wasted CPU on locked accounts.
10. **Schema-validated events.** Every NATS event is validated against its JSON Schema before publish. Invalid events are logged but the operation is not rolled back.

---

## API Surface (Summary)

Full OpenAPI contract: `contracts/auth/auth-v1.yaml`

### Authentication
- `POST /api/auth/register` — Register credentials for a user. Body: `{tenant_id, user_id, email, password}`. Returns `{ok: true}`. 409 if credentials exist.
- `POST /api/auth/login` — Authenticate. Body: `{tenant_id, email, password}`. Returns `{token_type, access_token, expires_in_seconds, refresh_token}`. 401 invalid credentials, 423 locked, 429 rate limited / seat limit, 403 inactive / tenant denied.
- `POST /api/auth/refresh` — Rotate tokens. Body: `{tenant_id, refresh_token}`. Returns same as login. 401 invalid/revoked/expired.
- `POST /api/auth/logout` — Revoke refresh token. Body: `{tenant_id, refresh_token}`. Returns `{ok: true}`.

### Password Reset
- `POST /api/auth/forgot-password` — Initiate reset. Body: `{email}`. Always returns 200 `{message: "..."}` (never reveals user existence). Rate limited per-email and per-IP.
- `POST /api/auth/reset-password` — Complete reset. Body: `{token, new_password}`. Returns `{ok: true}`. 400 on invalid/expired token or weak password.

### Discovery
- `GET /.well-known/jwks.json` — JWKS endpoint. Returns current signing key (and previous key during rotation overlap).

### Operational
- `GET /healthz` — Liveness probe (simple 200).
- `GET /health/live` — Liveness (legacy path).
- `GET /health/ready` — Readiness (legacy path).
- `GET /api/ready` — Standardized readiness JSON (checks DB + NATS connectivity).
- `GET /metrics` — Prometheus metrics.

---

## Configuration Reference

All configuration via environment variables:

| Variable | Default | Purpose |
|----------|---------|---------|
| `DATABASE_URL` | — (required) | PostgreSQL connection string |
| `NATS_URL` | — (required) | NATS server URL |
| `HOST` | `0.0.0.0` | Bind address |
| `PORT` | `8080` | HTTP port |
| `JWT_PRIVATE_KEY_PEM` | — (required) | RSA private key (PKCS8 PEM) |
| `JWT_PUBLIC_KEY_PEM` | — (required) | RSA public key (PEM) |
| `JWT_KID` | `auth-key-1` | Key ID in JWKS |
| `JWT_PREV_PUBLIC_KEY_PEM` | — | Previous public key for rotation overlap |
| `JWT_PREV_KID` | — | Previous key ID |
| `ACCESS_TOKEN_TTL_MINUTES` | `15` | Access token lifetime |
| `REFRESH_TOKEN_TTL_DAYS` | `14` | Refresh token lifetime |
| `ARGON_MEMORY_KB` | `65536` | Argon2id memory parameter |
| `ARGON_ITERATIONS` | `3` | Argon2id iteration count |
| `ARGON_PARALLELISM` | `1` | Argon2id parallelism |
| `LOCKOUT_THRESHOLD` | `10` | Failed attempts before lockout |
| `LOCKOUT_MINUTES` | `15` | Lockout duration |
| `LOGIN_PER_MIN_PER_EMAIL` | `5` | Login rate limit |
| `REGISTER_PER_MIN_PER_EMAIL` | `5` | Register rate limit |
| `REFRESH_PER_MIN_PER_TOKEN` | `20` | Refresh rate limit per token |
| `MAX_CONCURRENT_HASHES` | `50` | Semaphore cap for argon2 concurrency |
| `HASH_ACQUIRE_TIMEOUT_MS` | `5000` | Wait timeout for hash semaphore |
| `MAX_CONCURRENT_SESSIONS` | `5` | Seat limit fallback (used without tenant-registry) |
| `TENANT_REGISTRY_URL` | — | Control-plane URL; enables live entitlement/status checks |
| `ENTITLEMENT_TTL_SECS` | `60` | Entitlement cache TTL |
| `PASSWORD_RESET_TTL_MINUTES` | `30` | Reset token expiry |
| `FORGOT_PER_MIN_PER_EMAIL` | `3` | Forgot-password rate limit per email |
| `FORGOT_PER_MIN_PER_IP` | `10` | Forgot-password rate limit per IP |
| `RESET_PER_MIN_PER_IP` | `5` | Reset-password rate limit per IP |

---

## Metrics Reference

| Metric | Type | Labels | Description |
|--------|------|--------|-------------|
| `auth_login_total` | Counter | `result`, `reason` | Login attempts by outcome |
| `auth_register_total` | Counter | `result`, `reason` | Registration attempts |
| `auth_refresh_total` | Counter | `result`, `reason` | Refresh attempts |
| `auth_logout_total` | Counter | `result`, `reason` | Logout attempts |
| `auth_rate_limited_total` | Counter | `scope` (ip\|email\|refresh) | Rate-limited requests |
| `auth_nats_publish_fail_total` | Counter | `event_type` | NATS publish failures |
| `auth_refresh_replay_total` | Counter | `tenant_id` | Revoked-token replay attempts |
| `http_request_duration_seconds` | Histogram | `path`, `method`, `status` | HTTP latency |
| `auth_password_verify_duration_seconds` | Histogram | `result` (ok\|fail\|error) | Argon2 verification latency |
| `auth_dependency_up` | Gauge | `dep` (db\|nats) | Dependency health |
| `auth_entitlement_cache_hit_total` | Counter | — | Entitlement cache hits |
| `auth_entitlement_fetch_total` | Counter | `result` (ok\|fail) | Tenant-registry entitlement fetches |
| `auth_entitlement_denied_total` | Counter | `reason` | Logins denied due to unavailable entitlement |
| `auth_tenant_status_cache_hit_total` | Counter | — | Tenant status cache hits |
| `auth_tenant_status_fetch_total` | Counter | `result` (ok\|fail) | Tenant status fetches |
| `auth_tenant_status_denied_total` | Counter | `status` | Auth denied due to tenant lifecycle status |

---

## Document Standards Reference

This document follows the revision and decision log standards defined at:
`docs/frontend/DOC-REVISION-STANDARDS.md`

---

## Decision Log

Every significant product, architecture, or standards decision is recorded here.

| Date | Decision | Rationale | Decided By |
|------|----------|-----------|-----------|
| 2026-02-14 | Single crate (`auth-rs`) owns all credential, token, and RBAC data | Prevents credential sprawl across modules; single audit surface for security changes | Platform Orchestrator |
| 2026-02-14 | RS256 JWT with JWKS endpoint for public key distribution | RS256 allows asymmetric verification — modules need only the public key, not the private key; JWKS enables zero-coordination key rotation | Platform Orchestrator |
| 2026-02-14 | Refresh tokens hashed (SHA-256), never stored plaintext | DB compromise must not yield usable bearer tokens | Platform Orchestrator |
| 2026-02-14 | Argon2id for password hashing (not bcrypt or scrypt) | Argon2id is the Password Hashing Competition winner; configurable memory/time/parallelism; OWASP recommended | Platform Orchestrator |
| 2026-02-19 | RBAC claims embedded in JWT at issuance, not checked per-request | Zero runtime overhead at module boundaries; trade-off is a TTL-length lag on permission changes (acceptable for platform use cases) | Platform Orchestrator |
| 2026-02-19 | DB-backed session leases replace in-memory semaphore | In-memory counters are wrong under horizontal scaling; PostgreSQL advisory locks provide correct mutual exclusion across processes | Platform Orchestrator |
| 2026-02-19 | Entitlement limit fetched from tenant-registry, not static config | Plan limits vary by tenant subscription; a static env var would apply a single value to all tenants | Platform Orchestrator |
| 2026-02-19 | Fail-closed on entitlement unavailability | Allowing logins when the entitlement service is down could violate tenant plan limits at scale; stale cache for 5 minutes provides outage tolerance without abandoning the policy | Platform Orchestrator |
| 2026-02-19 | Tenant status gating (suspended/deleted deny all; past_due deny new login) | Suspended tenants must not be able to authenticate; past_due tenants have already authenticated and deserve a grace window before hard denial | Platform Orchestrator |
| 2026-02-22 | Zero-downtime key rotation via prev_key overlap window | JWT rotation previously required simultaneous invalidation of all active tokens; rolling restart with overlap means outstanding tokens remain valid through their natural TTL | Platform Orchestrator |
| 2026-02-22 | JWKS serves both current and previous key during rotation | Remote verifiers (other services) need to see the new key ID before old tokens expire; serving both keys prevents a validation gap | Platform Orchestrator |
| 2026-02-23 | Reset token stored as SHA-256 hash; raw_token dispatched on NATS | Raw token never touches the DB; if the DB is compromised, reset tokens cannot be used; NATS TLS requirement ensures transport security | Platform Orchestrator |
| 2026-02-23 | Atomic `claim_reset_token()` via single UPDATE...RETURNING | Single DB round-trip prevents concurrent double-claim without needing a separate read-then-update pattern | Platform Orchestrator |
| 2026-02-23 | Hard-delete sessions on password reset (not soft revoke) | Password reset is a security event; all prior sessions must be invalidated with certainty; soft revoke leaves the door open for race conditions | Platform Orchestrator |
| 2026-02-23 | Session revocation failure on reset returns 500 | There is no safe partial reset — if sessions cannot be revoked, the reset must fail; best-effort behavior would leave compromised sessions active | Platform Orchestrator |
| 2026-02-23 | `forgot-password` always returns 200 | Returning 404 for unknown emails enables email enumeration attacks; timing attacks are mitigated by applying rate limits before DB lookup | Platform Orchestrator |
| 2026-02-24 | No HTTP RBAC management API in v1 | Authorization bootstrapping is complex (who manages the roles?); defer to a future admin plane bead with proper governance | Platform Orchestrator (bd-15s1) |
