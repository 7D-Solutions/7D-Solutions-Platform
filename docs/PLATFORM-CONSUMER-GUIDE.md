# 7D Solutions Platform — Consumer Guide

**Audience:** Claude Code agents building vertical applications (TrashTech Pro, etc.) on the 7D Platform.
**Purpose:** Index + critical concepts. All API details are in the topic files below.

> **All data is verified against source.** File references included so you can re-verify.
> Last verified: 2026-02-19 against commit 474294cb.

---

## Topic Files

| File | What it covers |
|------|---------------|
| [CG-AUTH.md](./CG-AUTH.md) | Required HTTP headers, error format, identity-auth API (register, login, refresh, JWKS), JWT verification, permission strings |
| [CG-MODULE-APIS.md](./CG-MODULE-APIS.md) | Party Master endpoints, AR module (customers + invoices), complete "First Invoice" flow |
| [CG-EVENTS.md](./CG-EVENTS.md) | NATS event bus, EventEnvelope, MerchantContext, outbox pattern (migration + enqueue + publisher), Integrations module |
| [CG-TENANCY.md](./CG-TENANCY.md) | Tenant provisioning, database-per-tenant routing, per-app roles, cross-app navigation, support sessions |
| [CG-REFERENCE.md](./CG-REFERENCE.md) | Environment variables, Cargo.toml path dependencies, local development, reference E2E tests, source file index |

---

## Quick Find — Go Directly to What You Need

