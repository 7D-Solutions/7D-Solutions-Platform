# 7D Solutions Platform — Consumer Guide

**Audience:** Agent teams building vertical applications (TrashTech Pro, etc.) on top of the 7D Platform.
**Purpose:** Stop you from reading the entire codebase. Everything you need to know to build correctly is here.

---

## What the Platform IS

The 7D Solutions Platform is a **multi-tenant SaaS backend infrastructure**. It provides shared services (auth, entity master, AR billing, payments, GL, etc.) that vertical products consume via REST APIs.

**You (a vertical product) are a TENANT on the platform.**

- You get a `tenant_id` (UUID) provisioned during onboarding
- You get one or more `app_id` values (short string, e.g. `trashtech-pro`) that scope your data within the platform
- All platform module data is automatically scoped to your `tenant_id` — you cannot see other tenants' data
- You do **not** have direct DB access to platform module databases — you call REST APIs

---

## Module Reference

### Platform Services

| Service | Container | HTTP Port | Postgres Port | Purpose |
|---------|-----------|-----------|---------------|---------|
| identity-auth | 7d-auth-lb | 8080 | 5433 | JWT auth, users, RBAC |
| tenant-registry | (internal) | — | 5441 | Tenant provisioning |
| projections | (internal) | — | 5439 | GL/reporting projections |
| audit | (internal) | — | 5440 | Immutable audit trail |

### Business Modules

| Service | Container | HTTP Port | Postgres Port | Purpose |
|---------|-----------|-----------|---------------|---------|
| AR | 7d-ar | 8086 | 5434 | Invoices, payments, aging, dunning |
| Subscriptions | 7d-subscriptions | 8087 | 5435 | Recurring billing, plan lifecycle |
| Payments | 7d-payments | 8088 | 5436 | Payment processing, allocation |
| Notifications | 7d-notifications | 8089 | 5437 | Event-driven notifications |
| GL | 7d-gl | 8090 | 5438 | General ledger, journal entries |
| Inventory | 7d-inventory | 8092 | 5442 | Stock, reservations, receipts |
| AP | 7d-ap | 8093 | 5443 | Vendor bills, payment runs |
| Treasury | 7d-treasury | 8094 | 5444 | Cash position, recon, forecast |
| Fixed Assets | 7d-fixed-assets | 8095 | 5445 | Asset register, depreciation |
| Consolidation | 7d-consolidation | 8096 | 5446 | Multi-entity GL rollup |
| Timekeeping | 7d-timekeeping | 8097 | 5447 | Time entries, project tracking |
| Party Master | 7d-party | 8098 | 5448 | Entity master (customers, vendors, contacts) |
| Integrations | 7d-integrations | 8099 | 5449 | External ID mapping, inbound webhooks |
| TTP | 7d-ttp | 8100 | 5450 | Platform billing (7D bills your product) |

---

## Required HTTP Headers

Every call to a platform module API **must** include:

```
x-app-id: your-app-id           # Required: scopes data to your application
x-tenant-id: <tenant-uuid>      # Required for some endpoints (auth enforces this via JWT)
x-correlation-id: <uuid>        # Required: for distributed tracing
x-actor-id: <user-uuid>         # Required: who is performing the action (audit trail)
Authorization: Bearer <jwt>     # Required: JWT from identity-auth
```

The `x-app-id` is the most critical — **all data stored in platform modules is partitioned by app_id**. If you send the wrong app_id, you will get back the wrong data (or no data).

---

## Data Ownership Rules

### What lives in PLATFORM databases (you call APIs)

- **Counterparties** (customers, vendors, contacts) → Party Master
- **Invoices and AR** (receivables from your customers) → AR module
- **Vendor bills and AP** (payables to your vendors) → AP module
- **Payments** → Payments module
- **Subscriptions** → Subscriptions module
- **Financial reports and GL** → GL module (journal entries posted via events)
- **Auth tokens, users, RBAC roles** → identity-auth
- **Audit trail** → Audit service (written automatically on all mutations)

