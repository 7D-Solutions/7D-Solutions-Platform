# Consolidation Module — Product Vision & Technical Specification

**7D Solutions Platform**
**Status:** Vision Document + Technical Specification (v0.1.0)
**Last updated:** 2026-02-24

---

## Revision History

| Rev | Date | Changed By | Summary |
|-----|------|-----------|---------|
| 1.0 | 2026-02-24 | Platform Orchestrator | Initial vision doc — full module analysis from source, schema, integrations, tests, and API surface |

---

## The Business Problem

Any organization with multiple legal entities — subsidiaries, divisions, branches across regions — needs to produce **consolidated financial statements** that combine all entities into a single reporting view. This is not optional: GAAP and IFRS require it for group reporting, investor reporting, and regulatory filings.

The problem is multi-layered. Each entity may use a different chart of accounts (entity A calls it "1010-Cash", entity B calls it "CASH-001"). Each entity may operate in a different functional currency (USD, GBP, EUR). And intercompany transactions — where entity A sells to entity B — must be **eliminated** so the group doesn't double-count internal revenue and expenses.

Manual consolidation in spreadsheets is slow, error-prone, and untraceable. Enterprise ERP consolidation modules cost six figures and require months of configuration. Mid-market organizations — the ones growing fast enough to acquire subsidiaries — either can't afford it or live with brittle spreadsheet processes that break every quarter-close.

---

## What the Module Does

The Consolidation module is the **authoritative system for multi-entity financial consolidation** on the platform. It takes per-entity trial balances from the GL module and produces a unified consolidated trial balance, balance sheet, and profit & loss statement.

It answers four questions:
1. **What entities are in the group?** — A consolidation group defines which tenants (entities) are combined, their ownership percentages, and their consolidation method (full, proportional, equity).
2. **How do their accounts align?** — Chart of accounts (COA) mappings translate each entity's local account codes into a uniform group-level chart.
3. **What intercompany balances need elimination?** — Elimination rules identify intercompany receivables/payables, revenue/cost, and investment/equity that must be reversed in the consolidated view.
4. **What are the consolidated financials?** — A deterministic pipeline fetches entity trial balances, verifies period close hashes, maps accounts, translates currencies, applies eliminations, and caches the consolidated result.

---

## Who Uses This

The module is a platform service consumed by any vertical application that manages multi-entity groups. It does not have its own frontend — it exposes an API that frontends consume.

### Group Controller / CFO
- Defines consolidation groups and adds entities
- Maps entity-level accounts to group-level consolidated accounts
- Configures elimination rules for intercompany transactions
- Sets FX translation policies per entity
- Runs consolidation and reviews the consolidated trial balance
- Generates consolidated balance sheet and P&L

### Finance Team / Accountant
- Reviews intercompany matching results and elimination suggestions
- Validates group completeness (all entities have COA mappings and FX policies)
- Posts elimination journals to GL with exactly-once semantics
- Compares cached consolidated TB across periods

### System (Consolidation Engine)
- Fetches per-entity trial balances from GL via HTTP
- Verifies period close status and records close hashes for determinism
- Applies COA mapping, FX translation, and elimination rules
- Caches results in `csl_trial_balance_cache` for fast retrieval
- Computes deterministic `input_hash` from entity close hashes

---

## Design Principles

### GL is the Source of Truth for Entity Balances
The consolidation module never stores raw entity-level trial balances. It fetches them from GL at consolidation time via HTTP. This means GL remains the single source of truth for entity financials. Consolidation only stores derived, consolidated data in its own cache tables.

### Deterministic and Verifiable
Every consolidation run records entity close hashes and computes a deterministic `input_hash` from them (SHA-256 of sorted entity hashes). Running the same consolidation with the same closed periods produces identical results. The cache stores the `input_hash` so consumers can verify that cached data matches expected inputs.

### COA Mapping with Pass-Through
Entity account codes are mapped to group-level codes via explicit COA mappings. If no mapping exists for an account, it passes through unchanged — this avoids hard failures when entities add new accounts before the mapping is updated.

