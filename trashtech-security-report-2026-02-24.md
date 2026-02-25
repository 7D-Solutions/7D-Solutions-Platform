# TrashTech Pro — Deep Security Audit Report

**Date:** 2026-02-24
**Scope:** Full codebase deep investigation — Rust backend (`modules/trashtech/`), frontend demo, Docker/nginx config, production deployment (`deploy/prod/`), env files, platform integration clients
**Method:** Manual code-level audit of every handler, middleware, DB query, integration client, Docker config, and NATS consumer

---

## Executive Summary

TrashTech Pro has **strong security foundations** and demonstrates mature security engineering. After a thorough code-level deep investigation of every endpoint handler, every DB query pattern, every integration client, both Docker Compose configurations, the production Caddyfile, all NATS consumers, and the full auth middleware stack, I can confirm:

- **No critical vulnerabilities found.**
- **No SQL injection vectors found.** Zero `format!()` SQL construction with user input anywhere.
- **No IDOR vulnerabilities found.** Every customer/driver endpoint resolves identity exclusively from JWT claims, never from client-supplied IDs.
- **No cross-tenant data leakage found.** Database-per-tenant architecture provides physical isolation.
- **No SSRF vectors found.** Platform client URLs are loaded from env vars at startup, never from user input.
- **No path traversal in media uploads.** Storage keys are server-generated using UUIDs; client cannot supply keys.

There are **three medium-severity** and **three low-severity** findings to address, all pre-production hardening items rather than architectural flaws.

---

## DEEP INVESTIGATION RESULTS

### 1. SendGrid Webhook — Full Attack Surface Analysis

**File:** `tt-api/src/sendgrid_webhook.rs`

**Finding M1 (MEDIUM): No Signature Verification**

The `POST /api/webhooks/sendgrid/events` endpoint is public (no JWT) and has no signature verification. The code comments acknowledge this explicitly.

**Deep investigation trace:**

The `handle_events` handler parses a JSON array of `SendGridEvent` objects, filters for `bounce|blocked|spamreport` event types, then calls `customer::set_email_invalid_by_email()` across ALL tenant pools via `resolver.all_pools()`. This means a single forged bounce event for `jane@example.com` will mark that email as `email_invalid = true` in every tenant database where a customer has that email.

**Cascading impact:**
- Password reset emails are suppressed when `email_invalid = true` (verified in `password_reset_handler.rs:223`)
- Any future transactional emails to that customer will be suppressed
- This is a multi-tenant denial-of-service against email communications

**Mitigating factor:** The `email_invalid` flag only prevents new sends; it doesn't delete customer records or affect authentication.

**Fix:** Implement SendGrid's signed event webhook verification using the `SENDGRID_WEBHOOK_PUBLIC_KEY` env var before production launch.

---

### 2. Tenant Isolation — Every Handler Verified

**Result: EXCELLENT — No cross-tenant leakage found.**

Every endpoint was traced to verify tenant scoping:

| Endpoint | Tenant Source | Isolation Method |
|----------|--------------|-----------------|
| `GET /api/customer/me` | `claims.tenant_id` → `resolver.resolve()` → per-tenant pool | DB-per-tenant + `user_id` from JWT |
| `GET /api/customer/invoices` | `claims.tenant_id` → per-tenant pool | customer resolved from JWT `user_id` chain |
| `GET /api/customer/map/live` (SSE) | `claims.tenant_id` → per-tenant pool | customer_id derived from JWT user_id |
| `GET /api/admin/map/live` (SSE) | `claims.tenant_id` → per-tenant pool | All queries scoped to tenant pool |
| `POST /api/driver/stops/{id}/start` | `claims.tenant_id` → per-tenant pool | driver_id from JWT, route_run checked with `tenant_id` |
| `POST /api/driver/stops/{id}/complete` | `claims.tenant_id` → per-tenant pool | Same as above |
| `POST /api/driver/stops/{id}/skip` | `claims.tenant_id` → per-tenant pool | Same as above |
| `POST /api/driver/evidence/{id}/media/presign-upload` | `claims.tenant_id` → per-tenant pool | evidence ownership verified against driver_id from JWT |
| `POST /api/auth/login` | Body `tenant_id` (pre-auth) | Rate-limited per (email, tenant_id) pair |
| `GET /api/portal/tenant?slug=` | Public, no tenant context | Only returns non-sensitive `tenant_id` + name |
| `POST /api/webhooks/sendgrid/events` | No tenant context | Iterates ALL tenant pools (by design — see M1) |

