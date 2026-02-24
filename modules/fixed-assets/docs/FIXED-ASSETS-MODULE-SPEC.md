# Fixed-Assets Module — Product Vision & Technical Specification

**7D Solutions Platform**
**Status:** Vision Document + Technical Specification (v0.1.x)
**Last updated:** 2026-02-24

---

## Revision History

| Rev | Date | Changed By | Summary |
|-----|------|-----------|---------|
| 1.0 | 2026-02-24 | Platform Orchestrator | Initial technical spec — schema, depreciation engine, disposal lifecycle, AP capitalization, events, API, invariants, integration points. Documented from source code. |

---

## The Business Problem

Every organization that owns long-lived assets — vehicles, furniture, equipment, buildings, computers — faces the same accounting challenge: **those assets lose value over time, and the loss must be tracked accurately for financial reporting, tax compliance, and operational decision-making.**

A $50,000 delivery truck bought today is not worth $50,000 in five years. Accounting standards (GAAP, IFRS) require that the cost be spread over the asset's useful life as depreciation expense. When the asset is sold, scrapped, or written down, the gain or loss must be computed from the asset's net book value at that moment — not its original cost.

Small and mid-size businesses often track this in spreadsheets, which breaks down as asset counts grow: depreciation runs are error-prone, disposal calculations are manual, and there is no audit trail connecting fixed-asset movements to the general ledger. Worse, when a vendor invoice contains both expense items and capital expenditures, the capitalization decision is manual and easy to miss.

---

## What the Module Does

The Fixed-Assets module is the **authoritative system for capital asset accounting**: acquisition, categorization, depreciation, disposal, and impairment. It answers five questions:

1. **What do we own?** — An asset register with tag, name, category, acquisition cost, depreciation parameters, and status lifecycle.
2. **How is it organized?** — Asset categories that define default depreciation method, useful life, salvage percentage, and GL account references for journal posting.
3. **How much has it depreciated?** — A period-by-period depreciation schedule computed by a straight-line engine, with batch depreciation runs that post periods up to a given date.
4. **What happened when we got rid of it?** — Disposal and impairment records that compute gain/loss from net book value vs. proceeds, and emit GL-ready event data.
5. **Did that AP bill create a capital asset?** — An event consumer that listens for AP bill approvals and automatically capitalizes bill lines whose GL account maps to an asset category.

---

## Who Uses This

The module is a platform service consumed by any vertical application that manages capitalized assets. It does not have its own frontend — it exposes an API that frontends consume.

### Controller / CFO
- Defines asset categories with depreciation defaults and GL account mappings
- Runs monthly/quarterly depreciation batches
- Reviews disposal gain/loss reports
- Audits AP-to-asset capitalization linkages

### Asset Manager / Operations
- Registers assets with tags, locations, departments, and responsible persons
- Puts assets in service (sets in_service_date to begin depreciation)
- Tracks asset status through the lifecycle (draft → active → fully_depreciated → disposed)
- Initiates disposals (sale, scrap, write-off, transfer) and impairments

### AP / Procurement
- Approves vendor bills in the AP module
- Capital expenditure lines are automatically detected and capitalized into draft assets
- Non-capex lines are silently skipped

### GL Consumer (Downstream)
- Subscribes to `depreciation_run_completed` events to post balanced journal entries (DR Depreciation Expense, CR Accumulated Depreciation)
- Subscribes to `asset_disposed` events to derecognize assets and record gain/loss
- Never called by Fixed-Assets — purely event-driven

### Maintenance Module (Cross-Reference)
- Maintenance's `maintainable_assets` table can optionally reference a fixed-asset UUID via `fixed_asset_ref`
- This correlates maintenance costs to capitalized assets for reporting
- Fixed-Assets never calls Maintenance at runtime

---

## Design Principles

### Category-Driven Defaults
Every asset belongs to a category. Categories carry default depreciation method, useful life, salvage percentage, and GL account references. When creating an asset, any parameter not explicitly provided inherits from the category. This ensures consistency across asset classes while allowing per-asset overrides.

### Immutable Financial Parameters Post-Creation
Once an asset is created, its acquisition cost, depreciation method, useful life, and salvage value are immutable. Only descriptive fields (name, location, department, responsible person, notes) can be updated. Changes to financial parameters require explicit lifecycle events (disposal, impairment) — not quiet edits.