### Intercompany Elimination is Configurable, Not Hardcoded
Elimination rules are tenant-defined. Each rule specifies a rule type (intercompany_revenue_cost, intercompany_receivable_payable, intercompany_investment_equity, or custom) and the debit/credit account pair for the elimination journal. The matching engine uses these rules to find and match intercompany balances across entity pairs.

### Exactly-Once Elimination Posting
Elimination journals are posted to GL with SHA-256-based idempotency keys. If the same eliminations have already been posted for a group+period, the system returns the existing result without re-posting. This prevents duplicate elimination journals from accumulating.

### FX Translation Policy Without Live Rates
FX translation is policy-aware — each entity has a policy defining which rate type (closing, average, historical) to use for balance sheet, P&L, and equity items. However, actual FX rate lookup is deferred (currently 1:1 identity). The policy infrastructure is in place so that when an FX rates service is wired up, the module knows which rate type to request.

---

## MVP Scope (v0.1.x)

### In Scope
- Consolidation group management (CRUD, tenant-scoped)
- Group entity management with ownership percentage (basis points) and consolidation method (full/proportional/equity)
- COA mapping: entity account codes → group-level account codes
- Elimination rules: configurable debit/credit account pairs per rule type
- FX translation policies: per-entity policy for BS/P&L/equity rate types
- Group completeness validation (missing COA mappings, missing FX policies)
- Full consolidation pipeline: fetch GL TB → verify close hash → COA map → FX translate → eliminate → cache
- Deterministic input_hash for verifiable reruns
- Intercompany matching engine: in-memory entity pair matching by rule
- Elimination suggestion generation from intercompany matches
- Elimination posting to GL with exactly-once idempotency
- Consolidated trial balance cache (read and recompute)
- Consolidated balance sheet generation from cached TB
- Consolidated P&L generation from cached TB
- Integration clients for GL, AR, and AP
- Prometheus metrics (consolidation run counter, HTTP latency, consumer lag)
- Admin endpoints (projection status, consistency check)
- Docker deployment with health checks

### Explicitly Out of Scope for v1
- Live FX rates integration (rate lookup returns 1:1 identity as safe default)
- Minority interest calculations (proportional ownership applied but minority share not separated)
- Goodwill and fair value adjustments on acquisition
- Multi-level group hierarchies (sub-groups consolidated into parent groups)
- Period-over-period comparison reports
- Audit trail of consolidation changes (who changed what mapping, when)
- Active AR/AP integration for intercompany matching (clients exist but not wired into the matching pipeline)
- OpenAPI contract
- Frontend UI

---

## Technology Summary

| Layer | Technology | Notes |
|-------|-----------|-------|
| Language | Rust | Platform standard |
| HTTP framework | Axum | Port 8096 (default) |
| Database | PostgreSQL | Dedicated database (`consolidation_{app_id}_db`), SQLx for queries and migrations |
| Auth | JWT via platform `security` crate | Tenant-scoped, permission-gated mutations (`consolidation.mutate`) |
| GL integration | HTTP client (`reqwest`) | Fetches trial balances and period close status from GL service |
| AR integration | HTTP client (`reqwest`) | Fetches receivable summaries (client built, not yet wired) |
| AP integration | HTTP client (`reqwest`) | Fetches payable summaries (client built, not yet wired) |
| Hashing | SHA-256 (`sha2`) | Deterministic input_hash and elimination idempotency keys |
| Admin | Platform `projections` crate | Standardized admin endpoints |
| Metrics | Prometheus | `/metrics` endpoint |
| Crate | `consolidation` | Single crate, modular domain layout |

---

## Structural Decisions (The "Walls")

### 1. Consolidation fetches GL data via HTTP — no shared database
The module calls GL's existing API endpoints (`/api/gl/trial-balance`, `/api/gl/periods/{id}/close-status`, `/api/gl/journal-entries`) rather than directly accessing GL's database. This preserves module boundaries. GL can evolve its schema without breaking consolidation. The cost is network latency per entity per consolidation run, which is acceptable for a batch-oriented workflow.

### 2. Close hash verification gates every consolidation
Before processing an entity, the engine verifies the period is closed by fetching the close hash from GL. If a period is not closed, the consolidation fails with `PeriodNotClosed`. This prevents consolidating against in-flux data. The close hash is also recorded so that reruns can verify they're using the same snapshot.

