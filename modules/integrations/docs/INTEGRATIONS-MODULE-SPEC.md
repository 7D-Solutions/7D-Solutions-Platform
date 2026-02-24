# Integrations Module — Product Vision & Technical Specification

**7D Solutions Platform**
**Status:** Vision Document + Technical Specification (v0.1.0)
**Last updated:** 2026-02-24

---

## Revision History

| Rev | Date | Changed By | Summary |
|-----|------|-----------|---------|
| 1.0 | 2026-02-24 | Platform Orchestrator | Initial vision doc — documented from source: external refs, webhooks, connectors, schema, events, API surface |

---

## The Business Problem

Every multi-tenant SaaS platform that grows past its first customer encounters the same integration challenge: **internal records need to map to external systems, inbound events from third parties need to be received reliably, and outbound integrations need a consistent configuration model.**

Without a dedicated integration layer, these concerns get scattered across every module. The AR module builds its own Stripe mapping table. The customer module builds its own QuickBooks sync. Each webhook receiver re-implements signature verification, idempotency, and routing. Config for external systems lives in environment variables, custom tables, or worse — hardcoded in business logic.

The result is fragile, duplicated integration plumbing that nobody owns and nobody maintains. When a webhook signature algorithm changes or a new payment provider is added, engineers touch five different modules instead of one.

---

## What the Module Does

The Integrations module is the **centralised integration hub** for the platform. It owns three concerns:

1. **External Reference Registry** — A universal mapping table that links any internal platform entity (invoice, customer, order, party) to its identifier in any external system (Stripe, QuickBooks, Salesforce). One table serves all modules and all entity types, scoped per tenant.

2. **Inbound Webhook Ingestion** — A single entry point for all inbound webhooks from external systems. Raw payloads are persisted immediately (audit trail), signatures are verified per-system, idempotency is enforced, and source events are routed to platform domain events via a configurable mapping table.

3. **Outbound Connector Framework** — A pluggable connector model where each integration type (echo, HTTP push, Slack, etc.) declares its config schema and exposes a test action. Connectors are registered per-tenant, validated against their own schema, and invoked uniformly by the platform.

---

## Who Uses This

The module is a platform service consumed by other modules and by any vertical application that needs external system integration. It does not have its own frontend — it exposes an API that frontends and other services consume.

### Platform Module Developer
- Registers external references when syncing internal records to external systems
- Looks up internal entity IDs from external system identifiers (reverse lookup)
- Publishes events that the webhook dispatcher routes to external endpoints

### Integration Administrator
- Registers connector configurations per tenant (e.g. configure a Stripe connector, a Slack notifier)
- Tests connectors via the built-in test action to verify config before going live
- Manages inbound webhook endpoints and monitors ingest records

### External System (Inbound)
- Delivers webhooks to `POST /api/webhooks/inbound/{system}`
- Receives `200 OK` acknowledgment (or `duplicate` for replayed events)
- Signature verification ensures authenticity per source system

### Operations / Observability
- Monitors webhook ingest success/failure rates via Prometheus metrics
- Audits raw webhook payloads stored in the ingest table
- Tracks outbox event publish lag

---

## Design Principles

### One Table for All External References
Rather than each module maintaining its own `_external_refs` table (AR has Stripe IDs, CRM has Salesforce IDs, etc.), the Integrations module provides a single `integrations_external_refs` table parameterised by `entity_type`. This eliminates duplication and provides a universal reverse-lookup capability: given a Stripe ID, find the internal record regardless of which module owns it.

### Ingest First, Route Second
Inbound webhooks are persisted as raw payloads immediately on receipt, before any processing or routing. This decouples ingestion latency from processing latency and provides a durable audit trail. If routing logic changes, historical payloads can be replayed from the ingest table.

### Signature Verification is a Pluggable Adapter
Each supported source system (Stripe, GitHub, internal) has its own signature verifier resolved at dispatch time. The verifier is selected by the `system` path parameter and configured via environment variables. Adding a new system means adding one match arm and one env var — no schema changes.

