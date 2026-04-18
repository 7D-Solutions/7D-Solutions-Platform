# Identity-Auth: Refresh Tokens + Sliding-Expiry Sessions

**Status:** Implemented (bd-b9nof, 2026-04-18)
**Module:** `platform/identity-auth`
**Related tables:** `refresh_sessions` (new), `refresh_tokens` (legacy, preserved)

## Problem

Platform identity-auth issued access tokens (JWT) with a hard TTL (15 minutes).
The existing `refresh_tokens` table supported body-based rotation but had **no
sliding-expiry** semantics: a token's `expires_at` was fixed at creation
(`REFRESH_TOKEN_TTL_DAYS`), so an idle user's session did not decay and an active
user's session did not extend. Apps compensated with longer cookie lifetimes on
their own access-token cookies (e.g. hp-backend: 8h), but the embedded JWT's
`exp` was still 15 min and active users were logged out mid-session.

## Design

A short-lived access token (15-30 min JWT) + a long-lived refresh token
(opaque, server-side) with **two separate time windows**:

- **Sliding idle window** (`REFRESH_IDLE_MINUTES`, default 480 = 8h).
  On every successful `/api/auth/refresh`, the session's `last_used_at` is set
  to `NOW()` and `expires_at` is extended to `NOW() + REFRESH_IDLE_MINUTES`.
  Idle > 8h → next refresh returns `401 refresh_invalid`.

- **Absolute maximum lifetime** (`REFRESH_ABSOLUTE_MAX_DAYS`, default 30).
  Fixed at `issued_at + REFRESH_ABSOLUTE_MAX_DAYS`. A session, even if kept
  continuously active, expires at this hard cap. A compromised refresh cookie
  cannot outlive this window.

Active users stay logged in; idle users are kicked at the idle boundary.

## Schema: `refresh_sessions`

```sql
CREATE TABLE refresh_sessions (
    session_id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id UUID NOT NULL,
    user_id UUID NOT NULL,
    token_hash TEXT NOT NULL,                -- SHA-256 of opaque cookie value
    device_info JSONB NOT NULL DEFAULT '{}', -- ip, user_agent at session start
    issued_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    last_used_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    expires_at TIMESTAMPTZ NOT NULL,         -- sliding
    absolute_expires_at TIMESTAMPTZ NOT NULL,-- hard max
    revoked_at TIMESTAMPTZ,
    revocation_reason TEXT,
    CONSTRAINT refresh_sessions_token_hash_unique UNIQUE (token_hash)
);

CREATE INDEX idx_refresh_sessions_user        ON refresh_sessions (user_id, tenant_id);
CREATE INDEX idx_refresh_sessions_user_active ON refresh_sessions (user_id, tenant_id)
    WHERE revoked_at IS NULL;
CREATE INDEX idx_refresh_sessions_token_hash  ON refresh_sessions (token_hash);
```

Coexists with the legacy `refresh_tokens` table: body-flow callers keep
working unchanged, cookie-flow callers use `refresh_sessions`.

## Endpoints

### `POST /api/auth/login`

Unchanged response contract (still returns `{ access_token, refresh_token }`
in the body for legacy callers). Additionally:

- Creates a `refresh_sessions` row with `device_info` populated from the
  `X-Forwarded-For` / `User-Agent` headers.
- Plants `Set-Cookie: refresh=<opaque>; Path=/api/auth; HttpOnly; SameSite=Lax;
  Max-Age=<sliding-seconds>; Secure` (Secure is omitted when `ENV=development`).
- Emits `identity_auth.session_created`.

### `POST /api/auth/refresh`

**Cookie flow (preferred):** If the `Cookie: refresh=<opaque>` header is
present, the handler validates the session (not revoked, `expires_at > NOW()`,
`absolute_expires_at > NOW()`), rotates the session, mints a new access token,
rolls the `Set-Cookie` header, updates `last_used_at`, and emits
`identity_auth.session_refreshed`.