### 3. COA mapping is per-entity, not global
Each entity can have different local account codes, and the mapping to group-level codes is per-entity. This handles real-world scenarios where acquired subsidiaries use completely different charts of accounts. The trade-off is more configuration work per entity, but the alternative (forcing all entities to use the same chart) is unrealistic.

### 4. Elimination rules operate on group-level accounts, not entity-level
Rules reference group-level consolidated account codes (the target side of COA mappings). This means elimination matching happens after COA mapping, on a uniform chart. If rules used entity-level codes, every rule would need to enumerate all possible source codes across entities.

### 5. Intercompany matching is in-memory, no DB writes
The matching engine takes entity balances and elimination rules as inputs and produces match suggestions as output. It writes nothing to the database. This makes it safe to run repeatedly (preview mode) without side effects. Posting eliminations is a separate, explicit action.

### 6. FX translation is policy-aware but rate-deferred
The FX policy table records which rate type each entity needs per financial statement section (closing for BS, average for P&L, historical for equity). The actual rate lookup returns 1:1 identity until an FX rates service is wired in. This lets the configuration be correct from day one while deferring the rates integration to a future bead.

### 7. Consolidated TB cache uses DELETE + INSERT, not upsert
Each consolidation run deletes the previous cache for the group+as_of and inserts fresh rows. This guarantees that stale rows from removed accounts don't persist. The trade-off is a brief window where the cache is empty during a rerun, but consolidation is a batch process, not a real-time query.

### 8. Tenant identity via X-App-Id header
The consolidation module extracts `tenant_id` from the `X-App-Id` header, consistent with other platform services. The group table has `tenant_id` scoping, and every query filters by it. Entity member tenant IDs (`entity_tenant_id`) are separate — they identify which tenants' GL data to fetch for consolidation.

---

## Domain Authority

Consolidation is the **source of truth** for:

| Domain Entity | Consolidation Authority |
|---------------|------------------------|
| **Consolidation Groups** | Group identity: name, reporting currency, fiscal year end month, active status. Scoped by parent tenant_id. |
| **Group Entities** | Membership: which tenant_ids belong to a group, their ownership percentage (basis points), consolidation method, and functional currency. |
| **COA Mappings** | Account translation: per-entity mapping from local account codes to group-level consolidated account codes. |
| **Elimination Rules** | Intercompany elimination configuration: rule type, debit/credit account pairs for elimination journals. |
| **FX Translation Policies** | Per-entity policy: which rate type (closing/average/historical) to use for BS, P&L, and equity translation. |
| **Consolidated Trial Balance** | Cached post-mapping, post-FX, post-elimination balances per group per as_of date. |
| **Consolidated Financial Statements** | Derived balance sheet and P&L from the consolidated TB cache. |
| **Intercompany Match Results** | Computed matches between entity pairs per elimination rule (in-memory, not persisted). |
| **Elimination Posting Log** | Exactly-once record of which elimination journals were posted to GL per group+period. |

Consolidation is **NOT** authoritative for:
- Entity-level trial balances, journal entries, or period close status (GL module owns this)
- Entity-level receivable or payable balances (AR/AP modules own these)
- FX exchange rates (future FX rates service would own this)
- User identity, permissions, or JWT claims (security crate owns this)

---

## Data Ownership

### Tables Owned by Consolidation

All tables are prefixed `csl_` to avoid clashes with source-module schemas. Scoped by `tenant_id` (on the group) and `group_id`.