### Connectors are Stateless and Self-Describing
Every connector implementation satisfies a `Connector` trait with three methods: `capabilities()`, `validate_config()`, and `run_test_action()`. Connectors are zero-state — config is passed at invocation time so the same connector object can serve all tenants. The config schema is self-described via `ConfigField` declarations so UIs and validators work generically.

### Tenant Isolation via app_id
All tables use `app_id` (the tenant/application identifier) as the leading scope column. Every query filters by `app_id`. Cross-tenant data leakage is structurally impossible.

---

## MVP Scope (v0.1.0)

### In Scope
- External reference registry: CRUD with upsert semantics on `(app_id, system, external_id)`
- Reverse lookup: find internal entity by external system + external ID
- Entity lookup: list all external refs for a given internal entity
- Inbound webhook ingestion with raw payload persistence
- Signature verification for Stripe (HMAC-SHA256), GitHub (HMAC-SHA256), and internal (noop)
- Idempotency enforcement via `(app_id, system, idempotency_key)` unique constraint
- Event routing: map source system events to platform domain events
- Transactional outbox for all mutations (Guard-Mutation-Outbox pattern)
- 5 domain events emitted via outbox (see Events Produced)
- Connector framework with trait-based dispatch
- Echo connector (built-in test connector, no external dependencies)
- Connector config registration with per-connector schema validation
- Connector test action dispatch
- Webhook endpoint configuration table (schema defined, API pending)
- HTTP idempotency key table (schema defined, middleware pending)
- Prometheus metrics (request latency, request count, consumer lag)
- Readiness and liveness probes
- Docker multi-stage build

### Explicitly Out of Scope for v1
- Outbound webhook dispatcher (delivering events to registered webhook endpoints)
- Active outbox publisher (background NATS publisher for outbox rows)
- HTTP idempotency middleware (table exists, middleware not wired)
- Connector implementations beyond echo (HTTP push, Slack, etc.)
- Retry logic for failed webhook deliveries
- Webhook endpoint management API (CRUD for `integrations_webhook_endpoints`)
- Batch import/export of external references
- Rate limiting per external system
- Transformation pipelines (mapping external payloads to internal schemas)
- OAuth token management for external systems
- Frontend UI

---

## Technology Summary

| Layer | Technology | Notes |
|-------|-----------|-------|
| Language | Rust | Platform standard |
| HTTP framework | Axum 0.8 | Port 8099 (default) |
| Database | PostgreSQL | Dedicated database, SQLx for queries and migrations |
| Event bus | NATS | Via platform `event-bus` crate (configurable: NATS or InMemory) |
| Auth | JWT via platform `security` crate | Tenant-scoped via `X-App-Id` header |
| Signature verification | Platform `security` crate | StripeVerifier, GenericHmacVerifier, NoopVerifier |
| Outbox | Platform outbox pattern | Module-owned `integrations_outbox` table |
| Projections | Platform `projections` crate | Available but not yet used |
| Metrics | Prometheus | `/metrics` endpoint, SLO-oriented histograms and counters |
| Crypto | sha2, hmac, hex | For webhook signature verification |
| Crate | `integrations-rs` | Single crate, standard module layout |

---

## Structural Decisions (The "Walls")

### 1. Single external refs table parameterised by entity_type — not per-module tables
All external ID mappings live in `integrations_external_refs` with an `entity_type` discriminator. This means any module can register and look up external references through one API. The alternative — each module owning its own external refs table — would fragment the reverse-lookup capability and duplicate schema/code across modules.

### 2. app_id as TEXT, not UUID
The `app_id` column is `TEXT` across all tables, not `UUID`. This accommodates tenant identifiers that may come from external systems or legacy naming. The `database_url_for_app()` function sanitises app_id into safe database names. This is a deliberate trade-off: slightly less type safety for broader compatibility.

### 3. Upsert semantics for external ref creation
`create_external_ref` uses `ON CONFLICT (app_id, system, external_id) DO UPDATE` to make creation idempotent. The same external ID in the same system can only map to one internal entity within a tenant. To remap an external ID to a different entity, the caller must delete and recreate. This prevents silent overwrites of entity mappings.