### GL Account References, Not GL Coupling
Categories and assets carry GL account reference strings (e.g., "1500" for fixed-asset account, "6100" for depreciation expense). These are opaque identifiers passed through events for the GL consumer to interpret. Fixed-Assets never calls GL, never stores journal entries, and never validates account codes. The GL consumer owns the posting logic.

### AP Integration via Event Consumption
Fixed-Assets subscribes to `ap.vendor_bill_approved` events and automatically creates draft assets for bill lines whose `gl_account_code` matches an active category's `asset_account_ref`. This is an anti-corruption layer: the consumer mirrors AP payload types locally, never queries the AP database, and uses idempotency keys to handle event replay safely.

### Standalone First, Integrate Later
The module boots and runs without AP, GL, Maintenance, or any other service. AP capitalization is an event-driven add-on. GL posting is a downstream consumer. Every integration degrades gracefully — if NATS is unavailable, events accumulate in the outbox.

### No Silent Failures
Every state-changing mutation writes its event to the outbox in the same database transaction. If the event wasn't written, the state change didn't happen. The outbox publisher runs as a background task, polling every 500ms.

---

## MVP Scope (v0.1.x)

### In Scope
- Asset categories with default depreciation parameters and GL account references (CRUD)
- Asset register with full lifecycle tracking (draft → active → fully_depreciated → disposed/impaired)
- Straight-line depreciation engine (pure computation, no I/O)
- Period-by-period depreciation schedule generation (idempotent)
- Batch depreciation runs that post unposted periods up to an as_of_date (idempotent)
- Asset disposals: sale, scrap, impairment, write-off, transfer — with gain/loss computation
- AP capitalization: event consumer creates assets from approved vendor bill lines
- Capitalization linkage table for audit trail (bill_id + line_id → asset_id)
- 5 domain events emitted via outbox (see Events Produced)
- GL-ready payloads in depreciation and disposal events (account refs + amounts)
- Admin endpoints for projection status and consistency checks
- Prometheus metrics (assets created, depreciation runs, disposals, SLO histograms)
- Docker build with cargo-chef caching

### Explicitly Out of Scope for v1
- Declining-balance and units-of-production depreciation methods (engine stubs exist, not implemented)
- Asset revaluation (IFRS fair-value adjustments)
- Partial disposals (disposing a component of an asset)
- Asset transfers between tenants
- Bulk asset import
- Active GL consumer (platform-side NATS subscriber that posts journal entries)
- Frontend UI (consumed via API by vertical apps or TCP)

---

## Technology Summary

| Layer | Technology | Notes |
|-------|-----------|-------|
| Language | Rust | Platform standard |
| HTTP framework | Axum 0.8 | Port 8095 (default) |
| Database | PostgreSQL | Dedicated database, SQLx for queries and migrations |
| Event bus | NATS | Via platform `event-bus` crate; falls back to in-memory bus |
| Auth | JWT via platform `security` crate | Tenant-scoped, role-based; mutations require `fixed_assets:mutate` permission |
| Outbox | Platform outbox pattern | `fa_events_outbox` table, 500ms poll interval |
| Projections | Platform `projections` crate | Admin endpoints for projection status |
| Metrics | Prometheus | `/metrics` endpoint via custom registry |
| Crate | `fixed-assets` | Single crate, standard module layout |

---

## Structural Decisions (The "Walls")

### 1. Categories own GL account references — not assets
Assets optionally carry per-asset GL account overrides (`asset_account_ref`, `depreciation_expense_ref`, `accum_depreciation_ref`), but these default to NULL. The category's references are the primary source. This means changing a GL mapping for an entire asset class is a single category update, not a bulk asset update.

### 2. Depreciation schedule is pre-computed, not calculated on the fly
When an asset enters service, `generate_schedule` computes all periods upfront and stores them in `fa_depreciation_schedules`. Depreciation runs then simply mark periods as posted. This makes runs fast (one UPDATE, no computation), auditable (every period is a row), and idempotent (ON CONFLICT DO NOTHING on schedule insert, is_posted check on run).