| Table | Purpose | Key Fields |
|-------|---------|------------|
| **csl_groups** | Consolidation group definitions | `id`, `tenant_id`, `name`, `description`, `reporting_currency` (ISO 4217), `fiscal_year_end_month` (1-12), `is_active` |
| **csl_group_entities** | Group member entities | `id`, `group_id` (FK), `entity_tenant_id`, `entity_name`, `functional_currency`, `ownership_pct_bp` (1-10000), `consolidation_method` (full\|proportional\|equity), `is_active` |
| **csl_coa_mappings** | Account code translation | `id`, `group_id` (FK), `entity_tenant_id`, `source_account_code`, `target_account_code`, `target_account_name` |
| **csl_elimination_rules** | Elimination journal configuration | `id`, `group_id` (FK), `rule_name`, `rule_type` (intercompany_revenue_cost\|intercompany_receivable_payable\|intercompany_investment_equity\|custom), `debit_account_code`, `credit_account_code`, `description`, `is_active` |
| **csl_fx_policies** | FX translation rate-type policies | `id`, `group_id` (FK), `entity_tenant_id`, `bs_rate_type` (closing\|average\|historical), `pl_rate_type`, `equity_rate_type`, `fx_rate_source` |
| **csl_trial_balance_cache** | Consolidated TB results | `id`, `group_id` (FK), `as_of` (DATE), `account_code`, `account_name`, `currency`, `debit_minor` (BIGINT), `credit_minor` (BIGINT), `net_minor` (BIGINT), `input_hash`, `computed_at` |
| **csl_statement_cache** | Consolidated financial statement lines | `id`, `group_id` (FK), `statement_type` (income_statement\|balance_sheet), `as_of`, `line_code`, `line_label`, `currency`, `amount_minor` (BIGINT), `input_hash`, `computed_at` |
| **csl_elimination_postings** | Exactly-once elimination posting log | `id`, `group_id` (FK), `period_id`, `idempotency_key` (SHA-256), `journal_entry_ids` (JSONB), `suggestion_count`, `total_amount_minor`, `posted_at` |

**Monetary Precision:** All monetary amounts use **integer minor units** (e.g., `debit_minor` in cents). Currency stored as 3-letter ISO 4217 code.

**Uniqueness Constraints:**
- `csl_groups`: unique `(tenant_id, name)`
- `csl_group_entities`: unique `(group_id, entity_tenant_id)`
- `csl_coa_mappings`: unique `(group_id, entity_tenant_id, source_account_code)`
- `csl_elimination_rules`: unique `(group_id, rule_name)`
- `csl_fx_policies`: unique `(group_id, entity_tenant_id)`
- `csl_trial_balance_cache`: unique `(group_id, as_of, account_code, currency)`
- `csl_statement_cache`: unique `(group_id, statement_type, as_of, line_code, currency)`
- `csl_elimination_postings`: unique `(group_id, period_id, idempotency_key)`

### Data NOT Owned by Consolidation

Consolidation **MUST NOT** store:
- Entity-level journal entries, trial balances, or period metadata (GL module)
- Entity-level receivable or payable transactions (AR/AP modules)
- FX exchange rates or rate history (future FX rates service)
- User credentials, session data, or permission assignments (security crate)

---

## Consolidation Pipeline

```
For each entity in group:
  1. Verify period is closed (GL close-status API) → record close_hash
  2. Fetch trial balance (GL trial-balance API)
  3. Apply COA mapping (source → target account codes)
  4. Apply FX translation (functional → reporting currency)
  5. Accumulate into consolidated ledger (BTreeMap for sorted output)

After all entities:
  6. Apply elimination rules (min of debit/credit balances)
  7. Compute deterministic input_hash from sorted entity close hashes
  8. Cache result (DELETE + INSERT into csl_trial_balance_cache)
```

### Pipeline Invariants
- If any entity's period is not closed → fail with `PeriodNotClosed`
- COA mapping is optional per account — unmapped accounts pass through
- FX translation uses 1:1 identity until FX rates service is wired
- Elimination applies in-place on the consolidated ledger using `min(debit_balance, credit_balance)`
- The pipeline is deterministic: same closed periods → same output → same input_hash

---

## Events Produced

*None in v0.1.x. The consolidation module does not use the platform event bus. It communicates with GL synchronously via HTTP and stores results in its own cache tables.*

---

## Events Consumed

*None in v0.1.x. The consolidation module does not subscribe to any NATS events. It actively pulls data from GL on demand.*

---

## Integration Points

### GL (HTTP, Read + Write)

**Read:** Consolidation fetches entity trial balances and period close status from GL:
- `GET /api/gl/trial-balance?tenant_id=X&period_id=Y&currency=Z`
- `GET /api/gl/periods/{id}/close-status?tenant_id=X`