### What lives in YOUR database (you own, you manage)

**Everything domain-specific to your vertical.** Examples for TrashTech Pro:
- `pickup_jobs` — service requests, job status, scheduling
- `gps_pings` — driver telemetry
- `routes` and `stop_sequences` — route planning
- `evidence_records` — RFID scans, camera timestamps, proof of service
- Any TrashTech-specific operational data that no platform module covers

You run your own Postgres, your own sqlx migrations, your own repo layer. The platform does not manage your operational DB.

---

## Tenant and App Scoping

### tenant_id
A UUID assigned to your product organization during provisioning. Every platform module filters all queries by `tenant_id`. You cannot cross tenant boundaries.

### app_id
A short string (e.g. `trashtech-pro`, `trashtech-driver-app`). Use this to partition your data within a tenant if you run multiple apps. Most products have one app_id.

### merchant_context (EventEnvelope field)
Every event on the NATS bus carries a `merchant_context` field:
- `TENANT` — your product is operating (you billing your customers, your operational events)
- `PLATFORM` — 7D Solutions is billing you (TTP invoices, platform administrative events)

When you publish events, always use `TENANT`. The `PLATFORM` context is reserved for TTP and platform operations.

---

## Authentication

**Never build your own auth.** Use identity-auth.

1. **Login / token issuance:** `POST http://7d-auth-lb:8080/api/auth/login`
2. **Token validation:** Call identity-auth's introspection endpoint or validate JWT signature against the public key
3. **JWT claims include:** `user_id`, `tenant_id`, `app_id`, `roles`, `permissions`, `actor_type`
4. **RBAC:** Roles and permissions are defined in identity-auth. Your service can call `GET /api/auth/permissions` to check if a user has a specific permission.

For service-to-service calls (no human user), use a service account JWT with `actor_type: service`.

---

## The party_id Pattern

`party_id` is the **cross-module linking key** for counterparties.

**Flow:**
1. Create your customer/vendor/contact in **Party Master** → get a `party_id` (UUID)
2. Store that `party_id` in your own operational tables (e.g. on `pickup_jobs.customer_party_id`)
3. When creating an AR invoice, pass the `party_id` → AR links the invoice to the entity
4. When creating an AP vendor, pass the `party_id` → AP links the vendor to the entity

**Rules:**
- Party Master is the single source of truth for WHO an entity is
- Your vertical app stores `party_id` as a foreign reference, not the entity data itself
- Never duplicate customer name/address/contact info in your own tables — look it up from Party Master

**Party types:** `company`, `individual`, `contact`

---

## Billing Architecture

| Flow | Module | Who pays who |
|------|--------|-------------|
| Your customers pay you | AR module | Customers → your product |
| You bill on subscriptions | Subscriptions + AR | Recurring billing to your customers |
| Payments processing | Payments module | Gateway integration for actual money movement |
| 7D Platform bills you | TTP module | 7D Solutions → your product (metered usage) |

**Your product does not implement its own billing logic.** You create invoices in AR, create subscriptions in Subscriptions, and let the platform handle the lifecycle.

---

## Event Bus (NATS)

The platform uses **NATS JetStream** for async communication between services.

**To subscribe to platform events:**
- Use the `7d-integrations` module's inbound webhook if you want event routing from external systems
- Or subscribe directly to NATS subjects from your service (contact BrightHill for subject naming conventions)

**Key subjects (examples):**
- `invoice.issued` — AR emits when an invoice is created
- `payment.succeeded` — Payments emits on successful payment
- `payment.failed` — Payments emits on failed payment
- `subscription.status_changed` — Subscriptions emits on lifecycle transitions

**EventEnvelope structure (all events):**
```json
{
  "event_id": "<uuid>",
  "event_type": "invoice.issued",
  "tenant_id": "<uuid>",
  "app_id": "your-app-id",
  "actor_id": "<user-or-service-uuid>",
  "merchant_context": "TENANT",
  "payload": { ... }
}
```