**Key architectural strength:** The database-per-tenant pattern means even if a query inside a handler omits `tenant_id` in its WHERE clause (e.g., `get_stop()` queries by `stop_id` alone), it's still safe because the query runs against a per-tenant pool that only contains that tenant's data.

**`get_stop()` note:** `route_run::get_stop(&pool, stop_id)` queries without `tenant_id`, but this is safe because:
1. `pool` is the per-tenant pool resolved from `claims.tenant_id`
2. The subsequent `get_route_run()` call verifies `tenant_id` matches
3. Even without the second check, the per-tenant pool only contains that tenant's data

---

### 3. Auth Login Proxy — Credential Handling

**File:** `tt-api/src/auth_login.rs`

**Result: GOOD — No credential leakage found.**

- Credentials (`email`, `password`) are forwarded to identity-auth in the request body (not URL params)
- No logging of passwords or tokens
- Rate limiting applied BEFORE upstream call (5 attempts/min per email+tenant pair)
- Error responses from identity-auth forwarded transparently (401, 403, 423, 429, 503)
- `Retry-After` header properly forwarded on 429 responses

**Minor observation:** The login endpoint uses `post_json_anon` (no auth headers), which is correct since the caller doesn't have a token yet. The method only injects `Content-Type: application/json` — no inadvertent header leakage.

---

### 4. Media Presign/Upload — Path Traversal & SSRF Analysis

**Files:** `tt-api/src/driver_media_presign.rs`, `tt-api/src/driver_stops.rs` (presign + attach media), `tt-integrations/src/media_storage.rs`

**Result: EXCELLENT — No path traversal or SSRF vectors found.**

**Storage key generation is server-side:**
```
{tenant_id}/{evidence_id}/{uuid}.{ext}
```
- `tenant_id` from JWT claims (cannot be forged)
- `evidence_id` from DB lookup (UUID, not user-supplied)
- `uuid` is `Uuid::new_v4()` (server-generated)
- `ext` is validated: `chars().all(|c| c.is_ascii_alphanumeric())` — no dots, slashes, or special chars allowed

**Client cannot supply storage keys for presign.** The client receives an opaque `storage_key` from the presign endpoint and must pass it back verbatim when recording the media ref.

**SSRF analysis:** The `MediaStorageClient` only connects to `S3_ENDPOINT` from env var (set at startup). No user input flows into the endpoint URL. The presigned URL is returned to the client, not followed by the server.

**Ownership guards verified:**
- `presign_upload`: Verifies `ev.driver_id == profile.driver_id` (evidence record must belong to calling driver)
- `presign_stop_media_upload`: Same ownership check via `ev.driver_id != driver_id`
- `attach_stop_media_endpoint`: Same ownership check

---

### 5. Customer Portal Endpoints — Authorization Boundaries

**Files:** `tt-api/src/customer_me.rs`, `tt-api/src/customer_invoices.rs`

**Result: EXCELLENT — IDOR-proof by design.**

Both customer endpoints resolve identity exclusively from JWT claims:
1. `claims.user_id` → `customer_profiles.get_by_user_id()` → `party_id`
2. `party_id` → `customers.get_customer_by_party_id()` → `customer_id`
3. Data queried using `customer_id` derived from this chain

**No client-supplied IDs are accepted.** The `customer_me.rs` header comment explicitly documents this: "No client-supplied IDs are accepted. IDOR-safe."

Customer A cannot access Customer B's data because:
- JWT `user_id` uniquely identifies the caller
- `customer_profiles` maps `user_id` to exactly one `party_id` per tenant
- No query parameter or path variable allows overriding this resolution

---

### 6. Driver Endpoints — Authorization Boundaries

