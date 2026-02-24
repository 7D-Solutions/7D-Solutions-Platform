# Party Module — Product Vision & Technical Specification

**7D Solutions Platform**
**Status:** Vision Document + Technical Specification (v0.1.0)
**Last updated:** 2026-02-24

---

## Revision History

| Rev | Date | Changed By | Summary |
|-----|------|-----------|---------|
| 1.0 | 2026-02-24 | Platform Orchestrator | Initial vision doc — full module audit of source, schema, events, API, invariants, integration points, decision log |

---

## The Business Problem

Every multi-tenant SaaS platform needs a canonical answer to the question: **"who are we dealing with?"** Customers, vendors, employees, partners, subcontractors — these are all parties that appear across invoices, payments, subscriptions, work orders, and tenant provisioning. Without a unified party model, the same company or person gets duplicated across every module that needs to reference them. Name changes propagate inconsistently. External system IDs (Stripe customer, QuickBooks vendor, Salesforce contact) are scattered in module-specific columns with no cross-reference.

The problem compounds in a multi-tenant platform. Each vertical application (waste management, fleet services, property management) has its own party universe, but the platform modules (AR, AP, Subscriptions, Payments) all need to reference parties without knowing which vertical is calling. Without a shared party registry, every module independently reinvents customer/vendor storage, and cross-module operations (e.g., "bill the customer who owns this work order") require brittle ID mapping.

---

## What the Module Does

The Party module is the **authoritative registry of all legal and natural persons** across the platform. It is the single place where companies and individuals are created, updated, searched, and deactivated. Every other module that needs to reference a party does so by UUID — it never stores its own copy of party details.

It answers four questions:
1. **Who is this?** — A unified party record with display name, contact info, and type-specific extensions (company details or individual details).
2. **How do they map to external systems?** — External references link a party to identifiers in Stripe, QuickBooks, Salesforce, or any other system, with uniqueness enforced per app+system.
3. **Who are the people at this organization?** — Contacts represent named persons linked to a party (billing contact, technical contact, primary contact), each with role, email, phone, and primary flag.
4. **Where are they?** — Typed, multi-address support (billing, shipping, registered, mailing) with primary flag management per party.

---

## Who Uses This

The module is a platform service consumed by other modules and vertical applications via its REST API. It does not have its own frontend.

### Vertical Application (e.g., TrashTech, Fleet Manager)
- Creates companies and individuals during onboarding flows
- Searches parties by name, type, or external reference
- Links parties to external system IDs (Stripe customers, QuickBooks vendors)
- Manages contacts and addresses for each party

### AR / Billing Module
- References party UUIDs as the customer on invoices and payment records
- Looks up party details for invoice generation and payment receipts

### AP Module
- References party UUIDs as the vendor on purchase orders and bills

### Subscriptions Module
- References party UUIDs as the subscriber entity

### Tenant Control Plane
- Uses party records to display tenant-scoped customer/vendor lists
- Searches parties for assignment to operations

### System (Event Consumers)
- Consumes `party.created`, `party.updated`, and `party.deactivated` events to maintain projections and trigger downstream workflows (e.g., welcome emails, CRM sync)

---

## Design Principles

### Unified Party Model with Typed Extensions
The module uses a single base table (`party_parties`) for all parties, with a `party_type` discriminator (company or individual). Type-specific fields live in 1:1 extension tables (`party_companies`, `party_individuals`). This avoids duplicating shared fields (display_name, email, phone, address, status) while allowing type-specific data to be strongly typed. A `PartyView` composite response assembles the base record, extension, external refs, contacts, and addresses into a single response.

### App-Scoped Multi-Tenancy via `app_id`
Rather than the `tenant_id` pattern used by other modules, Party scopes all data by `app_id` — a string identifier that represents the calling application or tenant. Every query filters by `app_id`. This design allows the same Party database to serve multiple vertical applications, each with its own isolated party universe. The `X-App-Id` header carries this scope on every HTTP request.

### External References as First-Class Citizens
Party-to-external-system mappings are not metadata — they have their own table (`party_external_refs`) with a uniqueness constraint on `(app_id, system, external_id)`. This prevents the same Stripe customer ID from being claimed by two different parties within an app, and enables efficient lookup by external system and ID.

### Soft Deletion via Status
Parties are never hard-deleted. Deactivation sets `status = 'inactive'` and emits a `party.deactivated` event. Consumers decide how to handle deactivation (archive, block operations, notify). The `archived` status exists in the enum for future use but is not yet exposed in the API.

### Guard-Mutation-Outbox Atomicity
Every write operation follows the platform pattern: validate input (Guard), execute the database mutation, and write the corresponding event to the outbox — all in a single database transaction. If the event fails to write, the mutation is rolled back. No silent data changes.