### 3. Straight-line engine is pure — no I/O
The depreciation computation (`engine::compute_straight_line`) takes in-service date, cost, salvage, and life months, and returns a `Vec<PeriodEntry>`. No database calls, no side effects. This makes it fully unit-testable and keeps the computation separate from persistence.

### 4. Depreciation runs are batch operations, not per-asset
A run posts all unposted periods across all assets for a tenant up to `as_of_date` in a single transaction. This ensures consistency: all assets in a tenant are deprecated to the same point in time. The run record tracks how many assets and periods were processed.

### 5. Disposals compute gain/loss at mutation time
When disposing an asset, gain/loss is computed immediately from `net_book_value_minor` and `proceeds_minor`. The asset's NBV is zeroed and its status set to `disposed` or `impaired`. This is a one-way operation — there is no "undo disposal" flow.

### 6. AP capitalization uses an anti-corruption layer
The consumer mirrors AP event payload types locally (`VendorBillApprovedPayload`, `ApprovedGlLine`) rather than importing AP crate types. This insulates Fixed-Assets from AP schema changes. The matching logic is simple: does the bill line's `gl_account_code` match any active category's `asset_account_ref`?

### 7. Idempotency at every integration point
- Schedule generation: `ON CONFLICT (asset_id, period_number) DO NOTHING`
- Depreciation runs: already-posted periods skipped via `WHERE is_posted = FALSE`
- Disposals: if asset is already disposed/impaired, returns existing disposal record
- AP capitalization: `UNIQUE (tenant_id, bill_id, line_id)` prevents duplicate assets on replay
- Outbox: `ON CONFLICT (event_id) DO NOTHING`

### 8. Status stored as TEXT, not PostgreSQL ENUM
Migrations 8 and 9 converted `fa_asset_status` and `fa_run_status` ENUMs to plain TEXT columns. This avoids the SQLx `FromRow` decoding issue with custom PG ENUM types and makes the schema align directly with Rust `String` fields.

### 9. Tenant isolation via tenant_id on every table
Standard platform multi-tenant pattern. Every table has `tenant_id` as a non-nullable field. Every query filters by `tenant_id`. Indexes include `tenant_id` as a leading column.

### 10. No mocking in tests
Integration tests hit real Postgres (default: `postgres://fixed_assets_user:fixed_assets_pass@localhost:5445/fixed_assets_db`). Tests that mock the database or event bus test nothing useful. This is a platform-wide standard.

---

## Domain Authority

Fixed-Assets is the **source of truth** for:

| Domain Entity | Fixed-Assets Authority |
|---------------|----------------------|
| **Asset Categories** | Depreciation defaults (method, useful life, salvage %), GL account references (asset account, depreciation expense, accumulated depreciation, gain/loss), category codes. |
| **Fixed Assets** | Asset register: tag, name, description, acquisition date/cost, in-service date, depreciation method/life/salvage, accumulated depreciation, net book value, status lifecycle, location, department, serial number, vendor, purchase order reference. |
| **Depreciation Schedules** | Period-by-period planned depreciation for each asset: amounts, cumulative totals, remaining book value, posting status. |
| **Depreciation Runs** | Batch execution records: as-of date, periods posted, total depreciation, completion status. |
| **Disposals & Impairments** | Disposal records: type (sale/scrap/impairment/write-off/transfer), disposal date, NBV at disposal, proceeds, gain/loss, GL entry data. |
| **AP Capitalization Linkage** | Mapping from AP bill lines (bill_id + line_id) to created assets, with GL account code and amount for audit trail. |

Fixed-Assets is **NOT** authoritative for:
- Vendor bill approval status or payment terms (AP module owns this)
- GL journal entries or account balances (GL module owns this)
- Maintenance schedules, work orders, or maintenance costs (Maintenance module owns this)
- Inventory stock levels or spare parts (Inventory module owns this)

---

## Data Ownership

### Tables Owned by Fixed-Assets

All tables use `tenant_id` for multi-tenant isolation. Every query **MUST** filter by `tenant_id`.