**File:** `tt-api/src/driver_stops.rs`

**Result: EXCELLENT — Proper ownership enforcement.**

All three driver stop actions (`start`, `complete`, `skip`) follow the same pattern:
1. `claims.user_id` → `driver_profile::get_by_user_id()` → `driver_id`
2. Load stop by `stop_id` (from URL path)
3. Load parent route run and verify `run.assigned_driver_id == Some(driver_id)`
4. If mismatch → `403 Forbidden` ("not assigned")

A driver cannot operate on stops assigned to another driver. The stop_id comes from the URL path (which is a UUID, not guessable), and ownership is always verified against the JWT-derived driver identity.

**Additional checks in complete_stop:**
- Geofence validation (location check against service address coordinates)
- RFID UID validation against bin registry + address matching
- Idempotency key support for safe offline replay
- Force-completion requires explicit reason with notes for certain categories

---

### 7. SSE Streams — Cross-Tenant Data Leakage Analysis

**Files:** `tt-api/src/customer_sse.rs`, `tt-api/src/admin_sse.rs`

**Result: EXCELLENT — No cross-tenant leakage possible.**

**Customer SSE (`/api/customer/map/live`):**
- `tenant_id` from JWT → per-tenant pool
- `customer_id` resolved from JWT `user_id` chain (same as customer_me)
- All queries join through `service_addresses.customer_id` — a customer can only see truck positions and stop states for their own service addresses
- Per-user SSE connection limit (3) and per-tenant SSE connection limit (20) prevent resource exhaustion
- Heartbeat every 5s, keepalive ping every 20s

**Admin SSE (`/api/admin/map/live`):**
- `tenant_id` from JWT → per-tenant pool
- Admin sees all data within their tenant (appropriate for admin role)
- Optional `route_run_id` query param filters to a single run (validated as UUID)
- Same per-user (3) and per-tenant (20) connection limits

**SSE resource exhaustion protection:** Both endpoints acquire `OwnedSemaphorePermit` from `ConnectionLimiter` BEFORE any DB work. Permits are held for the SSE stream lifetime and released automatically on disconnect (RAII pattern).

---

### 8. Docker Compose — Production Security Analysis

**Files:** `modules/trashtech/docker-compose.yml` (dev), `deploy/prod/docker-compose.yml` (prod)

**Result: GOOD (dev), EXCELLENT (prod)**

**Production compose (`deploy/prod/docker-compose.yml`):**
- All internal services use `expose` (not `ports`) — no host port binding except Caddy on 80/443
- Caddy provides automatic HTTPS via Let's Encrypt
- Secrets loaded from `/opt/trashtech/secrets/backend.env` at runtime, never baked into images
- PostgreSQL, NATS, and MinIO are internal-only (no exposed ports)
- Non-root containers: tt-server (uid 1001), tt-frontend (uid 1001), tt-portal (nginx uid 101), MinIO (uid 1000), NATS (uid 1000), Postgres (uid 70)
- Image digests are pinned for reproducible builds (nginx alpine)
- MinIO is gated behind `--profile minio` for optional self-hosted S3
- All services have healthchecks with appropriate start periods

**Dev compose (`modules/trashtech/docker-compose.yml`):**
- Exposes ports to host (`8103:8101`, `3001:3000`, `3003:8080`, `8000:80`) — appropriate for dev
- No production secrets in dev compose

**Caddyfile (`deploy/prod/Caddyfile`):**
- HSTS with 1-year max-age, includeSubDomains, preload
- X-Content-Type-Options: nosniff
- X-Frame-Options: DENY
- Referrer-Policy: strict-origin-when-cross-origin
- Server banner removed
- SSE flush_interval: -1 for immediate flushing

---

### 9. Password Reset Flow — Token Leakage Analysis

**File:** `tt-events/src/password_reset_handler.rs`

**Result: GOOD — No token leakage found, one observation.**

The flow:
1. identity-auth publishes `auth.events.password_reset_requested` on NATS
2. TrashTech subscribes and receives `{ email, raw_token }` in an `EventEnvelope`
3. Customer is looked up by email in the tenant DB
4. If `email_invalid = true` → suppressed (no email sent)
5. Reset link constructed as `{portal_base_url}/auth/reset?token={raw_token}`
6. Email sent via SendGrid