### No Runtime Dependencies on Other Modules
Party does not call any other module at runtime. It is consumed by other modules, not the other way around. This makes it a foundational service that can boot and operate independently.

---

## MVP Scope (v0.1.0)

### In Scope
- Base party register (company and individual types) with CRUD
- Company extension: legal name, trade name, registration number, tax ID, country of incorporation, industry code, founded date, employee count, annual revenue, currency
- Individual extension: first name, last name, middle name, date of birth, tax ID, nationality, job title, department
- External references: map parties to identifiers in external systems (Stripe, QuickBooks, etc.)
- Contacts: named persons linked to a party with role, email, phone, primary flag
- Addresses: typed addresses (billing, shipping, registered, mailing, other) per party with primary flag
- Search: by name (ILIKE), party type, status, external system/ID, with pagination
- Soft deactivation with event emission
- 3 domain events via transactional outbox: `party.created`, `party.updated`, `party.deactivated`
- HTTP idempotency key infrastructure (table exists, not yet wired to handlers)
- Event consumer deduplication infrastructure (`party_processed_events` table)
- Prometheus metrics (request latency histogram, request counter, consumer lag gauge)
- Readiness probe with DB connectivity check
- OpenAPI contract
- Docker image (multi-stage build with cargo-chef)

### Explicitly Out of Scope for v1
- Party merge / deduplication (combining duplicate party records)
- Relationship graph (party-to-party relationships: parent company, subsidiary, partner)
- Party history / audit log (versioned change tracking beyond events)
- Active external ref synchronization (push/pull to external systems)
- HTTP-level idempotency enforcement (infrastructure exists but not wired)
- Bulk import/export
- Party verification / KYC integration
- Frontend UI (consumed via API by vertical apps or TCP)

---

## Technology Summary

| Layer | Technology | Notes |
|-------|-----------|-------|
| Language | Rust | Platform standard |
| HTTP framework | Axum | Port 8098 (default) |
| Database | PostgreSQL | Dedicated database, SQLx for queries and migrations |
| Event bus | NATS | Via platform `event-bus` crate; InMemory mode available for testing |
| Auth | JWT via platform `security` crate | App-scoped via `X-App-Id` header, rate limiting, timeout middleware |
| Outbox | Platform outbox pattern | `party_outbox` table, same pattern as all other modules |
| Metrics | Prometheus | `/metrics` endpoint via `prometheus` crate |
| Projections | Platform `projections` crate | Dependency declared (event consumer infrastructure) |
| Crate | `party-rs` | Single crate, standard module layout |

---

## Structural Decisions (The "Walls")

### 1. Unified base table with typed extensions, not separate company/individual tables
All parties share a single `party_parties` base table. Type-specific fields live in 1:1 extension tables joined by `party_id`. This means search, listing, and status operations work identically regardless of party type — you never need to query two separate tables to find "all active parties." The extension tables are only joined when you need type-specific detail (the `PartyView` composite).

### 2. `app_id` string scope instead of UUID `tenant_id`
Party uses `app_id` (a freeform string) rather than the UUID `tenant_id` used by other modules. This accommodates multiple vertical applications sharing the same Party service, each with a different app identifier. The tradeoff is looser typing — `app_id` is not FK-validated against a tenants table. The benefit is flexibility: any calling service can self-identify without pre-registration.

### 3. External references are a dedicated table with uniqueness constraint
External system mappings (`party_external_refs`) have their own table rather than being stored in party metadata. The `UNIQUE(app_id, system, external_id)` constraint prevents the same external ID from being claimed by two parties within an app. This is critical for reliable lookups like "find the party for Stripe customer cus_abc123."

### 4. Contacts and addresses are subresources, not embedded in the party record
The base party table has inline address fields (`address_line1`, `city`, etc.) for backward compatibility, but the primary address model is the separate `party_addresses` table with typed addresses and primary flag management. Similarly, contacts are a separate table rather than embedded JSON. This allows multiple addresses and contacts per party with individual CRUD operations.

### 5. Primary flag is exclusive per party — enforced in application code
When a contact or address is marked as `is_primary = true`, the service clears the primary flag on all other contacts/addresses for that party within the same transaction. This ensures exactly one primary contact and one primary address per party, enforced atomically.

### 6. Soft delete only — no hard delete of parties
Deactivation sets `status = 'inactive'`. There is no delete endpoint. This preserves referential integrity for all modules that hold party UUIDs (AR invoices, work orders, subscriptions). Downstream consumers react to the `party.deactivated` event.