| Table | Purpose | Key Fields |
|-------|---------|------------|
| **fa_categories** | Asset category definitions with depreciation defaults and GL account refs | `id`, `tenant_id`, `code` (unique per tenant), `name`, `default_method`, `default_useful_life_months`, `default_salvage_pct_bp`, `asset_account_ref`, `depreciation_expense_ref`, `accum_depreciation_ref`, `gain_loss_account_ref`, `is_active` |
| **fa_assets** | Core asset register | `id`, `tenant_id`, `category_id` (FK), `asset_tag` (unique per tenant), `name`, `status` (draft\|active\|fully_depreciated\|disposed\|impaired), `acquisition_date`, `in_service_date`, `acquisition_cost_minor`, `currency`, `depreciation_method`, `useful_life_months`, `salvage_value_minor`, `accum_depreciation_minor`, `net_book_value_minor`, GL account override fields, location/department/serial/vendor fields |
| **fa_depreciation_schedules** | Period-by-period depreciation plan | `id`, `tenant_id`, `asset_id` (FK), `period_number`, `period_start`, `period_end`, `depreciation_amount_minor`, `cumulative_depreciation_minor`, `remaining_book_value_minor`, `is_posted`, `posted_by_run_id` |
| **fa_depreciation_runs** | Batch depreciation execution records | `id`, `tenant_id`, `as_of_date`, `status` (pending\|running\|completed\|failed), `assets_processed`, `periods_posted`, `total_depreciation_minor`, `idempotency_key` |
| **fa_disposals** | Asset disposal and impairment records | `id`, `tenant_id`, `asset_id` (FK), `disposal_type` (sale\|scrap\|impairment\|write_off\|transfer), `disposal_date`, `net_book_value_at_disposal_minor`, `proceeds_minor`, `gain_loss_minor`, `journal_entry_ref`, `is_posted` |
| **fa_events_outbox** | Transactional outbox for event publishing | Standard outbox schema with `event_id`, `event_type`, `aggregate_type`, `aggregate_id`, `tenant_id`, `payload` (JSONB) |
| **fa_processed_events** | Consumer deduplication tracking | `event_id`, `event_type`, `processor`, `processed_at` |
| **fa_idempotency_keys** | HTTP-level idempotency for API callers | `tenant_id`, `idempotency_key`, `request_hash`, `response_body`, `status_code`, `expires_at` |
| **fa_ap_capitalizations** | AP bill line → asset linkage for audit trail | `id`, `tenant_id`, `bill_id`, `line_id` (unique per tenant+bill+line), `asset_id` (FK), `gl_account_code`, `amount_minor`, `source_ref` |

**Monetary Precision:** All monetary amounts use **integer minor units** (e.g., `acquisition_cost_minor` in cents). Currency stored as lowercase 3-letter ISO 4217 code (default: `usd`).

**Salvage Percentage:** Stored as basis points (0–10000, where 10000 = 100%) in categories. Individual assets store `salvage_value_minor` as an absolute amount.

### Data NOT Owned by Fixed-Assets
Fixed-Assets **MUST NOT** store:
- AP vendor bill details, approval workflows, or payment terms
- GL journal entries, account balances, or chart-of-accounts definitions
- Maintenance work orders, schedules, or costs
- Inventory quantities, lot codes, or reorder points

---

## Asset Status Lifecycle

```
draft ──→ active ──→ fully_depreciated
  |          |              |
  |          v              v
  |       disposed      disposed
  |          |              |
  v          v              v
disposed  (terminal)    (terminal)
  |
  v
impaired (impairment only)
```

### Status Transitions

| From | Allowed To | Trigger |
|------|-----------|---------|
| draft | active | Asset put in service (in_service_date set) |
| draft | disposed | Direct disposal before service |
| active | fully_depreciated | All depreciation periods posted |
| active | disposed | Sale, scrap, write-off, or transfer |
| active | impaired | Impairment write-down |
| fully_depreciated | disposed | Disposal after full depreciation |

**Terminal states:** `disposed` and `impaired` — no further transitions.

**Deactivation:** The `deactivate` API endpoint sets status to `disposed`. It is idempotent — calling it on an already-disposed or impaired asset returns the current state without modification.

---

## Depreciation Engine

### Straight-Line Method (Implemented)

The engine (`domain::depreciation::engine`) is a pure function with no I/O:

```
monthly_amount = (acquisition_cost - salvage_value) / useful_life_months
```

