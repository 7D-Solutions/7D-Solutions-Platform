# Consumer Guide — Authentication & HTTP Protocol

> **Who reads this:** Agents building vertical apps on the 7D Platform.
> **What it covers:** Required HTTP headers, error format, identity-auth API, JWT verification in your service.
> **Parent:** [PLATFORM-CONSUMER-GUIDE.md](./PLATFORM-CONSUMER-GUIDE.md)

## Revision History

| Rev | Date | Changed By | Summary |
|-----|------|-----------|---------|
| 1.0 | 2026-02-20 | Platform Orchestrator | Extracted from PLATFORM-CONSUMER-GUIDE.md. Required HTTP headers, error format, identity-auth endpoints, JWT verification, permission strings. |

---

## Required HTTP Headers

Source: `modules/party/src/http/party.rs` (enforced pattern across all modules)

```
x-app-id: <your-app-id>           # REQUIRED — wrong or missing = 400 or empty results
x-tenant-id: <tenant-uuid>        # REQUIRED — read directly from header by handlers (NOT extracted from JWT)
x-correlation-id: <uuid>          # REQUIRED — propagated through audit trail
x-actor-id: <user-or-service-uuid> # REQUIRED — who is performing the action
Authorization: Bearer <jwt>        # REQUIRED — JWT from identity-auth
```

**If `x-app-id` is missing:** Party Master returns `400 Bad Request` with `{ "error": "missing_header", "message": "x-app-id header is required" }`. Other modules may return empty results silently — which looks like "no data" and is hard to debug. Always include it.

---

## Error Response Format

**Consistent across all platform modules:**

```json
{
  "error": "error_code",
  "message": "Human-readable description"
}
```

Source: `modules/party/src/http/party.rs` → `ErrorBody { error: String, message: String }`, `modules/ar/src/models/common.rs` → `ErrorResponse::new(code, message)`.

Common error codes: `missing_header`, `not_found`, `validation_error`, `duplicate_email`, `conflict`, `database_error`.

---

## Authentication (identity-auth)

Source: `platform/identity-auth/src/auth/handlers.rs`, `platform/identity-auth/src/auth/jwt.rs`, `platform/identity-auth/src/main.rs`

**Base URL:** `http://7d-auth-lb:8080`

### 1. Register a User

```
POST /api/auth/register
Content-Type: application/json
```

Request body (all fields required):
```json
{
  "tenant_id": "550e8400-e29b-41d4-a716-446655440000",
  "user_id": "6ba7b810-9dad-11d1-80b4-00c04fd430c8",
  "email": "user@example.com",
  "password": "SecurePass123!"
}
```

- `user_id`: You generate this UUID before calling register. Store it.
- Password policy: enforced (minimum complexity — strong passwords required)
- Returns `200 OK` with `{ "ok": true }`
- Returns `409 Conflict` if email already registered for this tenant

### 2. Login

```
POST /api/auth/login
Content-Type: application/json
```

Request body (all fields required):
```json
{
  "tenant_id": "550e8400-e29b-41d4-a716-446655440000",
  "email": "user@example.com",
  "password": "SecurePass123!"
}
```

Response `200 OK`:
```json
{
  "token_type": "Bearer",
  "access_token": "<jwt>",
  "expires_in_seconds": 900,
  "refresh_token": "<opaque-token>"
}
```

Error cases:
- `401 Unauthorized`: invalid credentials (wrong password or email not found)
- `403 Forbidden`: account inactive, or tenant status suspended/canceled
- `423 Locked`: account temporarily locked after failed attempts
- `429 Too Many Requests`: rate limited (includes `Retry-After` header in seconds)
- `503 Service Unavailable`: tenant status check unavailable (fail-closed)

### 3. Refresh Token

Source: `platform/identity-auth/src/auth/session.rs` → `RefreshReq`

```
POST /api/auth/refresh
```

Request body:
```json
{
  "tenant_id": "<tenant-uuid>",
  "refresh_token": "<opaque-token-from-login>"
}
```

Response body: same `TokenResponse` shape as login (new `access_token` + new `refresh_token`).

Error cases:
- `401 Unauthorized`: refresh token expired, revoked, or not found
- `403 Forbidden`: tenant suspended/canceled

### 4. JWKS (Public Key for Verification)

```
GET /.well-known/jwks.json
```

Returns RSA public key for JWT signature verification. Algorithm: **RS256**.

### JWT Claims Structure

Source: `platform/identity-auth/src/auth/jwt.rs` → `AccessClaims`

```json
{
  "sub": "<user-id-uuid>",
  "iss": "auth-rs",
  "aud": "7d-platform",
  "iat": 1708300000,
  "exp": 1708300900,
  "jti": "<unique-token-uuid>",
  "tenant_id": "<tenant-uuid>",
  "app_id": null,
  "roles": ["operator", "driver"],
  "perms": ["ar.mutate", "ar.read"],
  "actor_type": "user",
  "ver": "1"
}
```

