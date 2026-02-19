# 7D Solutions Platform — Consumer Guide for Claude Code Agents

**Audience:** Claude Code agents building vertical applications (TrashTech Pro, etc.) on the 7D Platform.
**Purpose:** Complete, source-verified API reference. Every fact here is checked against actual Rust source code. Use this instead of reading the platform codebase.

> **All data in this file is verified against source.** File references included so you can re-verify.
> Last verified: 2026-02-19 against commit 474294cb.

---

## CRITICAL CONCEPTS (Read Before Anything Else)

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

### The AR Two-Step (Non-Obvious — Read Carefully)

AR invoices are **not** created directly from a `party_id`. There is a mandatory two-step flow:

```
Step 1: POST /api/ar/customers  →  get ar_customer_id (integer)
Step 2: POST /api/ar/invoices   →  uses ar_customer_id (integer), not party_id
```

`party_id` (UUID from Party Master) is an optional cross-reference field on both the AR customer and the invoice. It is **not** the primary key the AR module uses internally. You must create an AR customer first.

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
  "app_id": "trashtech-pro",
  "roles": ["operator", "driver"],
  "perms": ["ar.create", "ar.read"],
  "actor_type": "user",
  "ver": "1"
}
```

Field notes:
- `sub` = `user_id` UUID string — use as `x-actor-id` in downstream calls
- `actor_type` values: `"user"` | `"service"` | `"system"`
- `app_id` in the JWT is a UUID string (not a short string like "trashtech-pro") — currently always `null` in issued tokens; do not rely on it
- `x-app-id` header (e.g. `trashtech-pro`) is separate from the JWT `app_id` claim — always send the header explicitly
- `ver` = `"1"` (current schema version)
- For service-to-service calls: obtain a service account token with `actor_type: "service"`
- Use the `security` crate's `JwtVerifier` to validate tokens — never decode manually

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
    ar_customer_id INTEGER NOT NULL,      -- AR module ar_customer_id
    status TEXT NOT NULL DEFAULT 'pending',
    scheduled_at TIMESTAMPTZ,
    -- ... your domain fields
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
```

---

## NATS Event Bus

Source: `platform/event-bus/src/envelope/mod.rs`

Platform uses **NATS JetStream** for async events.

### EventEnvelope Structure

Source: `platform/event-bus/src/envelope/mod.rs` → `EventEnvelope<T>` (the canonical envelope — 17 fields)

> **Note:** identity-auth has a LEGACY EventEnvelope (11 fields, different field names). Do NOT use that as a reference. The canonical one is in the `event-bus` crate.

All cross-module events use this envelope structure:
```json
{
  "event_id": "<uuid>",
  "event_type": "invoice.created",
  "occurred_at": "2026-02-19T10:00:00Z",
  "tenant_id": "<tenant-uuid>",
  "source_module": "ar",
  "source_version": "1.0.0",
  "schema_version": "1.0.0",
  "replay_safe": true,
  "payload": { ... },

  "merchant_context": { "type": "Tenant", "id": "<tenant-uuid>" },

  "trace_id": "<uuid>",
  "correlation_id": "<uuid>",
  "causation_id": "<uuid>",
  "actor_id": "<user-uuid>",
  "actor_type": "user",
  "mutation_class": "financial",

  "reverses_event_id": null,
  "supersedes_event_id": null,
  "side_effect_id": null
}
```

Fields with `skip_serializing_if = "Option::is_none"` are omitted when null: `trace_id`, `correlation_id`, `causation_id`, `reverses_event_id`, `supersedes_event_id`, `side_effect_id`, `actor_id`, `actor_type`, `mutation_class`, `merchant_context`.

### merchant_context Serialization

Source: `platform/event-bus/src/envelope/mod.rs` → `MerchantContext` enum with `#[serde(tag = "type", content = "id")]`

```json
// Tenant context (your product operating):
"merchant_context": { "type": "Tenant", "id": "<tenant-uuid-string>" }

// Platform context (7D billing you — DO NOT USE):
"merchant_context": { "type": "Platform" }
```

**CRITICAL:** `merchant_context` is NOT a string. It is an object with a `type` field. Using `"merchant_context": "TENANT"` will cause deserialization errors. Always use the object form.

For financial events (AR, payments, GL): set `merchant_context` to `{ "type": "Tenant", "id": "<your-tenant-id>" }`.
Non-financial events may omit `merchant_context` entirely.