- Periods are **full calendar months** anchored to the first day of the in-service month
- Integer division is used; the **last period absorbs the remainder** so cumulative total exactly equals the depreciable amount
- Returns empty schedule when depreciable amount is zero or useful life is zero
- Period dates handle leap years and year crossings correctly

### Schedule Generation

`DepreciationService::generate_schedule` computes all periods via the engine and inserts them into `fa_depreciation_schedules`. Uses `ON CONFLICT (asset_id, period_number) DO NOTHING` for idempotency — safe to call multiple times.

### Depreciation Runs

`DepreciationService::run` posts all unposted periods up to `as_of_date` in a single transaction:
1. Insert run record in `running` status
2. `UPDATE fa_depreciation_schedules SET is_posted = TRUE WHERE period_end <= as_of_date AND is_posted = FALSE`
3. Compute stats (assets processed, periods posted, total depreciation)
4. Query GL entry data (joins schedules → assets → categories for account refs)
5. Emit `depreciation_run_completed` outbox event with GL entry payload
6. Commit

Idempotent: re-running for the same `as_of_date` produces a run with `periods_posted = 0`.

---

## Events Produced

All events are written to `fa_events_outbox` atomically with the triggering mutation. The background publisher sends them to NATS with subject `{aggregate_type}.{event_type}`.

| Event | Subject | Trigger | Key Payload Fields |
|-------|---------|---------|-------------------|
| `asset_created` | `fa_asset.asset_created` | Asset created (manual or from AP capitalization) | `asset_id`, `tenant_id`, `asset_tag`, `category_id`, `acquisition_cost_minor`, `currency` |
| `asset_updated` | `fa_asset.asset_updated` | Asset descriptive fields updated | `asset_id`, `tenant_id` |
| `asset_deactivated` | `fa_asset.asset_deactivated` | Asset deactivated (disposed via API) | `asset_id`, `tenant_id`, `previous_status` |
| `depreciation_run_completed` | `fa_depreciation_run.depreciation_run_completed` | Depreciation run completed | `run_id`, `tenant_id`, `as_of_date`, `periods_posted`, `total_depreciation_minor`, `gl_entries[]` (per-entry: `entry_id`, `asset_id`, `period_end`, `depreciation_amount_minor`, `expense_account_ref`, `accum_depreciation_ref`) |
| `asset_disposed` | `fa_disposal.asset_disposed` | Asset disposed or impaired | `disposal_id`, `asset_id`, `tenant_id`, `disposal_type`, `disposal_date`, `gl_data` (acquisition cost, accum depreciation, NBV, proceeds, gain/loss, account refs) |
| `category_created` | `fa_category.category_created` | Asset category created | `category_id`, `tenant_id`, `code` |

---

## Events Consumed

| Event | Source | NATS Subject | Action |
|-------|--------|-------------|--------|
| `ap.vendor_bill_approved` | AP module | `ap.events.ap.vendor_bill_approved` | For each GL line where `gl_account_code` matches an active category's `asset_account_ref`: create a draft asset and record the capitalization linkage. Non-capex lines are skipped. Idempotent on replay. |

---

## Integration Points

### AP (Event-Driven, One-Way Inbound)

Fixed-Assets subscribes to `ap.vendor_bill_approved` via NATS. The consumer uses an anti-corruption layer — it mirrors AP payload types locally (`VendorBillApprovedPayload`, `ApprovedGlLine`) and never imports AP crate types. For each bill line whose `gl_account_code` matches an active category's `asset_account_ref`, a draft asset is created with:
- `asset_tag`: `AP-{line_id}`
- `name`: vendor invoice reference
- `acquisition_cost_minor`: line amount
- Category defaults for depreciation parameters

The `fa_ap_capitalizations` table provides the audit trail: `(tenant_id, bill_id, line_id)` → `asset_id`.

**Fixed-Assets never writes to the AP database.**

### GL (Event-Driven, One-Way Outbound)