### 4. Raw payload storage before processing
Inbound webhooks are persisted verbatim before signature verification completes the processing pipeline. The `integrations_webhook_ingest` table stores the JSON body and all HTTP headers. This means failed routing does not lose data, and historical payloads can be replayed.

### 5. System-based signature verification dispatch
Signature verification is a pure function dispatched by the `system` path parameter. Each system resolves to a different verifier implementation. Unknown systems are rejected with `UnsupportedSystem` before any database writes. This keeps the verification layer stateless and testable.

### 6. Connector trait enforces deterministic test actions
Every connector must implement `run_test_action()` that returns a stable, predictable result. This enables E2E tests to validate the full registration → dispatch pipeline without relying on external APIs. The echo connector demonstrates this: given the same config and idempotency key, it always returns the same output.

### 7. Webhook ingest idempotency via database constraint
Duplicate webhook delivery is prevented by a `UNIQUE (app_id, system, idempotency_key)` constraint on `integrations_webhook_ingest`. When a duplicate arrives, the `ON CONFLICT DO NOTHING` returns no rows, and the service returns `is_duplicate: true` without re-emitting events. This is cheaper and more reliable than application-level dedup.

### 8. No cross-module database writes
The `db::resolve_pool()` function is the single point where PgPool instances are created. All database access flows through this function. The module never writes to another module's database. Integration happens exclusively through events and API calls.

---

## Domain Authority

Integrations is the **source of truth** for:

| Domain Entity | Integrations Authority |
|---------------|----------------------|
| **External References** | Universal mapping of internal entities to external system identifiers. Any entity type, any external system, scoped per tenant. |
| **Webhook Ingest Records** | Raw inbound webhook payloads with headers, receipt timestamps, processing status, and idempotency keys. |
| **Webhook Endpoints** | Outbound webhook endpoint configurations: URL, signing secret hash, event type subscriptions, enabled status. |
| **Connector Configs** | Per-tenant connector registrations: connector type, name, config blob, enabled status. |
| **Event Routing Map** | Translation table from `(system, source_event_type)` to platform domain event types. |

Integrations is **NOT** authoritative for:
- The business entities referenced by external refs (invoices, customers, orders — owned by their respective modules)
- External system credentials or OAuth tokens (secrets manager concern)
- Event bus infrastructure or NATS configuration (platform infrastructure concern)
- Business logic triggered by routed events (downstream module concern)

---

## Data Ownership

### Tables Owned by Integrations

All tables use `app_id` for multi-tenant isolation. Every query **MUST** filter by `app_id`.

| Table | Purpose | Key Fields |
|-------|---------|------------|
| **integrations_schema_version** | Schema version tracking | `id`, `version`, `applied_at` |
| **integrations_external_refs** | Universal external ID registry | `id` (BIGSERIAL), `app_id`, `entity_type`, `entity_id`, `system`, `external_id`, `label`, `metadata` (JSONB), `created_at`, `updated_at`. UNIQUE on `(app_id, system, external_id)` |
| **integrations_webhook_endpoints** | Outbound webhook endpoint configs | `id` (UUID), `app_id`, `name`, `url`, `secret_hash` (SHA-256 hex), `event_types` (JSONB array), `enabled`, `created_at`, `updated_at`, `deleted_at` (soft-delete) |
| **integrations_webhook_ingest** | Raw inbound webhook payloads | `id` (BIGSERIAL), `app_id`, `system`, `event_type`, `raw_payload` (JSONB), `headers` (JSONB), `received_at`, `processed_at`, `idempotency_key`. UNIQUE on `(app_id, system, idempotency_key)` |
| **integrations_outbox** | Transactional outbox | `id` (BIGSERIAL), `event_id` (UUID, UNIQUE), `event_type`, `aggregate_type`, `aggregate_id`, `app_id`, `payload` (JSONB), `correlation_id`, `causation_id`, `schema_version`, `created_at`, `published_at` |
| **integrations_processed_events** | Consumer deduplication | `id` (BIGSERIAL), `event_id` (UUID, UNIQUE), `event_type`, `processor`, `processed_at` |
| **integrations_idempotency_keys** | HTTP-level idempotency | `id` (BIGSERIAL), `app_id`, `idempotency_key`, `request_hash`, `response_body` (JSONB), `status_code`, `created_at`, `expires_at`. UNIQUE on `(app_id, idempotency_key)` |
| **integrations_connector_configs** | Per-tenant connector registrations | `id` (UUID), `app_id`, `connector_type`, `name`, `config` (JSONB), `enabled`, `created_at`, `updated_at`. UNIQUE on `(app_id, connector_type, name)` |