### 7. All write operations emit events via the transactional outbox
`party.created`, `party.updated`, and `party.deactivated` are written to `party_outbox` atomically with the business mutation. Contact and address CRUD operations do not currently emit events (they are subresource-level operations).

### 8. No mocking in tests
Integration tests hit real Postgres. Test helpers create pools against a real database URL and run migrations before each test. This is a platform-wide standard.

---

## Domain Authority

Party is the **source of truth** for:

| Domain Entity | Party Authority |
|---------------|----------------|
| **Parties (base)** | UUID identity, app scope, party type (company/individual), status (active/inactive/archived), display name, primary email/phone/website, inline address fields, freeform metadata |
| **Company Extension** | Legal name, trade name, registration number, tax ID, country of incorporation, industry code, founded date, employee count, annual revenue, currency |
| **Individual Extension** | First name, last name, middle name, date of birth, tax ID, nationality, job title, department |
| **External References** | Mappings from party to external system identifiers (Stripe, QuickBooks, Salesforce, etc.) with uniqueness per app+system |
| **Contacts** | Named persons linked to a party: first/last name, email, phone, role, primary flag |
| **Addresses** | Typed addresses per party: billing, shipping, registered, mailing, other. Street lines, city, state, postal code, country, primary flag |

Party is **NOT** authoritative for:
- Financial balances, invoices, or payment history (AR/Payments modules own this)
- Subscription status or billing plans (Subscriptions module owns this)
- Tenant provisioning or app configuration (Tenant Provisioning module owns this)
- Authentication credentials or user accounts (identity-auth module owns this)
- Maintenance assets, work orders, or service history (Maintenance module owns this)

---

## Data Ownership

### Tables Owned by Party

All tables use `app_id` for multi-application isolation. Every query **MUST** filter by `app_id`.

| Table | Purpose | Key Fields |
|-------|---------|------------|
| **party_parties** | Base party register | `id` (UUID PK), `app_id`, `party_type` (company\|individual), `status` (active\|inactive\|archived), `display_name`, `email`, `phone`, `website`, `address_line1`..`country`, `metadata` (JSONB), `created_at`, `updated_at` |
| **party_companies** | Company extension (1:1) | `party_id` (UUID PK, FK→party_parties), `legal_name`, `trade_name`, `registration_number`, `tax_id`, `country_of_incorporation`, `industry_code`, `founded_date`, `employee_count`, `annual_revenue_cents` (BIGINT), `currency`, `metadata` (JSONB) |
| **party_individuals** | Individual extension (1:1) | `party_id` (UUID PK, FK→party_parties), `first_name`, `last_name`, `middle_name`, `date_of_birth`, `tax_id`, `nationality`, `job_title`, `department`, `metadata` (JSONB) |
| **party_external_refs** | External system mappings | `id` (BIGSERIAL PK), `party_id` (FK), `app_id`, `system`, `external_id`, `label`, `metadata` (JSONB). UNIQUE on `(app_id, system, external_id)` |
| **party_contacts** | Contact persons per party | `id` (UUID PK), `party_id` (FK), `app_id`, `first_name`, `last_name`, `email`, `phone`, `role`, `is_primary` (BOOLEAN), `metadata` (JSONB) |
| **party_addresses** | Typed addresses per party | `id` (UUID PK), `party_id` (FK), `app_id`, `address_type` (billing\|shipping\|registered\|mailing\|other), `label`, `line1`, `line2`, `city`, `state`, `postal_code`, `country`, `is_primary` (BOOLEAN), `metadata` (JSONB) |
| **party_outbox** | Transactional outbox | Standard platform outbox: `event_id`, `event_type`, `aggregate_type`, `aggregate_id`, `app_id`, `payload` (JSONB), `correlation_id`, `causation_id`, `schema_version`, `published_at` |
| **party_processed_events** | Event deduplication | `event_id` (UUID UNIQUE), `event_type`, `processor`, `processed_at` |
| **party_idempotency_keys** | HTTP idempotency | `app_id`, `idempotency_key`, `request_hash`, `response_body` (JSONB), `status_code`, `expires_at`. UNIQUE on `(app_id, idempotency_key)` |

**Monetary Precision:** `annual_revenue_cents` uses integer minor units (cents). Currency stored as ISO 4217 code (default `usd`).

**Cascade Deletes:** `party_companies`, `party_individuals`, `party_external_refs`, `party_contacts`, and `party_addresses` all cascade on delete from `party_parties`. However, party records are never hard-deleted via the API — this cascade exists as a schema safety net.

### Data NOT Owned by Party