All events are idempotent by `event_id`. Your consumer must deduplicate by `event_id`.

---

## Integrations Module

Use the **Integrations module** (port 8099) for:
- **Inbound webhooks:** POST `/api/integrations/webhooks/inbound` — ingest external system events (e.g. payment gateway callbacks, GPS provider webhooks)
- **External ref mapping:** Map your internal IDs to external system IDs and vice versa — `POST /api/integrations/external-refs`, `GET /api/integrations/external-refs/by-external`

---

## Standard Ops Endpoints

Every platform module exposes:
```
GET /api/health    — liveness check
GET /api/ready     — readiness check
GET /api/version   — build version info
GET /metrics       — Prometheus metrics
```

---

## Do's and Don'ts

### DO

- ✅ **Create counterparties in Party Master first** — always get a `party_id` before creating invoices or vendor records
- ✅ **Include all required headers on every API call** — missing `x-app-id` will give you empty results, not an error
- ✅ **Use `x-correlation-id` on every request** — platform modules propagate this through the audit trail
- ✅ **Own your operational data** — your domain tables live in your DB; platform tables live in platform DBs
- ✅ **Subscribe to NATS events for cross-module reactions** — don't poll; listen for events
- ✅ **Use identity-auth for all user auth** — never roll your own JWT or session logic
- ✅ **Treat `party_id` as the universal counterparty key** — store it everywhere you reference an entity
- ✅ **Check existing E2E tests for usage examples** — `e2e-tests/tests/` has real working examples for every module

### DON'T

- ❌ **Don't talk directly to platform Postgres** — use REST APIs only
- ❌ **Don't store customer/vendor entity data in your own tables** — that lives in Party Master
- ❌ **Don't build your own billing** — AR + Subscriptions + Payments handle it
- ❌ **Don't build your own auth** — identity-auth handles it
- ❌ **Don't use `merchant_context: PLATFORM`** — that's reserved for 7D internal use
- ❌ **Don't skip `x-app-id`** — you will get wrong data with no error
- ❌ **Don't call platform modules from your DB layer** — keep API calls in your service/HTTP layer
- ❌ **Don't mock platform services in integration tests** — tests must use real services

---

## Quick Start for a New Vertical App

1. **Register your tenant:** Contact BrightHill (orchestrator) to provision `tenant_id` and `app_id` via tenant-registry
2. **Create your first party:** `POST http://7d-party:8098/api/party/parties` — get a `party_id` for your first customer
3. **Create an invoice:** `POST http://7d-ar:8086/api/ar/invoices` with `party_id` and required headers
4. **Subscribe to payment events:** Listen on NATS for `payment.succeeded` to trigger your fulfillment logic
5. **Your operational DB:** Provision your own Postgres, write your own migrations, store domain-specific data (jobs, routes, evidence, etc.)

---

## Reference: Useful E2E Tests

These test files in `e2e-tests/tests/` show working examples of platform API usage:

| Test | What it shows |
|------|--------------|
| `party_master_e2e.rs` | Party CRUD: create, get, update, deactivate, search |
| `party_ar_link.rs` | party_id on AR invoices |
| `ap_vendor_party_link_e2e.rs` | party_id on AP vendors |
| `cross_module_invoice_payment_e2e.rs` | Invoice → payment full cycle |
| `cross_module_subscription_invoice_e2e.rs` | Subscription → invoice creation |
| `integrations_integration.rs` | Inbound webhook → external ref |
| `subscriptions_lifecycle.rs` | Subscription state machine |
| `provisioning_full_lifecycle_e2e.rs` | Tenant provisioning |
| `rbac_enforcement.rs` | RBAC permission checks |
| `treasury_forecast_e2e.rs` | Cash forecast from AR/AP data |

---

*Maintained by BrightHill (7D Platform Orchestrator). Update this file when platform APIs change.*