### Data NOT Owned by Integrations

Integrations **MUST NOT** store:
- Business entity data (invoice amounts, customer names, order details)
- Raw credentials or OAuth tokens for external systems (only hashed secrets for webhook signing)
- GL account codes or financial data
- Event bus topic configuration or NATS subjects

---

## Events Produced

All events use the platform `EventEnvelope` and are written to the module outbox atomically with the triggering mutation.

| Event | Trigger | Mutation Class | Key Payload Fields |
|-------|---------|---------------|-------------------|
| `external_ref.created` | External ref created or upserted | DATA_MUTATION | `ref_id`, `app_id`, `entity_type`, `entity_id`, `system`, `external_id`, `label`, `created_at` |
| `external_ref.updated` | External ref label/metadata updated | DATA_MUTATION | `ref_id`, `app_id`, `entity_type`, `entity_id`, `system`, `external_id`, `label`, `updated_at` |
| `external_ref.deleted` | External ref hard-deleted | LIFECYCLE | `ref_id`, `app_id`, `entity_type`, `entity_id`, `system`, `external_id`, `deleted_at` |
| `webhook.received` | Raw webhook payload persisted to ingest table | INGEST | `ingest_id`, `system`, `event_type`, `idempotency_key`, `received_at`. `replay_safe: false` |
| `webhook.routed` | Inbound webhook mapped to a domain event type | ROUTING | `ingest_id`, `system`, `source_event_type`, `domain_event_type`, `outbox_event_id`, `routed_at`. `replay_safe: true` |
| `connector.registered` | Connector config persisted | — | `connector_id`, `app_id`, `connector_type`, `name`, `registered_at` |

---

## Events Consumed

| Event | Source | Action |
|-------|--------|--------|
| *(None in v1)* | — | Integrations is event-producing only in v1. Future: consume platform events to dispatch to outbound webhook endpoints. |

---

## Integration Points

### Platform Security Crate (Compile-Time Dependency)
Webhook signature verification delegates to `security::StripeVerifier`, `security::GenericHmacVerifier`, and `security::NoopVerifier` from the platform security crate. The `WebhookVerifier` trait provides the contract. Rate limiting, timeout, body size limits, and auth are applied via security middleware layers.

### Platform Event-Bus Crate (Compile-Time Dependency)
The `EventEnvelope` type and bus abstraction come from the platform `event-bus` crate. The module supports both NATS and InMemory bus types, selected via the `BUS_TYPE` environment variable.

### Platform Health Crate (Compile-Time Dependency)
Readiness probe logic (`build_ready_response`, `db_check`) comes from the platform `health` crate. The `/healthz` endpoint is provided by this crate directly.

### All Other Modules (Loose Coupling via External Refs)
Any module can store external references through the Integrations API. The `entity_type` + `entity_id` fields are opaque strings — Integrations does not validate that the referenced entity exists. This keeps the coupling loose: modules reference Integrations, but Integrations never calls other modules.

### Inbound Webhook Sources (Runtime, External)
External systems (Stripe, GitHub) deliver webhooks to `/api/webhooks/inbound/{system}`. Signature secrets are configured via environment variables (`STRIPE_WEBHOOK_SECRET`, `GITHUB_WEBHOOK_SECRET`). The module verifies signatures, persists payloads, and routes to domain events.

