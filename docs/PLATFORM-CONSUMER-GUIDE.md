# 7D Solutions Platform — Consumer Guide for Claude Code Agents

**Audience:** Claude Code agents building vertical applications (TrashTech Pro, etc.) on the 7D Platform.
**Purpose:** Complete, source-verified API reference. Every fact here is checked against actual Rust source code.

> **All data in this file is verified against source.** File references included so you can re-verify.
> Last verified: 2026-02-19 against commit 474294cb.

---

## TABLE OF CONTENTS

1. [Critical Concepts](#critical-concepts)
2. [Ownership Boundary](#ownership-boundary)
3. [Bootstrap Checklist](#bootstrap-checklist)
4. [Module Reference](#module-reference)
5. [Required HTTP Headers](#required-http-headers)
6. [Error Response Format](#error-response-format)
7. [Authentication (identity-auth)](#authentication-identity-auth)
8. [JWT Verification in Your Service](#jwt-verification-in-your-service)
9. [Party Master](#party-master)
10. [AR Module](#ar-module--customers-and-invoices)
11. [Complete "First Invoice" Flow](#complete-first-invoice-flow)
12. [NATS Event Bus](#nats-event-bus)
13. [Outbox Pattern](#outbox-pattern--copy-this)
14. [Integrations Module](#integrations-module)
15. [Tenant Provisioning](#tenant-provisioning)
16. [Data Ownership Decision Table](#data-ownership--decision-table)
17. [Environment Variables](#environment-variables)
18. [Cargo.toml Dependencies](#cargotoml-path-dependencies)
19. [Local Development](#local-development)
20. [Reference E2E Tests](#reference-e2e-tests)
21. [Source File Index](#source-file-index)

---

## CRITICAL CONCEPTS

### You are a TENANT

- You receive a `tenant_id` (UUID) and one or more `app_id` values (short string, e.g. `trashtech-pro`) during onboarding
- Every platform API automatically scopes data to your `tenant_id` — you cannot see other tenants' data
- **You never have direct database access to any platform module** — REST APIs only
- **Your own operational database** (pickups, GPS, routes, domain-specific data) is yours to own, provision, and migrate

### Two Databases: Yours and Theirs

| Data | Owner | Access |
|------|-------|--------|
| Customers, vendors, contacts | Party Master | `POST /api/party/companies` etc. |
| Invoices, AR aging | AR module | `POST /api/ar/invoices` etc. |
| Subscriptions | Subscriptions module | `POST /api/subscriptions/subscriptions` etc. |
| Payments | Payments module | API |
| Auth tokens, RBAC | identity-auth | `POST /api/auth/login` etc. |
| Audit trail | Audit service | Written automatically |
| Your domain data (jobs, GPS, etc.) | **Your Postgres** | Your own sqlx/migrations/repo |

### The AR Two-Step (Non-Obvious)

AR invoices are **not** created directly from a `party_id`. There is a mandatory two-step flow:

```
Step 1: POST /api/ar/customers  →  get ar_customer_id (integer)
Step 2: POST /api/ar/invoices   →  uses ar_customer_id (integer), not party_id
```

`party_id` (UUID from Party Master) is an optional cross-reference field on both the AR customer and the invoice. It is **not** the primary key the AR module uses internally. You must create an AR customer first.

### Command Flow Pattern (Guard → Mutation → Outbox)

All command handlers follow this pattern:

```
1. Guard:    Validate invariants, auth, permissions
2. Mutation: Apply domain change atomically (in a DB transaction)
3. Outbox:   Write event to outbox table IN THE SAME TRANSACTION
```

The outbox insert and domain mutation MUST be atomic. Use `enqueue_event_tx()` (transactional), never `enqueue_event()` (deprecated, non-transactional).

Source: `modules/ar/src/events/outbox.rs`

---

## OWNERSHIP BOUNDARY

This table defines what your vertical app agents CAN edit vs what requires 7D Platform agents to change.

### Your App Agents CAN Edit (your repo)

| What | Where | Notes |
|------|-------|-------|
| Domain models (pickups, GPS, routes, evidence) | `modules/trashtech/` | Your Postgres, your migrations |
| Domain HTTP handlers | `modules/trashtech/src/http/` | Your Axum routes |
| Domain event types (payload structs) | `modules/trashtech/src/events/` | Define your own event payloads |
| Outbox table + publisher | `modules/trashtech/src/events/` | Copy AR's pattern (see below) |
| Cargo.toml path deps to platform crates | `modules/trashtech/Cargo.toml` | `event-bus`, `security` |
| Docker Compose for your service | Your compose file | Your HTTP port, your PG port |
| Your DB migrations | `modules/trashtech/db/migrations/` | sqlx migrate |

### Requires 7D Platform Agents to Change (platform repo)

| What | Where | Why |
|------|-------|-----|
| Add permission strings (`trashtech.mutate`, `trashtech.read`) | `platform/security/src/permissions.rs` | Central permission registry |
| Register new NATS subjects | Platform event-bus config | Subject ACLs |
| Add new E2E test files | `e2e-tests/tests/` | Platform-wide test suite |
| Modify any platform module code | `platform/*/`, `modules/party/`, `modules/ar/`, etc. | Not your code |
| Provision tenants | Admin tooling | No public API |
| Add new EventEnvelope fields | `platform/event-bus/src/envelope/` | Shared struct |

### Shared Platform Crates (Read-Only Dependencies)

These crates live in the platform repo. You import them via path dependencies. Never modify them.

| Crate | Path | What you use |
|-------|------|-------------|
| `event-bus` | `platform/event-bus/` | `EventEnvelope<T>`, `MerchantContext`, `outbox::validate_and_serialize_envelope()` |
| `security` | `platform/security/` | `JwtVerifier`, `VerifiedClaims`, `ActorType`, `ClaimsLayer`, `RequirePermissionsLayer`, permission constants |

---

## Bootstrap Checklist

Before writing any application code, verify this sequence completes cleanly. If any step fails, stop — everything downstream depends on it.

```
1. Receive from BrightHill/orchestrator:
   - tenant_id: UUID (your tenant's identity)
   - app_id: string (your product slug, e.g. "trashtech-pro")
   Both come from environment variables in production: TENANT_ID, APP_ID

2. Confirm tenant is active:
   GET http://7d-tenant-registry/api/tenants/{tenant_id}/status
   → { "tenant_id": "<uuid>", "status": "active" }
   If status ≠ "active" → stop. Tenant is not provisioned.

3. Confirm platform modules are ready (run for each module you need):
   GET http://7d-auth-lb:8080/api/ready   → "ok"
   GET http://7d-party:8098/api/ready     → "ok"
   GET http://7d-ar:8086/api/ready        → "ok"

4. Create a test user and login:
   POST http://7d-auth-lb:8080/api/auth/register
   Body: { tenant_id, user_id: <new-uuid>, email, password }
   → { "ok": true }

   POST http://7d-auth-lb:8080/api/auth/login
   Body: { tenant_id, email, password }
   → { "access_token": "<jwt>", ... }

5. Verify headers are correct — make one real query:
   GET http://7d-party:8098/api/party/parties
   Headers: x-app-id: trashtech-pro, Authorization: Bearer <jwt>, x-correlation-id: <uuid>
   → 200 OK with empty list (not 400, not 401)

   If you get 400: x-app-id is wrong or missing.
   If you get 401: JWT is invalid or expired.
   If you get 200 with empty list: headers are correct.
```

---

## Module Reference

Source: `docker-compose.yml` and each module's `main.rs`.

| Module | HTTP Port | Postgres Port | Container |
|--------|-----------|---------------|-----------|
| identity-auth (behind nginx LB) | **8080** | 5433 | 7d-auth-lb |
| AR | **8086** | 5434 | 7d-ar |
| Subscriptions | **8087** | 5435 | 7d-subscriptions |
| Payments | **8088** | 5436 | 7d-payments |
| Notifications | **8089** | 5437 | 7d-notifications |
| GL | **8090** | 5438 | 7d-gl |
| Inventory | **8092** | 5442 | 7d-inventory |
| AP | **8093** | 5443 | 7d-ap |
| Treasury | **8094** | 5444 | 7d-treasury |
| Fixed Assets | **8095** | 5445 | 7d-fixed-assets |
| Consolidation | **8096** | 5446 | 7d-consolidation |
| Timekeeping | **8097** | 5447 | 7d-timekeeping |
| Party Master | **8098** | 5448 | 7d-party |
| Integrations | **8099** | 5449 | 7d-integrations |
| TTP (platform billing) | **8100** | 5450 | 7d-ttp |

**Standard endpoints on every module:**
```
GET /api/health   — liveness
GET /api/ready    — readiness
GET /metrics      — Prometheus
```

---

## Required HTTP Headers

Source: `modules/party/src/http/party.rs` (enforced pattern across all modules)

```
x-app-id: trashtech-pro           # REQUIRED — wrong or missing = 400 or empty results
x-tenant-id: <tenant-uuid>        # REQUIRED where enforced by auth middleware
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
  "email": "driver@trashtech.com",
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
  "email": "driver@trashtech.com",
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

```
POST /api/auth/refresh
```

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

```rust
use security::authz_middleware::{ClaimsLayer, RequirePermissionsLayer, optional_claims_mw};
use security::claims::JwtVerifier;
use std::sync::Arc;

let verifier: Option<Arc<JwtVerifier>> = JwtVerifier::from_env().map(Arc::new);

let app = Router::new()
    // Mutation routes — require specific permissions
    .route("/api/trashtech/pickups", post(create_pickup))
    .route_layer(RequirePermissionsLayer::new(&["trashtech.mutate"]))
    // Read routes — no permission guard (or use trashtech.read)
    .route("/api/trashtech/pickups", get(list_pickups))
    .route("/api/trashtech/pickups/{id}", get(get_pickup))
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
- `RequirePermissionsLayer::new(&["trashtech.mutate"])` — checks `VerifiedClaims.perms` contains all listed strings. Returns `403` if missing, `401` if no claims at all.

### Accessing Claims in Handlers

```rust
use security::claims::VerifiedClaims;
use axum::Extension;

async fn create_pickup(
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
fixed_assets.mutate, fixed_assets.read, trashtech.mutate, trashtech.read
```

---

## Party Master

Source: `modules/party/src/http/party.rs`, `modules/party/src/domain/party/models.rs`

**Base URL:** `http://7d-party:8098`

`party_id` is the universal cross-module counterparty key. Create a party before creating AR customers, AP vendors, or any other counterparty record.

### Create a Company

```
POST /api/party/companies
x-app-id: trashtech-pro
Content-Type: application/json
```

Request body:
```json
{
  "display_name": "Acme Waste Management",
  "legal_name": "Acme Waste Management LLC",
  "trade_name": "Acme Waste",
  "registration_number": "TX-12345678",
  "tax_id": "12-3456789",
  "country_of_incorporation": "US",
  "industry_code": "5621",
  "email": "billing@acme.com",
  "phone": "+15551234567",
  "website": "https://acme.com",
  "address_line1": "123 Main St",
  "address_line2": "Suite 400",
  "city": "Austin",
  "state": "TX",
  "postal_code": "78701",
  "country": "US",
  "metadata": { "crm_id": "acme-001" }
}
```

Required fields: **`display_name`**, **`legal_name`**. All others are optional.

Response `201 Created` → `PartyView`:
```json
{
  "id": "a1b2c3d4-e5f6-7890-abcd-ef1234567890",
  "app_id": "trashtech-pro",
  "party_type": "company",
  "status": "active",
  "display_name": "Acme Waste Management",
  "email": "billing@acme.com",
  "phone": "+15551234567",
  "website": "https://acme.com",
  "address_line1": "123 Main St",
  "city": "Austin",
  "state": "TX",
  "postal_code": "78701",
  "country": "US",
  "metadata": { "crm_id": "acme-001" },
  "created_at": "2026-02-19T10:00:00Z",
  "updated_at": "2026-02-19T10:00:00Z",
  "legal_name": "Acme Waste Management LLC",
  "tax_id": "12-3456789",
  "external_refs": []
}
```

**The `id` field is your `party_id`. Store it everywhere.**

### Create an Individual

```
POST /api/party/individuals
x-app-id: trashtech-pro
Content-Type: application/json
```

Request body:
```json
{
  "display_name": "John Smith",
  "first_name": "John",
  "last_name": "Smith",
  "middle_name": "A",
  "email": "john@example.com",
  "phone": "+15559876543",
  "job_title": "Driver",
  "department": "Operations",
  "metadata": { "employee_id": "D-0042" }
}
```

Required fields: **`display_name`**, **`first_name`**, **`last_name`**. All others optional.

Response: Same `PartyView` shape, `party_type` = `"individual"`.

### Get a Party

```
GET /api/party/parties/{party_id}
x-app-id: trashtech-pro
```

Response: `PartyView` (200) or `{ "error": "not_found", "message": "..." }` (404).

### Search Parties

```
GET /api/party/parties/search?name=Acme&party_type=company&limit=20&offset=0
x-app-id: trashtech-pro
```

Query parameters (all optional): `name` (partial match), `party_type` (`company`|`individual`|`contact`), `status` (`active`|`inactive`), `external_system`, `external_id`, `limit` (default 50, max 200), `offset`.

### List Parties

```
GET /api/party/parties
x-app-id: trashtech-pro
```

### Update a Party

```
PUT /api/party/parties/{party_id}
x-app-id: trashtech-pro
Content-Type: application/json
```

Send only the fields you want to update.

### Deactivate a Party

```
POST /api/party/parties/{party_id}/deactivate
x-app-id: trashtech-pro
```

Response: `204 No Content`.

---

## AR Module — Customers and Invoices

Source: `modules/ar/src/models/customer.rs`, `modules/ar/src/models/invoice.rs`, `modules/ar/src/routes/customers.rs`

**Base URL:** `http://7d-ar:8086`

### Step 1: Create an AR Customer

**You must do this before creating invoices.**

```
POST /api/ar/customers
x-app-id: trashtech-pro
Content-Type: application/json
```

Request body:
```json
{
  "email": "billing@acme.com",
  "name": "Acme Waste Management",
  "external_customer_id": "crm-acme-001",
  "party_id": "a1b2c3d4-e5f6-7890-abcd-ef1234567890",
  "metadata": { "segment": "enterprise" }
}
```

Field notes:
- `email`: **Effectively required** — the create handler validates it is non-empty and contains `@`
- `name`: Optional display name
- `external_customer_id`: Optional — your internal ID for this customer (useful for lookup)
- `party_id`: Optional UUID — link to Party Master record

Response `201 Created`:
```json
{
  "id": 42,
  "app_id": "trashtech-pro",
  "email": "billing@acme.com",
  "name": "Acme Waste Management",
  "external_customer_id": "crm-acme-001",
  "status": "active",
  "party_id": "a1b2c3d4-e5f6-7890-abcd-ef1234567890",
  "tilled_customer_id": null,
  "default_payment_method_id": null,
  "payment_method_type": null,
  "metadata": { "segment": "enterprise" },
  "retry_attempt_count": 0,
  "created_at": "2026-02-19T10:00:00Z",
  "updated_at": "2026-02-19T10:00:00Z"
}
```

**The `id` field (integer) is your `ar_customer_id`. You need this for invoice creation.**

Error cases:
- `400 Bad Request`: email missing or invalid
- `409 Conflict`: email already exists for this `app_id`

### Step 2: Create an Invoice

```
POST /api/ar/invoices
x-app-id: trashtech-pro
Content-Type: application/json
```

Request body:
```json
{
  "ar_customer_id": 42,
  "amount_cents": 15000,
  "currency": "usd",
  "due_at": "2026-03-19T00:00:00",
  "party_id": "a1b2c3d4-e5f6-7890-abcd-ef1234567890",
  "billing_period_start": "2026-02-01T00:00:00",
  "billing_period_end": "2026-02-28T23:59:59",
  "line_item_details": [
    { "description": "Trash pickup - 10 stops", "amount_cents": 15000 }
  ],
  "metadata": { "job_id": "job-8812" }
}
```

Required fields: **`ar_customer_id`** (integer from Step 1), **`amount_cents`** (integer, in cents).

Optional fields: `subscription_id`, `status` (default: `"draft"`), `currency` (default: `"usd"`), `due_at`, `billing_period_start`, `billing_period_end`, `line_item_details` (JSON), `compliance_codes` (JSON), `correlation_id`, `party_id`, `metadata`.

Invoice status values: `"draft"` | `"open"` | `"paid"` | `"void"` | `"uncollectible"`

Response: `201 Created` with full invoice object including `id` (integer), `tilled_invoice_id`, `status`, `amount_cents`, etc.

### Lookup AR Customer by External ID

```
GET /api/ar/customers?external_customer_id=crm-acme-001&limit=1
x-app-id: trashtech-pro
```

Use this to check if you already created an AR customer before creating a duplicate.

### Other AR Endpoints

Source: `modules/ar/src/routes/mod.rs`

Mutation routes (require `ar.mutate` permission):
```
POST   /api/ar/invoices/{id}/finalize
POST   /api/ar/invoices/{id}/bill-usage
POST   /api/ar/invoices/{id}/credit-notes
POST   /api/ar/invoices/{id}/write-off
POST   /api/ar/charges
POST   /api/ar/refunds
POST   /api/ar/disputes
POST   /api/ar/payment-methods
POST   /api/ar/webhooks
POST   /api/ar/usage
POST   /api/ar/dunning/run
POST   /api/ar/allocation/run
POST   /api/ar/reconciliation/run
POST   /api/ar/tax-config
```

Read routes:
```
GET    /api/ar/invoices
GET    /api/ar/invoices/{id}
GET    /api/ar/customers
GET    /api/ar/customers/{id}
GET    /api/ar/subscriptions
GET    /api/ar/aging
```

---

## Complete "First Invoice" Flow

This is the canonical sequence for billing a customer for the first time.

```
1. Register user (once per user):
   POST http://7d-auth-lb:8080/api/auth/register
   Body: { tenant_id, user_id, email, password }

2. Login to get JWT:
   POST http://7d-auth-lb:8080/api/auth/login
   Body: { tenant_id, email, password }
   → save access_token, refresh_token

3. Create party in Party Master (once per customer entity):
   POST http://7d-party:8098/api/party/companies
   Headers: x-app-id, Authorization: Bearer <jwt>, x-correlation-id, x-actor-id
   Body: { display_name, legal_name, email, ... }
   → save party_id (UUID)

4. Create AR customer (once per billing relationship):
   POST http://7d-ar:8086/api/ar/customers
   Headers: x-app-id, Authorization: Bearer <jwt>, x-correlation-id, x-actor-id
   Body: { email, name, party_id: <from step 3>, external_customer_id }
   → save ar_customer_id (integer)

5. Create invoice:
   POST http://7d-ar:8086/api/ar/invoices
   Headers: x-app-id, Authorization: Bearer <jwt>, x-correlation-id, x-actor-id
   Body: { ar_customer_id: <from step 4>, amount_cents, party_id: <from step 3>, ... }
   → invoice created
```

**In your operational DB:** Store `party_id` (UUID) and `ar_customer_id` (integer) on your domain tables so you can reference them without re-querying Party Master or AR.

Example schema for your `pickup_jobs` table:
```sql
CREATE TABLE pickup_jobs (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    customer_party_id UUID NOT NULL,     -- Party Master party_id
    ar_customer_id INTEGER,               -- AR module ar_customer_id; NULL until billing relationship established
    status TEXT NOT NULL DEFAULT 'pending',
    scheduled_at TIMESTAMPTZ,
    -- ... your domain fields
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
```

---

## NATS Event Bus

Source: `platform/event-bus/src/envelope/mod.rs`, `modules/ar/src/events/publisher.rs`, `platform/identity-auth/src/auth/handlers.rs`

Platform uses **NATS JetStream** for async events.

### Subject Naming Convention

**Pattern:** `{module}.events.{event-type}`

```
ar.events.invoice.created
ar.events.payment.collection.requested
auth.events.user.registered
auth.events.user.logged_in
gl.events.journal.posted
trashtech.events.pickup.requested      ← your events
trashtech.events.pickup.completed      ← your events
```

Source: AR publisher at `modules/ar/src/events/publisher.rs` line 56: `format!("ar.events.{}", event.event_type)`.
Source: identity-auth at `platform/identity-auth/src/auth/handlers.rs`: publishes to `"auth.events.user.registered"`, `"auth.events.user.logged_in"`.

**Note:** Some older subjects may exist in flat format (e.g. `invoice.issued`). When subscribing, use the exact subject strings. When publishing new events, always use the namespaced `{module}.events.{type}` format.

### EventEnvelope — Canonical Structure (17 Fields)

Source: `platform/event-bus/src/envelope/mod.rs` → `EventEnvelope<T>`

This is the platform-wide event envelope. **Use the `event-bus` crate — do not reimplement.**

```rust
pub struct EventEnvelope<T> {
    pub event_id: Uuid,                           // Auto-generated. Idempotency key.
    pub event_type: String,                        // e.g. "pickup.requested"
    pub occurred_at: DateTime<Utc>,                // Auto-generated.
    pub tenant_id: String,                         // Multi-tenant isolation.
    pub source_module: String,                     // e.g. "trashtech"
    pub source_version: String,                    // Default "1.0.0". Use CARGO_PKG_VERSION.
    pub schema_version: String,                    // Default "1.0.0".
    pub trace_id: Option<String>,                  // Distributed tracing.
    pub correlation_id: Option<String>,            // Links events in a business transaction.
    pub causation_id: Option<String>,              // What caused this event.
    pub reverses_event_id: Option<Uuid>,           // Compensating transactions.
    pub supersedes_event_id: Option<Uuid>,         // Corrections.
    pub side_effect_id: Option<String>,            // Side-effect idempotency.
    pub replay_safe: bool,                         // Default true.
    pub mutation_class: Option<String>,            // e.g. "financial", "user-data"
    pub actor_id: Option<Uuid>,                    // Who caused this event.
    pub actor_type: Option<String>,                // "user", "service", "system"
    pub merchant_context: Option<MerchantContext>, // Money-mixing guard. Required for financial.
    pub payload: T,                                // Your event-specific data.
}
```

### Creating an Envelope

```rust
use event_bus::{EventEnvelope, MerchantContext};

// Basic construction
let envelope = EventEnvelope::new(
    tenant_id.to_string(),       // tenant_id
    "trashtech".to_string(),     // source_module
    "pickup.requested".to_string(), // event_type
    payload,                     // your struct implementing Serialize
);

// With builder methods
let envelope = EventEnvelope::new(tenant_id, "trashtech".into(), "pickup.requested".into(), payload)
    .with_source_version(env!("CARGO_PKG_VERSION").to_string())
    .with_correlation_id(Some(correlation_id))
    .with_causation_id(Some(causation_id))
    .with_mutation_class(Some("operational".to_string()))
    .with_actor(user_id, "user".to_string())
    .with_merchant_context(Some(MerchantContext::Tenant(tenant_id.to_string())));
```

Builder methods available (all return `Self`):
```
.with_source_version(String)
.with_schema_version(String)
.with_trace_id(Option<String>)
.with_correlation_id(Option<String>)
.with_causation_id(Option<String>)
.with_reverses_event_id(Option<Uuid>)
.with_supersedes_event_id(Option<Uuid>)
.with_side_effect_id(Option<String>)
.with_replay_safe(bool)
.with_mutation_class(Option<String>)
.with_actor(Uuid, String)
.with_actor_from(Option<Uuid>, Option<String>)
.with_merchant_context(Option<MerchantContext>)
.with_tracing_context(&TracingContext)
```

### MerchantContext

Source: `platform/event-bus/src/envelope/mod.rs`

```rust
#[serde(tag = "type", content = "id")]
pub enum MerchantContext {
    Tenant(String),  // Your events. Inner value = tenant_id.
    Platform,        // 7D internal. NEVER use this.
}
```

Serialized JSON:
```json
{ "type": "Tenant", "id": "550e8400-..." }
```

**For TrashTech domain events: always use `MerchantContext::Tenant(tenant_id)`.** The `Platform` variant is reserved for 7D internal billing operations (e.g. when the platform invoices a tenant for its own SaaS fees). TrashTech events are never platform-of-record transactions.

Rule: `merchant_context` must match the merchant of record for the transaction. TrashTech charges customers → `Tenant`. 7D charges TrashTech Pro → `Platform` (but you never emit those events).

Required for financial events (invoicing, payments). Optional for non-financial (GPS pings, route updates).

### Idempotency

All events are deduplicated by `event_id`. Your consumer must check and skip already-processed `event_id` values using a `processed_events` table.

### Known NATS Subjects

| Subject | Published by | Trigger |
|---------|-------------|---------|
| `auth.events.user.registered` | identity-auth | User registered |
| `auth.events.user.logged_in` | identity-auth | Successful login |
| `ar.events.invoice.created` | AR | Invoice created |
| `ar.events.payment.collection.requested` | AR | Collection triggered |
| `gl.events.journal.posted` | AR (cross-module) | GL posting |
| `payments.events.payment.succeeded` | Payments | Payment gateway success |

### Event Evolution Rules

1. Never remove fields from event payloads
2. Only add fields with safe defaults
3. Breaking change → emit new event type OR bump `schema_version`
4. Consumers must handle older schema versions until cutover

---

## Outbox Pattern — Copy This

Source: `modules/ar/db/migrations/20260211000001_create_events_outbox.sql`, `modules/ar/db/migrations/20260216000001_add_envelope_metadata_to_outbox.sql`

### Migration: Create Outbox Tables

Copy this into your first migration for TrashTech:

```sql
-- events_outbox: Transactional outbox for reliable event publishing
CREATE TABLE events_outbox (
    id SERIAL PRIMARY KEY,
    event_id UUID NOT NULL UNIQUE,
    event_type VARCHAR(255) NOT NULL,
    aggregate_type VARCHAR(100) NOT NULL,
    aggregate_id VARCHAR(255) NOT NULL,
    payload JSONB NOT NULL,
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    published_at TIMESTAMP,
    -- Envelope metadata (all from EventEnvelope)
    tenant_id VARCHAR(255),
    source_module VARCHAR(100),
    source_version VARCHAR(50),
    schema_version VARCHAR(50),
    occurred_at TIMESTAMPTZ,
    replay_safe BOOLEAN DEFAULT true,
    trace_id VARCHAR(255),
    correlation_id VARCHAR(255),
    causation_id VARCHAR(255),
    reverses_event_id UUID,
    supersedes_event_id UUID,
    side_effect_id VARCHAR(255),
    mutation_class VARCHAR(100)
);

-- Index for unpublished events (background publisher polls this)
CREATE INDEX idx_events_outbox_unpublished ON events_outbox (created_at)
WHERE published_at IS NULL;

-- Index for cleanup queries
CREATE INDEX idx_events_outbox_published ON events_outbox (published_at)
WHERE published_at IS NOT NULL;

-- Index for tenant-scoped queries
CREATE INDEX idx_events_outbox_tenant_id ON events_outbox(tenant_id)
WHERE tenant_id IS NOT NULL;

-- Index for distributed tracing
CREATE INDEX idx_events_outbox_trace_id ON events_outbox(trace_id)
WHERE trace_id IS NOT NULL;

-- processed_events: Idempotent consumer dedup
CREATE TABLE processed_events (
    id SERIAL PRIMARY KEY,
    event_id UUID NOT NULL UNIQUE,
    event_type VARCHAR(255) NOT NULL,
    processed_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    processor VARCHAR(100) NOT NULL
);

CREATE INDEX idx_processed_events_event_id ON processed_events (event_id);
```

### Outbox Enqueue (Transactional)

Source: `modules/ar/src/events/outbox.rs` → `enqueue_event_tx()`

```rust
use event_bus::outbox::validate_and_serialize_envelope;

pub async fn enqueue_event_tx<T: Serialize>(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    event_type: &str,          // e.g. "pickup.requested"
    aggregate_type: &str,      // e.g. "pickup"
    aggregate_id: &str,        // e.g. pickup UUID
    envelope: &EventEnvelope<T>,
) -> Result<(), sqlx::Error> {
    let payload = validate_and_serialize_envelope(envelope)
        .map_err(|e| sqlx::Error::Encode(Box::new(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("Envelope validation failed: {}", e),
        ))))?;

    sqlx::query(
        r#"INSERT INTO events_outbox (
            event_id, event_type, aggregate_type, aggregate_id, payload,
            tenant_id, source_module, source_version, schema_version,
            occurred_at, replay_safe, trace_id, correlation_id, causation_id,
            reverses_event_id, supersedes_event_id, side_effect_id, mutation_class
        ) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$18)"#,
    )
    .bind(envelope.event_id)
    .bind(event_type)
    .bind(aggregate_type)
    .bind(aggregate_id)
    .bind(payload)
    .bind(&envelope.tenant_id)
    .bind(&envelope.source_module)
    .bind(&envelope.source_version)
    .bind(&envelope.schema_version)
    .bind(envelope.occurred_at)
    .bind(envelope.replay_safe)
    .bind(&envelope.trace_id)
    .bind(&envelope.correlation_id)
    .bind(&envelope.causation_id)
    .bind(&envelope.reverses_event_id)
    .bind(&envelope.supersedes_event_id)
    .bind(&envelope.side_effect_id)
    .bind(&envelope.mutation_class)
    .execute(&mut **tx)
    .await?;

    Ok(())
}
```

### Background Publisher

Source: `modules/ar/src/events/publisher.rs`

Polls `events_outbox` every 1 second. For each unpublished event:
1. Build NATS subject: `format!("{module}.events.{event_type}")`
2. Serialize payload to bytes
3. Publish via `event_bus::EventBus` trait
4. Mark as published (`UPDATE events_outbox SET published_at = NOW() WHERE event_id = $1`)

Copy this pattern for TrashTech. Subject routing:
```rust
let subject = format!("trashtech.events.{}", event.event_type);
```

---

## Integrations Module

Source: `modules/integrations/src/`

**Base URL:** `http://7d-integrations:8099`

### Inbound Webhooks (External → Platform)

```
POST /api/integrations/webhooks/inbound
x-app-id: trashtech-pro
Content-Type: application/json
```

Use for routing external system events (GPS provider webhooks, payment gateway callbacks) into the platform event bus.

### External ID Mapping

Map your internal IDs to external system IDs:

```
POST /api/integrations/external-refs
GET /api/integrations/external-refs/by-external?system=stripe&external_id=cus_12345
```

---

## Tenant Provisioning

Tenant provisioning is an **internal admin process** — there is no public API endpoint for creating tenants.

**Flow:**
1. Contact BrightHill (orchestrator) to provision your tenant
2. BrightHill creates the tenant record via admin tools
3. You receive: `tenant_id` (UUID) + `app_id` (string, e.g. `trashtech-pro`)
4. Provisioning states: `pending` → `provisioning` → `active` | `failed`
5. Only `active` tenants can log in — identity-auth enforces this at login time

**After provisioning:**
```
GET http://7d-tenant-registry/api/tenants/{tenant_id}/status
→ { "tenant_id": "<uuid>", "status": "active" }
```

---

## Data Ownership — Decision Table

When writing code, use this table to decide where data lives:

| Question | If YES → | If NO → |
|----------|----------|---------|
| Is it a person, company, or contact? | Party Master | Your DB |
| Is it a receivable from your customer? | AR module | — |
| Is it a payable to a vendor? | AP module | — |
| Is it a recurring billing plan? | Subscriptions | — |
| Is it actual money movement? | Payments | — |
| Is it financial journal entries? | GL (via events) | — |
| Is it operational (pickups, GPS, routes, evidence)? | Your DB | — |
| Is it user authentication? | identity-auth | — |

**Rule:** Never duplicate data that belongs to a platform module in your own DB. Store the platform's IDs (`party_id`, `ar_customer_id`) as foreign references.

### Billing Separation (Non-Negotiable)

- Tenant Platform and TrashTech Pro are **independent billing contracts**
- Separate `product_code`s: `tenant_platform`, `trashtech_pro`
- TrashTech cannot silently cause platform billing to begin
- All TrashTech financial events use `MerchantContext::Tenant(tenant_id)`, never `Platform`

---

## Environment Variables

Required env vars for any module that uses platform crates. Set these in your `docker-compose.yml` and test scripts.

```bash
# Your module's Postgres connection
DATABASE_URL=postgres://postgres:postgres@trashtech-postgres:5432/trashtech_db

# NATS event bus (JetStream enabled)
NATS_URL=nats://nats:4222

# JWT public key for RS256 verification (from identity-auth)
# In Docker: read from volume or env. In tests: set to test key.
JWT_PUBLIC_KEY="-----BEGIN PUBLIC KEY-----\n...\n-----END PUBLIC KEY-----"

# Your service's HTTP bind address
LISTEN_ADDR=0.0.0.0
LISTEN_PORT=8101   # pick an unused port

# Your tenant identity (set by orchestrator during provisioning)
TENANT_ID=550e8400-e29b-41d4-a716-446655440000
APP_ID=trashtech-pro

# Platform module base URLs (use container names in Docker, localhost in dev)
PARTY_BASE_URL=http://7d-party:8098
AR_BASE_URL=http://7d-ar:8086
AUTH_BASE_URL=http://7d-auth-lb:8080

# Log level
RUST_LOG=info
```

**JwtVerifier::from_env() reads `JWT_PUBLIC_KEY`.** If that env var is absent, `from_env()` returns `None` (dev mode — auth bypassed). In production, always set it. In E2E tests against real platform, set it to the actual platform public key (read from identity-auth JWKS or platform ops team).

---

## Cargo.toml Path Dependencies

When your vertical app needs platform crates, add these to your `Cargo.toml`:

```toml
[dependencies]
# Platform crates (path dependencies — adjust relative path based on your module location)
# If your module is at modules/trashtech/:
event-bus = { path = "../../platform/event-bus" }
security = { path = "../../platform/security" }

# Common dependencies matching platform versions
axum = "0.8"
tokio = { version = "1", features = ["full"] }
sqlx = { version = "0.8", features = ["runtime-tokio", "tls-rustls", "postgres", "uuid", "chrono", "json"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
uuid = { version = "1", features = ["serde", "v4"] }
chrono = { version = "0.4", features = ["serde"] }
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter", "json"] }
tower = "0.5"
http = "1"
```

Source: `platform/security/Cargo.toml` confirms `event-bus = { path = "../event-bus" }` pattern.

---

## Do's and Don'ts

### DO

- Create a party in Party Master before creating AR customers or AP vendors — you need `party_id` first
- Create an AR customer before creating an invoice — you need `ar_customer_id` first
- Store `party_id` and `ar_customer_id` in your operational tables so you can join without re-querying
- Include all 5 required headers on every API call
- Use `x-correlation-id` (generate a UUID per request) for distributed tracing
- Deduplicate NATS events by `event_id`
- Use identity-auth for all user auth — validate JWT via `security` crate, not custom code
- Use `MerchantContext::Tenant(tenant_id)` on all events you publish
- Use `enqueue_event_tx()` for outbox writes — MUST be in the same transaction as domain mutation
- Check `e2e-tests/tests/` for working code patterns before implementing from scratch
- Use `env!("CARGO_PKG_VERSION")` for `source_version` in event envelopes

### DON'T

- Never connect directly to platform Postgres databases — REST APIs only
- Never store customer/vendor entity data in your own tables — that's Party Master's job
- Never build your own billing logic — AR + Subscriptions + Payments handle it
- Never build your own JWT issuance or validation — identity-auth issues, `security` crate validates
- Never use `MerchantContext::Platform` — reserved for 7D internal operations
- Never omit `x-app-id` — you will get empty results or 400 with no other error indication
- Never skip `x-correlation-id` — audit trail gaps are hard to debug
- Never mock platform services in tests — tests must call real services
- Never use `enqueue_event()` (non-transactional, deprecated) — use `enqueue_event_tx()` always
- Never modify platform crates (`event-bus`, `security`) — request changes from 7D Platform agents

---

## Local Development

To run platform services locally for integration tests:

```bash
# From the 7D Solutions Platform repo root
docker compose up -d

# Wait for services to be healthy
docker compose ps   # all should show "healthy" or "running"

# Verify key services are ready
curl http://localhost:8080/api/ready  # identity-auth
curl http://localhost:8086/api/ready  # AR
curl http://localhost:8098/api/ready  # Party Master

# Run a specific E2E test
AUDIT_DATABASE_URL=postgres://postgres:postgres@localhost:5432/audit_db \
PROJECTIONS_DATABASE_URL=postgres://postgres:postgres@localhost:5432/projections_db \
TENANT_REGISTRY_DATABASE_URL=postgres://postgres:postgres@localhost:5432/tenant_registry_db \
./scripts/cargo-slot.sh test -p e2e-tests -- party_master_e2e --nocapture
```

Platform services bind to localhost in development:
- identity-auth: localhost:8080
- AR: localhost:8086
- Party Master: localhost:8098

In Docker Compose networking (service-to-service), use container names: `7d-auth-lb`, `7d-ar`, `7d-party`.

---

## Reference E2E Tests

Copy patterns from these files in `e2e-tests/tests/`:

| Test file | What it shows |
|-----------|---------------|
| `party_master_e2e.rs` | Full Party CRUD: create company, get, update, deactivate, search |
| `party_ar_link.rs` | Create party → create AR customer with party_id → verify |
| `ap_vendor_party_link_e2e.rs` | Create party → create AP vendor with party_id |
| `cross_module_invoice_payment_e2e.rs` | Invoice → payment → status update full cycle |
| `cross_module_subscription_invoice_e2e.rs` | Subscription → auto-invoice generation |
| `integrations_integration.rs` | Inbound webhook → external ref mapping |
| `subscriptions_lifecycle.rs` | Subscription state machine transitions |
| `provisioning_full_lifecycle_e2e.rs` | Tenant provisioning end-to-end |
| `rbac_enforcement.rs` | RBAC permission enforcement patterns |
| `treasury_forecast_e2e.rs` | Cash forecast from AR/AP data |

---

## Source File Index

For re-verification or deeper reading:

| Topic | Source file |
|-------|-------------|
| Auth endpoints | `platform/identity-auth/src/routes/auth.rs` |
| JWT claims structure | `platform/identity-auth/src/auth/jwt.rs` → `AccessClaims` |
| JWKS endpoint | `platform/identity-auth/src/main.rs` (mounted at `/.well-known/jwks.json`) |
| JWT verification | `platform/security/src/claims.rs` → `JwtVerifier`, `VerifiedClaims` |
| Auth middleware | `platform/security/src/authz_middleware.rs` → `ClaimsLayer`, `RequirePermissionsLayer` |
| Permission constants | `platform/security/src/permissions.rs` |
| Security crate Cargo.toml | `platform/security/Cargo.toml` |
| EventEnvelope (canonical) | `platform/event-bus/src/envelope/mod.rs` |
| EventEnvelope builder | `platform/event-bus/src/envelope/builder.rs` |
| EventEnvelope validation | `platform/event-bus/src/envelope/validation.rs` |
| MerchantContext enum | `platform/event-bus/src/envelope/mod.rs` |
| TracingContext | `platform/event-bus/src/envelope/tracing_context.rs` |
| Party Master endpoints | `modules/party/src/http/party.rs` |
| Party Master router | `modules/party/src/http/mod.rs` |
| Party Master models | `modules/party/src/domain/party/models.rs` |
| AR customer model | `modules/ar/src/models/customer.rs` |
| AR invoice model | `modules/ar/src/models/invoice.rs` |
| AR router (all routes) | `modules/ar/src/routes/mod.rs` |
| AR customer endpoints | `modules/ar/src/routes/customers.rs` |
| AR envelope helper | `modules/ar/src/events/envelope.rs` |
| AR outbox functions | `modules/ar/src/events/outbox.rs` |
| AR publisher | `modules/ar/src/events/publisher.rs` |
| AR outbox migration | `modules/ar/db/migrations/20260211000001_create_events_outbox.sql` |
| AR outbox metadata migration | `modules/ar/db/migrations/20260216000001_add_envelope_metadata_to_outbox.sql` |
| NATS subjects (AR) | `modules/ar/src/events/publisher.rs` line 51-57 |
| NATS subjects (auth) | `platform/identity-auth/src/auth/handlers.rs` |
| Tenant status endpoint | `platform/tenant-registry/src/routes.rs` |
| Tenant lifecycle states | `platform/tenant-registry/src/lifecycle.rs` |

---

*Maintained by BrightHill (7D Platform Orchestrator). Update when platform APIs change and include source file reference for each change.*