### Known NATS Subjects

| Subject | Published by | Trigger |
|---------|-------------|---------|
| `auth.events.user.registered` | identity-auth | User registered |
| `auth.events.user.logged_in` | identity-auth | Successful login |
| `invoice.created` | AR | Invoice created |
| `invoice.payment_succeeded` | AR | Payment applied |
| `invoice.payment_failed` | AR | Payment attempt failed |
| `subscription.created` | Subscriptions | Subscription started |
| `subscription.updated` | Subscriptions | Subscription changed |
| `subscription.canceled` | Subscriptions | Subscription ended |
| `payment.collection.requested` | AR | Collection triggered |
| `payments.events.payment.succeeded` | Payments | Payment gateway success |

**Subject naming pattern:** Newer subjects use `{module}.events.{entity}.{verb}`. Older subjects use flat `{entity}.{verb}`. Subscribe to exact subject strings as listed.

**Idempotency:** All events are deduplicated by `event_id`. Your consumer must check and skip already-processed `event_id` values.

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

---

## Do's and Don'ts

### DO

- ✅ Create a party in Party Master before creating AR customers or AP vendors — you need `party_id` first
- ✅ Create an AR customer before creating an invoice — you need `ar_customer_id` first
- ✅ Store `party_id` and `ar_customer_id` in your operational tables so you can join without re-querying
- ✅ Include all 5 required headers on every API call
- ✅ Use `x-correlation-id` (generate a UUID per request) for distributed tracing
- ✅ Deduplicate NATS events by `event_id`
- ✅ Use identity-auth for all user auth — validate JWT signature against JWKS, not shared secret
- ✅ Use `merchant_context: { "type": "Tenant", "id": "<tenant-id>" }` on all financial events you publish
- ✅ Check `e2e-tests/tests/` for working code patterns before implementing from scratch

### DON'T

- ❌ Never connect directly to platform Postgres databases — REST APIs only
- ❌ Never store customer/vendor entity data in your own tables — that's Party Master's job
- ❌ Never build your own billing logic — AR + Subscriptions + Payments handle it
- ❌ Never build your own JWT issuance — identity-auth handles it
- ❌ Never use `merchant_context: { "type": "Platform" }` — reserved for 7D internal operations
- ❌ Never omit `x-app-id` — you will get empty results or 400 with no other error indication
- ❌ Never skip `x-correlation-id` — audit trail gaps are hard to debug
- ❌ Never mock platform services in tests — tests must call real services

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

## Platform Crate Dependencies

Source: `platform/event-bus/`, `platform/security/`

Your vertical app Cargo.toml must include these path dependencies to use platform types:

```toml
[dependencies]
# Canonical EventEnvelope, MerchantContext, EventBus trait
event-bus = { path = "../../platform/event-bus" }

# JwtVerifier, VerifiedClaims, ClaimsLayer, RequirePermissionsLayer, permission constants
security = { path = "../../platform/security" }
```

### event-bus crate

```rust
use event_bus::{EventEnvelope, MerchantContext};

// Build an envelope for a financial event:
let envelope = EventEnvelope::new(
    tenant_id.to_string(),
    "trashtech".to_string(),       // source_module
    "pickup.completed".to_string(), // event_type
    payload,
)
.with_merchant_context(Some(MerchantContext::Tenant(tenant_id.to_string())))
.with_mutation_class(Some("operational".to_string()));
```

### security crate — JWT verification

```rust
use security::{JwtVerifier, VerifiedClaims};

// At startup (once):
let verifier = JwtVerifier::from_env()
    .expect("JWT_PUBLIC_KEY env var required");

// Per-request (extract bearer token from Authorization header):
let claims: VerifiedClaims = verifier.verify(&bearer_token)?;
// claims.user_id: Uuid
// claims.tenant_id: Uuid
// claims.perms: Vec<String>
```

### security crate — RBAC middleware

```rust
use security::{ClaimsLayer, RequirePermissionsLayer};
use security::permissions::{TRASHTECH_MUTATE, TRASHTECH_READ};

let app = Router::new()
    .route("/api/trashtech/jobs", post(create_job))
    .layer(RequirePermissionsLayer::new(
        verifier.clone(),
        vec![TRASHTECH_MUTATE.to_string()],
    ))
    .route("/api/trashtech/jobs", get(list_jobs))
    .layer(ClaimsLayer::new(verifier.clone()));
```