### Event Routing Map
The routing map in `domain/webhooks/routing.rs` translates source system events to platform domain events:

| Source System | Source Event | Domain Event |
|---------------|-------------|-------------|
| stripe | payment_intent.succeeded | payment.received |
| stripe | payment_intent.payment_failed | payment.failed |
| stripe | invoice.payment_succeeded | invoice.paid.external |
| stripe | invoice.payment_failed | invoice.payment_failed.external |
| stripe | customer.subscription.created | subscription.created.external |
| stripe | customer.subscription.deleted | subscription.cancelled.external |
| github | push | repository.push |
| github | pull_request | repository.pull_request |
| internal | *(any)* | *(pass-through)* |

---

## Invariants

1. **Tenant isolation is unbreakable.** Every query filters by `app_id`. No cross-tenant data leakage.
2. **External ID uniqueness per system.** The `UNIQUE (app_id, system, external_id)` constraint prevents the same external identifier from being claimed by more than one internal record within a tenant.
3. **Outbox atomicity.** Every state-changing mutation writes its event to the outbox in the same database transaction. No silent event loss.
4. **Webhook idempotency.** Duplicate webhook delivery is prevented by the `UNIQUE (app_id, system, idempotency_key)` constraint. Duplicates return `is_duplicate: true` without re-emitting events.
5. **Signature before storage.** Webhook signature verification runs before any database writes. Invalid signatures are rejected at the guard layer.
6. **Connector type validation.** Connector registration fails if the `connector_type` is not in the registry. Config is validated against the connector's own schema before persisting.
7. **No cross-module database writes.** All DB access flows through the single `db::resolve_pool()` function. The module never writes to another module's tables.
8. **Secret hashing.** Webhook endpoint signing secrets are stored as SHA-256 hex hashes, never in plaintext.

---

## API Surface (Summary)

### External References
- `POST /api/integrations/external-refs` — Create or upsert an external ref
- `GET /api/integrations/external-refs/by-entity?entity_type=X&entity_id=Y` — List refs by internal entity
- `GET /api/integrations/external-refs/by-system?system=X&external_id=Y` — Reverse lookup by external system
- `GET /api/integrations/external-refs/{id}` — Get ref by ID
- `PUT /api/integrations/external-refs/{id}` — Update label/metadata
- `DELETE /api/integrations/external-refs/{id}` — Hard delete

### Inbound Webhooks
- `POST /api/webhooks/inbound/{system}` — Ingest an inbound webhook (Stripe, GitHub, internal)

### Connectors
- `GET /api/integrations/connectors/types` — List all registered connector types and capabilities
- `POST /api/integrations/connectors` — Register a connector config for this tenant
- `GET /api/integrations/connectors` — List tenant's connector configs (optional `?enabled_only=true`)
- `GET /api/integrations/connectors/{id}` — Get a single connector config
- `POST /api/integrations/connectors/{id}/test` — Run the connector's test action

### Operational
- `GET /healthz` — Kubernetes liveness probe (platform health crate)
- `GET /api/health` — Liveness check (module-level)
- `GET /api/ready` — Readiness check (verifies DB connectivity)
- `GET /api/version` — Module identity and schema version
- `GET /metrics` — Prometheus metrics

---

## Supported Webhook Systems

| System | Verifier | Required Env Var | Idempotency Key Source |
|--------|----------|-----------------|----------------------|
| `stripe` | `StripeVerifier` (HMAC-SHA256) | `STRIPE_WEBHOOK_SECRET` | Payload `id` field (Stripe event ID) |
| `github` | `GenericHmacVerifier` (SHA-256, `x-hub-signature-256` header) | `GITHUB_WEBHOOK_SECRET` | `X-Webhook-Id` header |
| `internal` | `NoopVerifier` (no verification) | — | `X-Webhook-Id` header or payload `event_type` |

Unknown systems are rejected with `404 Not Found` before any database writes.

---

## Connector Framework

### Connector Trait Contract

Every connector implementation must satisfy:

| Method | Purpose |
|--------|---------|
| `connector_type()` | Unique type discriminator (e.g. `"echo"`, `"http-push"`) |
| `capabilities()` | Advertise supported features, config fields, and test action availability |
| `validate_config()` | Validate tenant-supplied config blob against the connector's declared schema |
| `run_test_action()` | Execute a deterministic test action, echoing the caller's idempotency key |

### Registered Connectors (v0.1.0)

| Type | Description | Config Fields | Test Action |
|------|-------------|--------------|-------------|
| `echo` | Built-in test connector — no external dependencies | `echo_prefix` (Text, optional, default: `"ping"`, max 64 chars) | Returns `"echo: <prefix> \| idempotency: <key>"` |

---

## v2 Roadmap (Deferred)

| Feature | Rationale for Deferral |
|---------|----------------------|
| **Outbound Webhook Dispatcher** | Delivers events to registered `integrations_webhook_endpoints`. Requires retry logic, circuit breaker, and delivery status tracking. Schema is ready (table exists). |
| **Outbox Publisher** | Background task that reads unpublished outbox rows and publishes to NATS. Required for event-driven architecture to function end-to-end. |
| **HTTP Idempotency Middleware** | Table and schema exist (`integrations_idempotency_keys`). Middleware to intercept requests and return cached responses not yet wired. |
| **HTTP Push Connector** | Outbound HTTP connector for pushing events to arbitrary endpoints with retry. |
| **Slack Connector** | Notify Slack channels on platform events. |
| **OAuth Token Management** | Secure storage and refresh of OAuth tokens for external systems. |
| **Transformation Pipelines** | Map external payloads to internal schemas via configurable transformations. |
| **Batch External Ref Import** | Bulk import of external reference mappings from CSV/JSON. |
| **Webhook Endpoint CRUD API** | Management API for `integrations_webhook_endpoints` (table exists, endpoints not implemented). |
| **Event Replay** | Re-process historical webhook ingest records through updated routing logic. |

---

## Document Standards Reference

This document follows the revision and decision log standards defined at:
`docs/frontend/DOC-REVISION-STANDARDS.md`

---

## Decision Log

| Date | Decision | Rationale | Decided By |
|------|----------|-----------|-----------|
| 2026-02-24 | Single external refs table parameterised by entity_type | Universal reverse-lookup; eliminates per-module duplication; any module can register refs through one API | Platform Orchestrator |
| 2026-02-24 | app_id as TEXT not UUID | Accommodates external/legacy tenant identifiers; sanitised for database names via `database_url_for_app()` | Platform Orchestrator |
| 2026-02-24 | Upsert semantics on external ref creation | Idempotent creates prevent duplicate mappings; remapping requires explicit delete+recreate to prevent silent overwrites | Platform Orchestrator |
| 2026-02-24 | Raw payload storage before processing | Decouples ingestion from routing; provides durable audit trail; enables future replay capability | Platform Orchestrator |
| 2026-02-24 | System-based signature verification dispatch | Stateless, testable adapter pattern; adding a new system is one match arm + one env var; unknown systems rejected before DB writes | Platform Orchestrator |
| 2026-02-24 | Connector trait with deterministic test actions | E2E tests validate full pipeline without external APIs; self-describing config schema enables generic UI rendering | Platform Orchestrator |
| 2026-02-24 | Webhook idempotency via database constraint not application logic | Cheaper, race-safe, and simpler than in-memory dedup; leverages PostgreSQL's UNIQUE constraint + ON CONFLICT DO NOTHING | Platform Orchestrator |
| 2026-02-24 | No cross-module database writes | All integration through events and API calls; single `resolve_pool()` entry point prevents accidental cross-module coupling | Platform Orchestrator |
| 2026-02-24 | Secret hashing for webhook endpoints | Signing secrets stored as SHA-256 hex, never plaintext; raw secret returned once at creation time only | Platform Orchestrator |
| 2026-02-24 | BUS_TYPE configurable (NATS or InMemory) | InMemory mode enables local development and testing without NATS infrastructure; NATS for production | Platform Orchestrator |