**Security strengths:**
- TrashTech does NOT store the reset token — identity-auth owns the token lifecycle
- Email suppression respects bounce status
- Tenant routing verified via EventEnvelope `tenant_id`
- Unknown tenants and missing customers are gracefully skipped

**Finding L3 (LOW): Reset Token in URL Query Parameter**

The reset token is placed in the URL query string: `?token={raw_token}`. While this is standard practice and the token is described as "base64url / alphanumeric, URL-safe by contract," URL query parameters can leak through:
- Browser history
- Referrer headers (mitigated by Caddy's `Referrer-Policy: strict-origin-when-cross-origin`)
- Server access logs

The Caddy Referrer-Policy header provides mitigation for cross-origin referrer leakage. This is industry-standard practice for password reset flows and not a significant risk, but noted for completeness.

---

### 10. Input Validation — All Endpoints

**File:** `tt-api/src/validate.rs` and individual handler files

**Result: GOOD — Consistent validation patterns.**

| Endpoint | Validation |
|----------|-----------|
| Portal slug lookup | 1-63 chars, `[a-z0-9-]`, no leading/trailing hyphens, regex-validated |
| Media presign upload | `mime_type` non-empty + format check, `file_extension` non-empty + alphanumeric-only |
| Stop skip | Notes non-empty (trimmed) |
| Stop complete | Mode enum validation, force_reason enum validation, notes required for certain force reasons |
| Login | Structural validation via serde deserialization (email as String, password as String, tenant_id as UUID) |
| SendGrid webhook | Structural JSON validation; invalid payloads return 200 to prevent retries |

**`format!()` SQL construction audit:** I searched for all instances of `format!()` near SQL keywords across the entire TrashTech codebase. The ONLY `format!()` usage in SQL queries is for interpolating the compile-time constant `STOP_COLUMNS` and `RUN_COLUMNS` into queries — these are string literals defined at compile time (`const STOP_COLUMNS: &str = "id, route_run_id, ..."`) and cannot be influenced by user input. All user-supplied values use parameterized bindings (`$1`, `$2`, etc.).

---

### 11. Platform Integration Clients — SSRF Analysis

**File:** `tt-integrations/src/platform_client.rs`

**Result: EXCELLENT — No SSRF vectors found.**

The `PlatformClient` constructs all URLs from `PlatformConfig` fields:
- `party_base_url` → `PARTY_BASE_URL` env var
- `ar_base_url` → `AR_BASE_URL` env var
- `auth_base_url` → `AUTH_BASE_URL` env var
- `ap_base_url` → `AP_BASE_URL` env var

These are set once at startup via `from_env()` and never modified. No user input flows into URL construction. The `RequestContext` carries `tenant_id`, `actor_id`, `correlation_id`, and `bearer_token` as headers — all from the verified JWT, not from request parameters.

**Retry logic is safe:** Retries on 429/503/transport errors with exponential backoff, capped at 3 retries. The `Retry-After` header from upstream services is respected.

---

## FINDINGS SUMMARY

### M1. SendGrid Webhook Has No Signature Verification (MEDIUM)

**File:** `tt-api/src/sendgrid_webhook.rs`

Without signature verification, any attacker who knows the endpoint URL can POST fabricated bounce events, causing customer emails to be marked as `email_invalid = true` across all tenants. This suppresses password reset emails and all future transactional email to affected addresses.

**Fix:** Implement SendGrid's signed event webhook verification before production launch.

---

### M2. Root `.env.example` Contains Weak Placeholder Secrets (MEDIUM)

**File:** `.env.example` (tracked in git)

Weak placeholder values (`JWT_SECRET=change_this_in_production`, `DB_PASSWORD=trashtech_dev_password`) could be accidentally deployed if `.env.example` is copied to `.env` without modification.

**Fix:** Add a startup validation check that refuses to start if secrets match known placeholder values. The production deployment already uses `/opt/trashtech/secrets/backend.env` — ensure the dev setup also has a guard.

---

### M3. MinIO Default Credentials in Backend `.env.example` (MEDIUM)

**File:** `modules/trashtech/.env.example` (tracked in git)

Ships `S3_ACCESS_KEY_ID=minioadmin` / `S3_SECRET_ACCESS_KEY=minioadmin` — MinIO defaults that could be deployed unchanged.

**Fix:** Replace with placeholder comments. The production compose properly uses `${MINIO_ROOT_USER}` / `${MINIO_ROOT_PASSWORD}` from env vars, but the dev `.env.example` should not contain working credentials.

---

### L1. PII Redaction in Audit Log Uses Truncation, Not Hashing (LOW)

**File:** `tt-api/src/sendgrid_webhook.rs:175-183`

The `sha256_hex()` function truncates to first 4 characters rather than computing SHA-256. `john***` still leaks PII.

**Fix:** Add `sha2` to `Cargo.toml` and implement proper SHA-256 hashing.

---

### L2. Nginx Dev Config Missing Security Headers (LOW)

**File:** `modules/trashtech/nginx.conf`

The dev nginx config has no security headers. However, the **production Caddyfile** (`deploy/prod/Caddyfile`) correctly includes HSTS, X-Frame-Options, X-Content-Type-Options, and Referrer-Policy. This finding is downgraded to LOW because it only affects the dev environment.

**Fix:** Add security headers to the dev nginx config for defense-in-depth during development.

---

### L3. Password Reset Token in URL Query Parameter (LOW)

**File:** `tt-events/src/password_reset_handler.rs:249-252`

Reset tokens are placed in URL query strings (`?token={raw_token}`), which can leak through browser history and server access logs. Mitigated by Caddy's `Referrer-Policy: strict-origin-when-cross-origin`.

**Fix:** Consider using a POST-based token exchange (token in form body) rather than a GET query parameter. Alternatively, ensure token TTL is short (verify with identity-auth configuration).

---

## POSITIVE FINDINGS (What's Working Well)

| Area | Assessment | Deep Investigation Details |
|------|-----------|---------------------------|
| **JWT Verification** | EXCELLENT | RS256 via JWKS with background cache refresh every 5 min, kid-miss recovery with immediate re-fetch, proper exp/iss/aud validation. `JwtVerifier` struct with `Arc<RwLock<HashMap<String, DecodingKey>>>` for thread-safe key rotation. |
| **Permission Middleware** | EXCELLENT | Layered `RequirePermissionsLayer` — mutations need `trashtech.mutate`, reads need `trashtech.read`, public routes placed below both layers in the router tree. Verified that layer ordering in `lib.rs` is correct. |
| **Tenant Isolation** | EXCELLENT | Database-per-tenant via `TenantResolver`. Every handler extracts `claims.tenant_id` and resolves to the correct pool. Physical DB isolation means even queries without `tenant_id` in WHERE clause are safe. Verified across all 12+ endpoint handlers. |
| **IDOR Prevention** | EXCELLENT | Customer endpoints resolve identity from JWT `user_id` chain — no client-supplied customer/party IDs accepted. Driver endpoints verify route run assignment against JWT-derived driver_id. |
| **CORS** | EXCELLENT | Origin predicate validates `*.trashtech.app` with slug regex + configurable extra origins; `allow_credentials(true)` with specific origins (not `Any`). |
| **Rate Limiting** | EXCELLENT | Five distinct limiters: login (5/min per email+tenant), GPS ping (1/10s per tenant+user), tenant slug lookup (10/min per IP), SSE per-user (3 concurrent), SSE per-tenant (20 concurrent). All use token bucket or semaphore patterns with proper `Retry-After` headers. |
| **Media Upload Security** | EXCELLENT | Server-generated storage keys (`{tenant_id}/{evidence_id}/{uuid}.{ext}`). Client cannot influence key path. File extension validated to alphanumeric only. Ownership verified before presign and attach. |
| **SQL Injection** | EXCELLENT | Zero `format!()` SQL construction with user input. All queries use sqlx parameterized bindings. The only `format!()` in SQL is for compile-time constant column lists (`STOP_COLUMNS`, `RUN_COLUMNS`). |
| **Password Hashing** | EXCELLENT (platform) | Argon2id with configurable parameters at the identity-auth level. |
| **Unsafe Rust** | PASS | Zero `unsafe` blocks in the entire TrashTech codebase. |
| **Sensitive Logging** | PASS | No passwords, secrets, or tokens logged. Email addresses redacted in audit logs (imperfectly — see L1). NATS payload parsing errors log the raw payload but never include tokens. |
| **.env Security** | GOOD | `.env`, `.env.production`, `.env.local` all gitignored. Production secrets injected at runtime from `/opt/trashtech/secrets/backend.env`, never baked into Docker images. |
| **Production Docker** | EXCELLENT | Non-root containers across the board. Internal-only networking for all services except Caddy (80/443). Automatic HTTPS via Let's Encrypt. Image digests pinned. Healthchecks on all services. |
| **Platform Client** | EXCELLENT | No SSRF vectors. URLs from env vars only. Retry logic with exponential backoff. Required auth headers enforced at the type level (`RequestContext` struct). 30s timeout on all outbound requests. |
| **SSE Security** | EXCELLENT | Connection semaphores prevent resource exhaustion. Permits held via RAII — automatic cleanup on disconnect. Customer SSE double-joins through `customer_id` to prevent cross-customer leakage. |
| **Input Validation** | GOOD | Consistent validation on slug lookup, media uploads, stop actions. Serde deserialization provides structural validation on all JSON endpoints. UUID types prevent injection in path params. |
| **Idempotency** | EXCELLENT | Stop completion supports client-supplied idempotency keys with DB-level `completion_idempotency_key` column. SendGrid webhook uses `email_invalid = false` guard for safe re-delivery. |
| **Login Proxy** | GOOD | Rate-limited before upstream call. No credential logging. Error codes forwarded transparently including 429/Retry-After. |

---

## Priority Action Items

| # | Priority | Finding | Action |
|---|----------|---------|--------|
| 1 | **MEDIUM** | M1: SendGrid webhook unsigned | Implement webhook signature verification before production |
| 2 | **MEDIUM** | M2: Weak placeholder secrets in `.env.example` | Add startup validation rejecting placeholder secrets |
| 3 | **MEDIUM** | M3: MinIO default creds in tracked `.env.example` | Use placeholder comments instead of actual credentials |
| 4 | **LOW** | L1: PII leaks in fake `sha256_hex` | Add `sha2` crate and implement real hashing |
| 5 | **LOW** | L2: Missing nginx dev security headers | Add headers to dev nginx.conf for defense-in-depth |
| 6 | **LOW** | L3: Reset token in URL query param | Consider POST-based token exchange; verify token TTL is short |
| 7 | **CI** | Cargo audit not verified | Ensure `cargo audit` runs in CI pipeline |
| 8 | **CI** | npm audit not verified | Ensure `npm audit` runs in CI for frontend apps |

---

## Overall Security Posture

**Rating: STRONG**

TrashTech Pro demonstrates mature security engineering across every layer of the stack. The JWT→JWKS pipeline with key rotation, layered permission middleware, database-per-tenant physical isolation, IDOR-proof identity resolution from JWT claims, comprehensive multi-strategy rate limiting, server-side media key generation, zero SQL injection vectors, zero unsafe Rust, and a well-hardened production Docker deployment with automatic HTTPS place it well above average for a SaaS application at this stage.

The three medium findings (SendGrid webhook signing, placeholder secrets, MinIO defaults) are all pre-production hardening items with clear, simple fixes. The three low findings are defense-in-depth improvements. No architectural redesign is needed.

**Areas of particular excellence:**
- The database-per-tenant architecture provides a fundamentally strong isolation boundary that makes many classes of cross-tenant bugs impossible
- Customer identity resolution from JWT claims (rather than client-supplied IDs) eliminates IDOR at the design level
- The `ConnectionLimiter` semaphore pattern for SSE streams is an unusually thorough approach to real-time resource management
- The production Caddy + internal nginx architecture properly separates TLS termination from routing