**Write:** Consolidation posts elimination journals to GL:
- `POST /api/gl/journal-entries` with `source_module: "consolidation-elimination"`

The GL client (`integrations::gl::client::GlClient`) handles all HTTP communication. Base URL is configured via `GL_BASE_URL` env var (default: `http://localhost:8080`).

### AR (HTTP Client Built, Not Wired)

An AR client exists (`integrations::ar::client::ArClient`) that can fetch receivable summaries per customer. This is intended for future intercompany matching — identifying receivables where the customer is another entity in the group. Not currently wired into the matching pipeline.

### AP (HTTP Client Built, Not Wired)

An AP client exists (`integrations::ap::client::ApClient`) that can fetch payable summaries per vendor. Symmetric with the AR client — intended for intercompany payable matching. Not currently wired.

---

## Invariants

1. **Group-level tenant isolation.** Groups are scoped by `tenant_id`. One tenant cannot see or modify another tenant's consolidation groups.
2. **Close hash verification.** Every consolidation run verifies each entity's period is closed before processing. No consolidation against open periods.
3. **Deterministic reruns.** Same closed periods with same close hashes produce identical consolidated output and identical `input_hash`.
4. **Exactly-once elimination posting.** Elimination journals use SHA-256 idempotency keys (group + period + suggestion fingerprints). Reposting returns the existing result without creating duplicate journals.
5. **Monetary precision via integer minor units.** All amounts stored as BIGINT minor units. No floating-point arithmetic on monetary values except FX translation (where rounding is explicit).
6. **No silent failures on entity verification.** If a period is not closed, consolidation fails explicitly — no partial consolidation with missing entities.
7. **COA mapping pass-through is safe.** Unmapped accounts pass through with their source code and name. This prevents hard failures when entities add accounts before mapping is updated.
8. **Cache is fully replaceable.** Each consolidation run DELETEs previous cache rows for the group+as_of and INSERTs fresh data. No stale rows survive.

---

## API Surface (Summary)

No OpenAPI contract exists yet. The following routes are defined in the HTTP router.

### Groups
- `POST /api/consolidation/groups` — Create consolidation group
- `GET /api/consolidation/groups` — List groups (tenant-scoped, filterable by active status)
- `GET /api/consolidation/groups/{id}` — Get group detail
- `PUT /api/consolidation/groups/{id}` — Update group
- `DELETE /api/consolidation/groups/{id}` — Delete group (cascades to entities, mappings, rules, policies)
- `GET /api/consolidation/groups/{id}/validate` — Check group completeness (missing COA/FX)

### Entities
- `POST /api/consolidation/groups/{group_id}/entities` — Add entity to group
- `GET /api/consolidation/groups/{group_id}/entities` — List entities
- `GET /api/consolidation/entities/{id}` — Get entity detail
- `PUT /api/consolidation/entities/{id}` — Update entity
- `DELETE /api/consolidation/entities/{id}` — Remove entity from group

### COA Mappings
- `POST /api/consolidation/groups/{group_id}/coa-mappings` — Create mapping
- `GET /api/consolidation/groups/{group_id}/coa-mappings` — List mappings (filterable by entity)
- `DELETE /api/consolidation/coa-mappings/{id}` — Delete mapping

### Elimination Rules
- `POST /api/consolidation/groups/{group_id}/elimination-rules` — Create rule
- `GET /api/consolidation/groups/{group_id}/elimination-rules` — List rules
- `GET /api/consolidation/elimination-rules/{id}` — Get rule detail
- `PUT /api/consolidation/elimination-rules/{id}` — Update rule
- `DELETE /api/consolidation/elimination-rules/{id}` — Delete rule

### FX Policies
- `PUT /api/consolidation/groups/{group_id}/fx-policies` — Upsert FX policy
- `GET /api/consolidation/groups/{group_id}/fx-policies` — List policies
- `DELETE /api/consolidation/fx-policies/{id}` — Delete policy

### Consolidation Engine
- `POST /api/consolidation/groups/{group_id}/consolidate` — Run full consolidation pipeline
- `GET /api/consolidation/groups/{group_id}/trial-balance?as_of=YYYY-MM-DD` — Get cached consolidated TB