### Permission constants (platform/security/src/permissions.rs)

```rust
// Platform modules:
AR_MUTATE = "ar.mutate"      AR_READ = "ar.read"
PAYMENTS_MUTATE              GL_POST = "gl.post"
SUBSCRIPTIONS_MUTATE         INVENTORY_MUTATE / INVENTORY_READ
AP_MUTATE / AP_READ          TREASURY_MUTATE / TREASURY_READ

// Your product:
TRASHTECH_MUTATE = "trashtech.mutate"
TRASHTECH_READ   = "trashtech.read"
```

---

## Transactional Outbox Pattern

Source: `modules/ar/db/migrations/20260211000001_create_events_outbox.sql`

**Never publish NATS events directly inside a mutation handler.** Use the transactional outbox pattern so events are never lost if NATS is down or the process crashes.

### Required DB Tables (copy into your migrations)

```sql
-- Write events here in the SAME transaction as your domain mutation
CREATE TABLE events_outbox (
    id           SERIAL PRIMARY KEY,
    event_id     UUID NOT NULL UNIQUE,
    event_type   VARCHAR(255) NOT NULL,
    aggregate_type VARCHAR(100) NOT NULL,
    aggregate_id VARCHAR(255) NOT NULL,
    payload      JSONB NOT NULL,
    created_at   TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    published_at TIMESTAMP          -- NULL = not yet published
);

CREATE INDEX idx_events_outbox_unpublished ON events_outbox (created_at)
    WHERE published_at IS NULL;

-- Idempotent consumer: check this before processing any event
CREATE TABLE processed_events (
    id           SERIAL PRIMARY KEY,
    event_id     UUID NOT NULL UNIQUE,
    event_type   VARCHAR(255) NOT NULL,
    processed_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    processor    VARCHAR(100) NOT NULL
);
```

### The Pattern

```
Mutation handler (single transaction):
  1. Write domain record (e.g. INSERT INTO pickup_jobs)
  2. Write to events_outbox (same TX)
  3. COMMIT

Background drainer (separate task, runs every ~100ms):
  4. SELECT * FROM events_outbox WHERE published_at IS NULL ORDER BY created_at LIMIT 100
  5. For each row: publish EventEnvelope to NATS
  6. UPDATE events_outbox SET published_at = NOW() WHERE id = $1

Consumer (your NATS subscriber):
  7. Receive event
  8. Check processed_events: if event_id exists → skip (idempotent)
  9. INSERT INTO processed_events (event_id, event_type, processor)
  10. Process the event
```

**The invariant:** The domain mutation and the outbox write are atomic. Either both succeed or neither does. This guarantees at-least-once delivery without dual-write risk.

---

## Source File Index

For re-verification or deeper reading:

| Topic | Source file |
|-------|-------------|
| Auth endpoints | `platform/identity-auth/src/routes/auth.rs` |
| JWT claims structure | `platform/identity-auth/src/auth/jwt.rs` → `AccessClaims` |
| JWKS endpoint | `platform/identity-auth/src/main.rs` (mounted at `/.well-known/jwks.json`) |
| Party Master endpoints | `modules/party/src/http/party.rs` |
| Party Master models | `modules/party/src/domain/party/models.rs` |
| AR customer model | `modules/ar/src/models/customer.rs` |
| AR invoice model | `modules/ar/src/models/invoice.rs` |
| AR customer endpoints | `modules/ar/src/routes/customers.rs` |
| NATS subjects (AR) | `modules/ar/src/consumer_tasks.rs` |
| Tenant status endpoint | `platform/tenant-registry/src/routes.rs` |
| Tenant lifecycle states | `platform/tenant-registry/src/lifecycle.rs` |
| EventEnvelope structure (canonical) | `platform/event-bus/src/envelope/mod.rs` → `EventEnvelope<T>` |
| MerchantContext serialization | `platform/event-bus/src/envelope/mod.rs` → `MerchantContext` |
| JWT verifier (security crate) | `platform/security/src/claims.rs` → `JwtVerifier`, `VerifiedClaims` |
| Permission constants | `platform/security/src/permissions.rs` |
| Outbox migration (AR reference) | `modules/ar/db/migrations/20260211000001_create_events_outbox.sql` |

---

*Maintained by BrightHill (7D Platform Orchestrator). Update when platform APIs change and include source file reference for each change.*