Party **MUST NOT** store:
- Financial transaction data (invoices, payments, journal entries)
- Subscription plans, billing cycles, or usage records
- Authentication credentials, session tokens, or user account data
- Inventory items, stock levels, or warehouse locations
- Maintenance assets, work orders, or service schedules

---

## Events Produced

All events use the platform `EventEnvelope` and are written to `party_outbox` atomically with the triggering mutation.

| Event | Trigger | Key Payload Fields | Mutation Class |
|-------|---------|-------------------|----------------|
| `party.created` | Company or individual created | `party_id`, `app_id`, `party_type`, `display_name`, `email`, `created_at` | DATA_MUTATION |
| `party.updated` | Base party fields updated | `party_id`, `app_id`, `display_name` (if changed), `email` (if changed), `updated_by`, `updated_at` | DATA_MUTATION |
| `party.deactivated` | Party soft-deleted | `party_id`, `app_id`, `deactivated_by`, `deactivated_at` | LIFECYCLE |

**Event envelope metadata:**
- `source_module`: `"party"`
- `schema_version`: `"1.0.0"`
- `replay_safe`: `true`
- `correlation_id`: Propagated from `X-Correlation-Id` header
- `actor_id` / `actor_type`: Optional, propagated from VerifiedClaims on HTTP mutations

**Note:** Contact and address CRUD operations do not currently emit events. If downstream consumers need to react to contact/address changes, this would be a future enhancement.

---

## Events Consumed

| Event | Source | Action |
|-------|--------|--------|
| *(None in v1)* | — | Party is event-producing only. It does not subscribe to events from other modules. |

---

## Integration Points

### AR / Payments / Subscriptions (Consume Party UUIDs)
These modules store `party_id` as a foreign reference to identify the customer or vendor on their records. They call Party's GET endpoint to resolve display names for invoices and reports. **Party never calls these modules.**

### External Systems (via External References)
The `party_external_refs` table maps party UUIDs to identifiers in external systems (Stripe `cus_xxx`, QuickBooks vendor IDs, Salesforce contact IDs). In v1, this mapping is passive — it is set by the calling application and used for lookup. Active synchronization (push/pull) is deferred to v2.

### Notifications (Event-Driven, One-Way)
The Notifications module can subscribe to party events (`party.created`, `party.deactivated`) to trigger welcome emails, deactivation notices, or CRM sync workflows. **Party never calls Notifications.**

### Identity / Auth (No Direct Integration)
Party records represent business entities (customers, vendors). They are not the same as user accounts in `identity-auth`. A user account may be associated with a party via the calling application's logic, but Party has no direct dependency on or reference to identity-auth.

---

## Invariants

1. **App isolation is unbreakable.** Every query filters by `app_id`. No cross-app data leakage.
2. **External reference uniqueness.** `(app_id, system, external_id)` is unique — the same external ID cannot be claimed by two parties within an app.
3. **Outbox atomicity.** Every state-changing party mutation writes its event to the outbox in the same database transaction. No silent event loss.
4. **Soft delete only.** Parties are deactivated, never hard-deleted. Referential integrity for all consumers is preserved.
5. **Primary flag exclusivity.** At most one contact and one address per party is marked `is_primary = true`, enforced atomically within a transaction.
6. **Extension 1:1 integrity.** A company party has exactly one `party_companies` row; an individual party has exactly one `party_individuals` row. Created atomically with the base party.
7. **Display name is required.** Both company and individual creation validate that `display_name` is non-empty. Updates validate the same if `display_name` is provided.
8. **No runtime dependencies.** Party boots and functions without any other module running. It is a foundational, dependency-free service.

---

## API Surface (Summary)

Full OpenAPI contract: `contracts/party/party-v0.1.0.yaml`

### Parties
- `POST /api/party/companies` — Create a company party
- `POST /api/party/individuals` — Create an individual party
- `GET /api/party/parties` — List parties (base records, filterable by `include_inactive`)
- `GET /api/party/parties/search` — Search by name, type, status, external system/ID (paginated)
- `GET /api/party/parties/{id}` — Get party with extension, external refs, contacts, addresses
- `PUT /api/party/parties/{id}` — Update base party fields
- `POST /api/party/parties/{id}/deactivate` — Soft-delete (set status to inactive)

### Contacts
- `POST /api/party/parties/{party_id}/contacts` — Create contact linked to party
- `GET /api/party/parties/{party_id}/contacts` — List contacts for party
- `GET /api/party/contacts/{id}` — Get contact by ID
- `PUT /api/party/contacts/{id}` — Update contact
- `DELETE /api/party/contacts/{id}` — Delete contact