| I need to… | Go to |
|------------|-------|
| See all required HTTP headers | [CG-AUTH.md → Required HTTP Headers](./CG-AUTH.md#required-http-headers) |
| Understand the error response format | [CG-AUTH.md → Error Response Format](./CG-AUTH.md#error-response-format) |
| Register a user / log in | [CG-AUTH.md → Authentication (identity-auth)](./CG-AUTH.md#authentication-identity-auth) |
| Refresh a JWT token | [CG-AUTH.md → Refresh Token](./CG-AUTH.md#3-refresh-token) |
| Understand JWT claims fields | [CG-AUTH.md → JWT Claims Structure](./CG-AUTH.md#jwt-claims-structure) |
| Set up JWT verification in my service | [CG-AUTH.md → JWT Verification in Your Service](./CG-AUTH.md#jwt-verification-in-your-service) |
| Wire Axum auth middleware | [CG-AUTH.md → Wiring Into Axum Router](./CG-AUTH.md#wiring-into-axum-router) |
| See available permission strings | [CG-AUTH.md → Permission Strings](./CG-AUTH.md#permission-strings) |
| Create a company or individual (Party Master) | [CG-MODULE-APIS.md → Party Master](./CG-MODULE-APIS.md#party-master) |
| Create an AR customer | [CG-MODULE-APIS.md → Step 1: Create an AR Customer](./CG-MODULE-APIS.md#step-1-create-an-ar-customer) |
| Create an invoice | [CG-MODULE-APIS.md → Step 2: Create an Invoice](./CG-MODULE-APIS.md#step-2-create-an-invoice) |
| See all AR endpoints | [CG-MODULE-APIS.md → Other AR Endpoints](./CG-MODULE-APIS.md#other-ar-endpoints) |
| Follow the end-to-end billing flow | [CG-MODULE-APIS.md → Complete "First Invoice" Flow](./CG-MODULE-APIS.md#complete-first-invoice-flow) |
| Understand NATS subject naming | [CG-EVENTS.md → Subject Naming Convention](./CG-EVENTS.md#subject-naming-convention) |
| See all EventEnvelope fields | [CG-EVENTS.md → EventEnvelope Canonical Structure](./CG-EVENTS.md#eventenvelopecanonical-structure-17-fields) |
| Build an EventEnvelope | [CG-EVENTS.md → Creating an Envelope](./CG-EVENTS.md#creating-an-envelope) |
| Understand MerchantContext | [CG-EVENTS.md → MerchantContext](./CG-EVENTS.md#merchantcontext) |
| Copy the outbox SQL migration | [CG-EVENTS.md → Migration: Create Outbox Tables](./CG-EVENTS.md#migration-create-outbox-tables) |
| Copy the outbox enqueue function | [CG-EVENTS.md → Outbox Enqueue (Transactional)](./CG-EVENTS.md#outbox-enqueue-transactional) |
| Wire the background publisher | [CG-EVENTS.md → Background Publisher](./CG-EVENTS.md#background-publisher) |
| Receive external webhooks / GPS events | [CG-EVENTS.md → Integrations Module](./CG-EVENTS.md#integrations-module) |
| Map internal IDs to external system IDs | [CG-EVENTS.md → External ID Mapping](./CG-EVENTS.md#external-id-mapping) |
| Check tenant provisioning status | [CG-TENANCY.md → Tenant Provisioning](./CG-TENANCY.md#tenant-provisioning) |
| Implement database-per-tenant isolation | [CG-TENANCY.md → Database-Per-Tenant Routing Pattern](./CG-TENANCY.md#database-per-tenant-routing-pattern) |
| Route requests to the correct tenant DB | [CG-TENANCY.md → Axum Middleware Pattern](./CG-TENANCY.md#axum-middleware-pattern) |
| React to `tenant.provisioned` NATS event | [CG-TENANCY.md → Tenant Provisioning — Database Creation](./CG-TENANCY.md#tenant-provisioning--database-creation) |
| Run migrations across tenant DBs | [CG-TENANCY.md → Migration Strategy](./CG-TENANCY.md#migration-strategy) |
| Define per-app permission strings | [CG-TENANCY.md → Per-App Roles and Cross-App Navigation](./CG-TENANCY.md#per-app-roles-and-cross-app-navigation) |
| Understand how TCP launch links work | [CG-TENANCY.md → Cross-App Navigation](./CG-TENANCY.md#cross-app-navigation-the-launch-link) |
| Handle support sessions in my app | [CG-TENANCY.md → Support Sessions — Technical Mechanism](./CG-TENANCY.md#support-sessions--technical-mechanism) |
| Set required environment variables | [CG-REFERENCE.md → Environment Variables](./CG-REFERENCE.md#environment-variables) |
| Add platform crates to Cargo.toml | [CG-REFERENCE.md → Cargo.toml Path Dependencies](./CG-REFERENCE.md#cargotoml-path-dependencies) |
| Run platform services locally | [CG-REFERENCE.md → Local Development](./CG-REFERENCE.md#local-development) |
| Find a reference E2E test to copy | [CG-REFERENCE.md → Reference E2E Tests](./CG-REFERENCE.md#reference-e2e-tests) |
| Find the source file for a specific topic | [CG-REFERENCE.md → Source File Index](./CG-REFERENCE.md#source-file-index) |

---

## Critical Concepts

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

### The AR Two-Step (Mandatory)

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

## Ownership Boundary

This table defines what your vertical app agents CAN edit vs what requires 7D Platform agents to change.

### Your App Agents CAN Edit (your repo)

| What | Where | Notes |
|------|-------|-------|
| Domain models (pickups, GPS, routes, evidence) | `modules/your-app/` | Your Postgres, your migrations |
| Domain HTTP handlers | `modules/your-app/src/http/` | Your Axum routes |
| Domain event types (payload structs) | `modules/your-app/src/events/` | Define your own event payloads |
| Outbox table + publisher | `modules/your-app/src/events/` | Copy AR's pattern (see CG-EVENTS.md) |
| Cargo.toml path deps to platform crates | `modules/your-app/Cargo.toml` | `event-bus`, `security` |
| Docker Compose for your service | Your compose file | Your HTTP port, your PG port |
| Your DB migrations | `modules/your-app/db/migrations/` | sqlx migrate |

### Requires 7D Platform Agents to Change (platform repo)

| What | Where | Why |
|------|-------|-----|
| Add permission strings (`yourapp.mutate`, `yourapp.read`) | `platform/security/src/permissions.rs` | Central permission registry |
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
   Headers: x-app-id: <your-app-id>, Authorization: Bearer <jwt>, x-correlation-id: <uuid>
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
- Separate `product_code`s: `tenant_platform`, `<your_app>` (one product code per vertical)
- TrashTech cannot silently cause platform billing to begin
- All TrashTech financial events use `MerchantContext::Tenant(tenant_id)`, never `Platform`

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

*Maintained by BrightHill (7D Platform Orchestrator). Update when platform APIs change and include source file reference for each change.*