### Intercompany & Eliminations
- `POST /api/consolidation/groups/{group_id}/intercompany-match` — Run intercompany matching
- `POST /api/consolidation/groups/{group_id}/eliminations` — Post elimination journals to GL

### Financial Statements
- `GET /api/consolidation/groups/{group_id}/pl?as_of=YYYY-MM-DD` — Consolidated P&L
- `GET /api/consolidation/groups/{group_id}/balance-sheet?as_of=YYYY-MM-DD` — Consolidated balance sheet

### Admin
- `POST /api/consolidation/admin/projection-status` — Projection status (requires X-Admin-Token)
- `POST /api/consolidation/admin/consistency-check` — Consistency check (requires X-Admin-Token)
- `GET /api/consolidation/admin/projections` — List projections (requires X-Admin-Token)

### Operational
- `GET /healthz` — Liveness probe
- `GET /api/health` — Health check (service name + version)
- `GET /api/ready` — Readiness probe (verifies DB connectivity)
- `GET /api/version` — Module identity and schema version
- `GET /metrics` — Prometheus metrics

---

## Document Standards Reference

This document follows the revision and decision log standards defined at:
`docs/frontend/DOC-REVISION-STANDARDS.md`

---

## Decision Log

Every significant product, architecture, or standards decision is recorded here. Do not re-open a decision without adding a new row that supersedes the old one.

| Date | Decision | Rationale | Decided By |
|------|----------|-----------|-----------|
| 2026-02-18 | Fetch GL data via HTTP, not shared database | Preserves module boundaries; GL can evolve its schema independently; consolidation only caches derived data | Platform Orchestrator |
| 2026-02-18 | Close hash verification gates every consolidation run | Prevents consolidating against in-flux data; close hash enables deterministic rerun verification | Platform Orchestrator |
| 2026-02-18 | COA mapping is per-entity with pass-through for unmapped accounts | Acquired subsidiaries use different charts; pass-through avoids hard failures when new accounts are added | Platform Orchestrator |
| 2026-02-18 | Elimination rules operate on group-level (post-COA-mapping) account codes | Uniform chart after mapping means rules don't need to enumerate all entity-level codes | Platform Orchestrator |
| 2026-02-18 | Intercompany matching is pure in-memory computation, no DB writes | Safe to run repeatedly in preview mode; posting is a separate explicit action | Platform Orchestrator |
| 2026-02-18 | FX translation is policy-aware but rate-deferred (1:1 identity) | Policy infrastructure built early; actual rate service wired later; safe default for same-currency groups | Platform Orchestrator |
| 2026-02-18 | Cache uses DELETE + INSERT, not upsert | Guarantees stale rows from removed accounts don't persist; acceptable for batch workflow | Platform Orchestrator |
| 2026-02-18 | Exactly-once elimination posting via SHA-256 idempotency key | Prevents duplicate elimination journals; key derived from group + period + suggestion fingerprints | Platform Orchestrator |
| 2026-02-18 | Tables prefixed `csl_` to avoid namespace collisions | Multiple modules may share a database in some deployment configurations; prefix prevents table name conflicts | Platform Orchestrator |
| 2026-02-18 | Ownership percentage stored as basis points (10000 = 100%) | Integer arithmetic avoids floating-point rounding; basis points are standard in financial systems | Platform Orchestrator |
| 2026-02-18 | Three consolidation methods: full, proportional, equity | Covers IFRS/GAAP requirements; full for subsidiaries, proportional for joint ventures, equity for associates | Platform Orchestrator |
| 2026-02-18 | AR and AP clients built as integration seams, not wired | Enables future intercompany matching against live receivable/payable data; no premature coupling | Platform Orchestrator |
| 2026-02-18 | Balance sheet account classification by code prefix (1=Assets, 2=Liabilities, 3=Equity) | Simple, consistent with standard chart of accounts conventions; P&L uses 4=Revenue, 5=COGS, 6=Expenses | Platform Orchestrator |
| 2026-02-18 | No mocking in tests — integrated tests against real Postgres | Platform-wide standard; config CRUD tests hit real consolidation database | Platform Orchestrator |