Field notes:
- `sub` = `user_id` UUID string — use as `x-actor-id` in downstream calls
- `actor_type` values: `"user"` | `"service"` | `"system"`
- `app_id`: currently always `null` in issued tokens (field exists in `AccessClaims` as `Option<String>` but is not populated). Do NOT rely on this field. The `x-app-id` HTTP header (your product slug like `"trashtech-pro"`) is separate and always required.
- `ver` = `"1"` (current schema version)
- For service-to-service calls: obtain a service account token with `actor_type: "service"`

---

## JWT Verification in Your Service

Source: `platform/security/src/claims.rs`, `platform/security/src/authz_middleware.rs`

The `security` crate provides ready-made JWT verification and permission enforcement. **Do not implement your own JWT validation.**

### JwtVerifier Setup

```rust
// In your main.rs or startup code
use security::claims::JwtVerifier;
use std::sync::Arc;

// Option A: From environment variable (recommended for production)
// Reads JWT_PUBLIC_KEY env var. Returns None if not set (dev mode).
let verifier: Option<Arc<JwtVerifier>> = JwtVerifier::from_env().map(Arc::new);

// Option B: From PEM string (for tests)
let verifier = Arc::new(JwtVerifier::from_public_pem(&pem_string).unwrap());
```

**JwtVerifier validates:** RS256 signature, expiration, issuer (`"auth-rs"`), audience (`"7d-platform"`).

### VerifiedClaims (What You Get After Verification)

Source: `platform/security/src/claims.rs`

```rust
pub struct VerifiedClaims {
    pub user_id: Uuid,           // from JWT "sub"
    pub tenant_id: Uuid,         // from JWT "tenant_id"
    pub app_id: Option<Uuid>,    // from JWT "app_id" (may be None)
    pub roles: Vec<String>,      // from JWT "roles"
    pub perms: Vec<String>,      // from JWT "perms"
    pub actor_type: ActorType,   // User | Service | System
    pub issued_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub token_id: Uuid,          // from JWT "jti"
    pub version: String,         // from JWT "ver"
}

pub enum ActorType { User, Service, System }
```

### Wiring Into Axum Router

Source: `platform/security/src/authz_middleware.rs`

> **⚠ Axum route ordering is load-bearing.** `route_layer` protects ONLY the routes defined ABOVE it. Routes defined BELOW `.route_layer(...)` are NOT protected — they are publicly accessible. Never group all routes together and add layers at the end.

```rust
use security::authz_middleware::{ClaimsLayer, RequirePermissionsLayer, optional_claims_mw};
use security::claims::JwtVerifier;
use std::sync::Arc;

let verifier: Option<Arc<JwtVerifier>> = JwtVerifier::from_env().map(Arc::new);

let app = Router::new()
    // Mutation routes — require specific permissions
    .route("/api/yourapp/orders", post(create_order))
    // IMPORTANT: route_layer applies ONLY to routes defined ABOVE it in this chain.
    // Routes defined below are NOT protected by RequirePermissionsLayer.
    // Do NOT move or reorganize routes without understanding this Axum ordering rule.
    .route_layer(RequirePermissionsLayer::new(&["yourapp.mutate"]))
    // Read routes — no permission guard (or use yourapp.read)
    .route("/api/yourapp/orders", get(list_orders))
    .route("/api/yourapp/orders/{id}", get(get_order))
    // Health/ready — no auth required
    .route("/api/health", get(health))
    .route("/api/ready", get(ready))
    // Claims extraction layer (must be outermost)
    .layer(axum::middleware::from_fn_with_state(verifier, optional_claims_mw));
```

**Behavior:**
- `ClaimsLayer::permissive(verifier)` — requests without valid JWT pass through (claims absent). Use with `RequirePermissionsLayer` on mutation routes.
- `ClaimsLayer::strict(verifier)` — requests without valid JWT get `401 Unauthorized`.
- `optional_claims_mw` — Axum middleware function variant. If `verifier` is `None` (dev mode, no JWT_PUBLIC_KEY), no claims extracted, mutation routes return `401`.
- `RequirePermissionsLayer::new(&["yourapp.mutate"])` — checks `VerifiedClaims.perms` contains all listed strings. Returns `403` if missing, `401` if no claims at all.

### Accessing Claims in Handlers

```rust
use security::claims::VerifiedClaims;
use axum::Extension;

async fn create_order(
    Extension(claims): Extension<VerifiedClaims>,
    // ... other extractors
) -> impl IntoResponse {
    let tenant_id = claims.tenant_id;
    let user_id = claims.user_id;
    let actor_type = claims.actor_type.as_str(); // "user", "service", "system"
    // ...
}
```

### Permission Strings

Source: `platform/security/src/permissions.rs`

Convention: `<module>.mutate` for writes, `<module>.read` for reads, `gl.post` for GL journal posting.

Existing constants:
```
ar.mutate, ar.read, payments.mutate, payments.read, subscriptions.mutate,
gl.post, gl.read, notifications.mutate, inventory.mutate, inventory.read,
reporting.mutate, treasury.mutate, treasury.read, ap.mutate, ap.read,
consolidation.mutate, consolidation.read, timekeeping.mutate, timekeeping.read,
fixed_assets.mutate, fixed_assets.read, yourapp.mutate, yourapp.read
```

---

> See `docs/PLATFORM-CONSUMER-GUIDE.md` for the master index and critical concepts.