`depreciation_run_completed` carries per-entry GL data:
- `expense_account_ref` (from category's `depreciation_expense_ref`)
- `accum_depreciation_ref` (from category's `accum_depreciation_ref`)
- `depreciation_amount_minor` and `currency`

A GL consumer (not part of this module) subscribes and posts:
- DR Depreciation Expense (`expense_account_ref`)
- CR Accumulated Depreciation (`accum_depreciation_ref`)

`asset_disposed` carries:
- `acquisition_cost_minor`, `accum_depreciation_minor`, `net_book_value_minor`
- `proceeds_minor`, `gain_loss_minor`
- `asset_account_ref`, `accum_depreciation_ref`, `gain_loss_account_ref`

A GL consumer posts the derecognition entries.

**Fixed-Assets never calls GL.**

### Maintenance (Cross-Reference, No Runtime Dependency)

Maintenance's `maintainable_assets.fixed_asset_ref` optionally references a fixed-asset UUID. This allows maintenance costs to be correlated with capitalized assets for reporting. **Fixed-Assets never calls Maintenance.**

---

## Invariants

1. **Tenant isolation is unbreakable.** Every query filters by `tenant_id`. No cross-tenant data leakage.
2. **Asset tag uniqueness per tenant.** `UNIQUE (tenant_id, asset_tag)` enforced at the database level.
3. **Category code uniqueness per tenant.** `UNIQUE (tenant_id, code)` enforced at the database level.
4. **Outbox atomicity.** Every state-changing mutation writes its event to `fa_events_outbox` in the same database transaction. No silent event loss.
5. **Financial parameters are immutable post-creation.** Acquisition cost, depreciation method, useful life, and salvage value cannot be changed via the update API. Only descriptive fields (name, description, location, department, responsible_person, notes) are mutable.
6. **Depreciation schedule is idempotent.** `ON CONFLICT (asset_id, period_number) DO NOTHING` prevents duplicate schedule rows.
7. **Depreciation runs are idempotent.** Already-posted periods are skipped; re-running for the same date produces zero new postings.
8. **Disposal is idempotent.** Disposing an already-disposed/impaired asset returns the existing disposal record without creating a duplicate.
9. **AP capitalization is idempotent.** `UNIQUE (tenant_id, bill_id, line_id)` prevents duplicate assets on event replay.
10. **Monetary precision.** All monetary values are integer minor units (cents). No floating-point arithmetic anywhere in the module.
11. **Cumulative depreciation exactly equals depreciable amount.** The last period absorbs integer-division remainder, guaranteeing the sum of all period amounts equals `acquisition_cost - salvage_value`.
12. **No forced dependencies.** The module boots and functions without AP, GL, Maintenance, or Notifications running. All integrations are event-driven and degrade gracefully.

---

## API Surface (Summary)

### Categories
- `POST /api/fixed-assets/categories` — Create asset category *(requires fixed_assets:mutate)*
- `PUT /api/fixed-assets/categories/:id` — Update category *(requires fixed_assets:mutate)*
- `DELETE /api/fixed-assets/categories/:tenant_id/:id` — Deactivate category (soft delete) *(requires fixed_assets:mutate)*
- `GET /api/fixed-assets/categories/:tenant_id/:id` — Get category
- `GET /api/fixed-assets/categories/:tenant_id` — List active categories

### Assets
- `POST /api/fixed-assets/assets` — Create asset *(requires fixed_assets:mutate)*
- `PUT /api/fixed-assets/assets/:id` — Update asset descriptive fields *(requires fixed_assets:mutate)*
- `DELETE /api/fixed-assets/assets/:tenant_id/:id` — Deactivate asset *(requires fixed_assets:mutate)*
- `GET /api/fixed-assets/assets/:tenant_id/:id` — Get asset
- `GET /api/fixed-assets/assets/:tenant_id` — List assets (filterable by `?status=`)

### Depreciation
- `POST /api/fixed-assets/depreciation/schedule` — Generate depreciation schedule for an asset *(requires fixed_assets:mutate)*
- `POST /api/fixed-assets/depreciation/runs` — Execute depreciation run *(requires fixed_assets:mutate)*
- `GET /api/fixed-assets/depreciation/runs/:tenant_id` — List runs
- `GET /api/fixed-assets/depreciation/runs/:tenant_id/:id` — Get run detail

### Disposals
- `POST /api/fixed-assets/disposals` — Dispose or impair an asset *(requires fixed_assets:mutate)*
- `GET /api/fixed-assets/disposals/:tenant_id` — List disposals
- `GET /api/fixed-assets/disposals/:tenant_id/:id` — Get disposal detail

### Admin (requires `X-Admin-Token` header)
- `POST /api/fixed-assets/admin/projection-status` — Query projection status
- `POST /api/fixed-assets/admin/consistency-check` — Run consistency check
- `GET /api/fixed-assets/admin/projections` — List projections

### Operational
- `GET /healthz` — Liveness probe (platform health crate)
- `GET /api/health` — Legacy liveness probe
- `GET /api/ready` — Readiness probe (verifies DB connectivity)
- `GET /api/version` — Module identity and schema version
- `GET /metrics` — Prometheus metrics

---

## v2 Roadmap (Deferred)

| Feature | Rationale for Deferral |
|---------|----------------------|
| **Declining-Balance Depreciation** | ENUM value exists in schema CHECK constraint; engine not implemented. Useful for tax-accelerated depreciation. |
| **Units-of-Production Depreciation** | Requires meter/usage data integration. Deferred until Maintenance module integration is mature. |
| **Asset Revaluation** | IFRS fair-value adjustments require revaluation surplus tracking and complex GL entries. Not needed for GAAP-only tenants. |
| **Partial Disposals** | Disposing a component of a composite asset (e.g., engine from a vehicle). Requires parent/child asset model. |
| **Bulk Import** | CSV/Excel asset import for migration from legacy systems. Needs validation pipeline and error reporting. |
| **Active GL Consumer** | Platform-side NATS consumer that posts journal entries from FA events. Not part of this module. |
| **Frontend UI** | Consumed via API by vertical apps or TCP. |

---

## Document Standards Reference

This document follows the revision and decision log standards defined at:
`docs/frontend/DOC-REVISION-STANDARDS.md`

---

## Decision Log

Every significant product, architecture, or standards decision is recorded here. Do not re-open a decision without adding a new row that supersedes the old one.

| Date | Decision | Rationale | Decided By |
|------|----------|-----------|-----------|
| 2026-02-18 | Categories own GL account references, assets optionally override | Changing GL mapping for an asset class is a single category update; per-asset overrides available but not required | Platform Orchestrator |
| 2026-02-18 | Depreciation schedule is pre-computed and stored, not calculated on the fly | Makes runs fast (one UPDATE), auditable (every period is a row), and idempotent (ON CONFLICT DO NOTHING) | Platform Orchestrator |
| 2026-02-18 | Straight-line engine is a pure function with no I/O | Separation of computation from persistence; fully unit-testable without a database | Platform Orchestrator |
| 2026-02-18 | Depreciation runs are batch operations across all tenant assets | Ensures all assets in a tenant are deprecated to the same point in time; consistent reporting | Platform Orchestrator |
| 2026-02-18 | Disposals compute gain/loss at mutation time and zero NBV | One-way operation, no undo; gain/loss is a fact at disposal time, not a deferred calculation | Platform Orchestrator |
| 2026-02-18 | AP capitalization uses anti-corruption layer (local payload mirrors) | Insulates Fixed-Assets from AP schema changes; never imports AP crate types | Platform Orchestrator |
| 2026-02-18 | Idempotency at every integration point (schedule, runs, disposals, AP cap, outbox) | Event replay and retry safety; no duplicate assets, schedules, disposals, or events under redelivery | Platform Orchestrator |
| 2026-02-18 | fa_asset_status and fa_run_status converted from ENUM to TEXT (migrations 8, 9) | SQLx FromRow cannot decode PG ENUM to Rust String without custom Type impl; TEXT aligns schema with domain models | Platform Orchestrator |
| 2026-02-18 | All tables prefixed `fa_` to avoid cross-module schema clashes | Platform convention for multi-module databases; each module's tables are visually grouped | Platform Orchestrator |
| 2026-02-18 | Monetary values as integer minor units, currency as lowercase ISO 4217 text | Avoids floating-point precision loss; platform-wide standard | Platform Orchestrator |
| 2026-02-18 | AP capitalization linkage stored in `fa_ap_capitalizations` with soft references to bill_id/line_id | No cross-module FK references; audit trail survives AP data lifecycle changes | Platform Orchestrator |
| 2026-02-18 | Salvage percentage in categories (basis points), absolute salvage value in assets | Categories define a percentage-based default; assets store the computed absolute amount for precision | Platform Orchestrator |