Response body: `{ token_type, access_token, expires_in_seconds }` — refresh
token is NOT in the body (it's in the cookie).

**Body flow (legacy):** If no cookie is present, falls through to the existing
`refresh_tokens` flow with its body-based request/response. Unchanged.

**Replay detection:** A previously rotated (revoked) token presented again
triggers `refresh_sessions::revoke_all_for_user(...)` — **all** live sessions
for the affected user are revoked immediately. Returns 401.

### `POST /api/auth/logout`

Cookie-aware. If a refresh cookie is present, revokes the `refresh_sessions`
row. Clears the cookie with `Max-Age=0`. If a legacy body is present, revokes
the corresponding `refresh_tokens` row. Emits `identity_auth.session_revoked`
for the cookie-flow path.

### `GET /api/auth/sessions?tenant_id=…&user_id=…`

Returns all active (`revoked_at IS NULL AND expires_at > NOW() AND
absolute_expires_at > NOW()`) sessions for a user: `session_id`, `device_info`,
`issued_at`, `last_used_at`, `expires_at`, `absolute_expires_at`.

### `POST /api/auth/sessions/{session_id}/revoke`

Body: `{ tenant_id, user_id }`. Revokes a specific session iff it belongs to
the caller. Emits `identity_auth.session_revoked`. Returns 404 if the session
does not exist, is already revoked, or belongs to a different user/tenant.

## Config (docker-compose / env)

| Variable                     | Default | Purpose                                  |
| ---------------------------- | ------- | ---------------------------------------- |
| `ACCESS_TOKEN_TTL_MINUTES`   | 30      | Access-token TTL (short, quick revoke)   |
| `REFRESH_IDLE_MINUTES`       | 480     | Sliding idle window (8h)                 |
| `REFRESH_ABSOLUTE_MAX_DAYS`  | 30      | Hard cap regardless of activity          |
| `REFRESH_TOKEN_TTL_DAYS`     | 14      | Legacy body-flow refresh TTL (unchanged) |

## Events

Three new event types (follow EventEnvelope convention, no `.v1` in
`event_type`):

- `identity_auth.session_created`  — emitted on login
- `identity_auth.session_refreshed` — emitted on cookie-flow refresh (carries
  `previous_session_id` for chain audit)
- `identity_auth.session_revoked`  — emitted on logout / explicit revoke /
  replay detection

Schemas live alongside the service in
`platform/identity-auth/src/events/schemas/identity_auth.session.*.v1.json`
and also in `contracts/events/identity-auth-session-*.v1.json` for
cross-module consumers.

## Security Posture

- Raw refresh tokens never stored (SHA-256 hashed).
- Cookie is `HttpOnly` (no JavaScript access) + `SameSite=Lax` (CSRF
  mitigation) + `Secure` outside development.
- Rotation on every use: presenting an old rotated token signals compromise →
  all user sessions revoked.
- Hard-max cap means a stolen cookie cannot outlive `REFRESH_ABSOLUTE_MAX_DAYS`
  even with continuous sliding.
- Revocation checked on every `/refresh` against `revoked_at IS NULL` — a
  revoked session cannot issue a new access token even if the client still
  physically holds the cookie.
- Rate-limiting per (tenant, token-hash prefix) via the existing
  `KeyedLimiters::check_refresh` — prevents brute force.

## Backwards Compatibility

hp-backend, Fireproof, and any other consumer of `POST /api/auth/login` +
`POST /api/auth/refresh` continues to work unchanged:

- Login still returns `{ access_token, refresh_token }` in the body.
- Body-flow `/refresh` (body contains `{ tenant_id, refresh_token }`) still
  works via the legacy `refresh_tokens` table.
- The new cookie is purely additive — apps can ignore it until they opt in to
  silent refresh.

## Consumer Integration

There is no `identity-auth-sdk` crate today. Apps wanting silent-refresh
support should:

1. After login, the browser already holds the HttpOnly `refresh` cookie. No
   application code required.
2. On a 401 response from any API, POST to `/api/auth/refresh` (with
   `credentials: 'include'` to send the cookie); on success, retry the
   original request with the new access token. Do not attempt to read the
   refresh token — it is HttpOnly.
3. On logout, POST to `/api/auth/logout` with `credentials: 'include'`.

Apps requiring a backend-to-backend silent-refresh helper can vendor the
same three HTTP calls; no Rust SDK shim is needed for v1.