### Addresses
- `POST /api/party/parties/{party_id}/addresses` — Create address for party
- `GET /api/party/parties/{party_id}/addresses` — List addresses for party
- `GET /api/party/addresses/{id}` — Get address by ID
- `PUT /api/party/addresses/{id}` — Update address
- `DELETE /api/party/addresses/{id}` — Delete address

### Operational
- `GET /healthz` — Liveness probe
- `GET /api/health` — Health check (service name + version)
- `GET /api/ready` — Readiness probe (DB connectivity check)
- `GET /api/version` — Module identity and schema version
- `GET /metrics` — Prometheus metrics

---

## v2 Roadmap (Deferred)

| Feature | Rationale for Deferral |
|---------|----------------------|
| **Party Merge / Deduplication** | Combining duplicate party records requires conflict resolution UI and downstream notification to all consumers holding party UUIDs. Complex and not needed at launch. |
| **Relationship Graph** | Party-to-party relationships (parent company, subsidiary, partner, employee-of). Requires recursive graph queries and a relationship type taxonomy. |
| **Audit Log / History** | Versioned change tracking beyond event emission. Would require a separate history table or event sourcing approach. |
| **Active External Ref Sync** | Push/pull synchronization with Stripe, QuickBooks, etc. Requires webhook handling, conflict resolution, and per-system adapters. |
| **HTTP Idempotency Enforcement** | Infrastructure exists (`party_idempotency_keys` table) but is not yet wired to HTTP handlers. Straightforward to add. |
| **Bulk Import / Export** | CSV/JSON import of party records for migration from legacy systems. Needs validation, dedup, and error reporting. |
| **KYC / Verification** | Identity verification integration for compliance. Varies by jurisdiction and industry. |
| **Party Events for Contacts/Addresses** | Contact and address changes could emit events for downstream CRM sync. Not needed until a consumer requires it. |
| **`archived` Status Lifecycle** | The `archived` enum value exists in the schema but is not exposed in the API. Future use for long-term retention with restricted access. |

---

## Document Standards Reference

This document follows the revision and decision log standards defined at:
`docs/frontend/DOC-REVISION-STANDARDS.md`

---

## Decision Log

Every significant product, architecture, or standards decision is recorded here. Do not re-open a decision without adding a new row that supersedes the old one.

| Date | Decision | Rationale | Decided By |
|------|----------|-----------|-----------|
| 2026-02-19 | Unified base party table with typed extension tables (company, individual) | Search, listing, and status operations work identically for all party types; type-specific data is cleanly separated in 1:1 extensions; avoids duplicating shared fields | Platform Orchestrator |
| 2026-02-19 | Use `app_id` string scope instead of UUID `tenant_id` | Allows multiple vertical applications to share the Party service with distinct party universes; looser typing traded for flexibility in multi-app scenarios | Platform Orchestrator |
| 2026-02-19 | External references in dedicated table with uniqueness constraint | Prevents same external ID from being claimed by two parties; enables efficient system+ID lookups; more queryable than metadata JSONB | Platform Orchestrator |
| 2026-02-19 | Soft delete only — no hard delete endpoint | Preserves referential integrity for all modules holding party UUIDs; consumers react to deactivation event; matches platform pattern | Platform Orchestrator |
| 2026-02-20 | Contacts and addresses as separate tables (not embedded JSON) | Enables individual CRUD, primary flag management, and typed addresses; cleaner than JSONB arrays for querying and validation | Platform Orchestrator |
| 2026-02-20 | Primary flag exclusivity enforced in application code (not DB constraint) | Partial unique indexes for boolean primary flags are fragile; application-level enforcement within a transaction is clearer and more portable | Platform Orchestrator |
| 2026-02-20 | Address type as PostgreSQL enum (billing, shipping, registered, mailing, other) | Prevents invalid types at the database level; five types cover business address needs; `other` provides escape hatch | Platform Orchestrator |
| 2026-02-19 | Event schema version 1.0.0 with replay_safe=true | All party events are safe to replay (idempotent inserts/updates); version locked at 1.0.0 for initial release | Platform Orchestrator |
| 2026-02-19 | Guard-Mutation-Outbox atomicity on all write operations | Platform-wide pattern; prevents silent data changes without corresponding events; rollback is automatic if outbox write fails | Platform Orchestrator |
| 2026-02-19 | Inline address fields on base party table retained alongside address subresource | Backward compatibility with early consumers; inline fields serve as a "primary/default" address shortcut; addresses table is the canonical multi-address model | Platform Orchestrator |
| 2026-02-19 | No runtime dependencies on other modules | Party is a foundational service — it must boot and operate without AR, Payments, Subscriptions, or any other module running | Platform Orchestrator |
