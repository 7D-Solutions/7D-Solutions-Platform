# Reporting Module — Vision & Specification

> **Status:** v0.1.0 — Unproven (active development)
> **Crate:** `reporting`
> **Default port:** 8096
> **Schema prefix:** `rpt_`

---

## Revision History

| Rev | Date       | Author    | Summary |
|-----|------------|-----------|---------|
| 0.1 | 2026-02-24 | DarkOwl   | Initial vision doc from source audit |

---

## 1. Business Problem

Every multi-tenant SaaS platform needs a single pane of glass for financial health. Tenants want to answer questions like:

- "What is our profit/loss this month?"
- "How much do customers owe us, and how overdue are they?"
- "When will outstanding invoices likely be paid?"
- "What are our key financial metrics right now?"

Querying source-of-truth tables (GL, AR, AP, Payments) in real-time is expensive and couples read-heavy dashboards to write-optimized schemas. The Reporting module solves this by maintaining **pre-computed, read-only caches** fed by domain events, giving tenants sub-second financial reports without touching upstream databases.

---

## 2. What the Module Does

The Reporting module is a **read-only CQRS projection service**. It:

1. **Subscribes** to domain events from AR, AP, GL, Payments, and Inventory via NATS.
2. **Ingests** those events into denormalized cache tables optimized for reporting queries.
3. **Serves** financial statements, aging reports, KPI snapshots, and cash flow forecasts through a REST API.
4. **Never writes back** to any source module's database.

### Report types available today

| Report | Endpoint | Source caches |
|--------|----------|---------------|
| Profit & Loss | `GET /api/reporting/pl` | `rpt_trial_balance_cache` |
| Balance Sheet | `GET /api/reporting/balance-sheet` | `rpt_trial_balance_cache` |
| Cash Flow Statement | `GET /api/reporting/cashflow` | `rpt_cashflow_cache`, `rpt_trial_balance_cache` |
| AR Aging | `GET /api/reporting/ar-aging` | `rpt_ar_aging_cache` |
| AP Aging | `GET /api/reporting/ap-aging` | `rpt_ap_aging_cache` |
| KPI Dashboard | `GET /api/reporting/kpis` | `rpt_kpi_cache`, `rpt_ar_aging_cache`, `rpt_ap_aging_cache`, `rpt_trial_balance_cache`, `rpt_cashflow_cache` |
| Cash Flow Forecast | `GET /api/reporting/forecast` | `rpt_open_invoices_cache`, `rpt_payment_history` |

---

## 3. Who Uses This

| Actor | Interaction |
|-------|-------------|
| **Tenant dashboard (TCP UI)** | Reads P&L, Balance Sheet, aging, KPIs, forecasts via REST |
| **Platform admin** | Triggers cache rebuilds, checks projection health via admin endpoints |
| **Upstream modules** | Produce domain events consumed by Reporting (AR, AP, GL, Payments, Inventory) |
| **Monitoring** | Scrapes `/metrics` for Prometheus SLO counters and cache health |

---

## 4. Design Principles

1. **Read-only projection** — The module never writes to another module's database. It owns only `rpt_*` tables.
2. **Event-driven refresh** — Caches update in near-real-time as domain events arrive via NATS, not on a polling schedule.
3. **Two-layer idempotency** — Framework checkpoint gate (skip already-processed sequence) + handler-level `ON CONFLICT DO UPDATE` (safe replay).
4. **Tenant isolation** — Every query, every cache row, every checkpoint is scoped by `tenant_id`. No cross-tenant data leakage.
5. **Multi-currency native** — All monetary amounts stored as `BIGINT` minor units with an explicit `currency` column. No implicit currency assumptions.
6. **Cache rebuildability** — Any cache can be fully reconstructed by resetting ingestion checkpoints and replaying events from NATS.
7. **Fail-open reads** — If a cache is stale, the API still returns data (with `computed_at` timestamps). Staleness is visible, not hidden.

---

## 5. MVP / Current Scope (v0.1.0)

### Shipped

- P&L and Balance Sheet from trial balance cache (account prefix classification: 4xxx Revenue, 5xxx COGS, 6xxx Expenses, 1xxx Assets, 2xxx Liabilities, 3xxx Equity)
- Cash Flow Statement — indirect method v1 (operating only: net income from GL + cash collections from Payments)
- AR aging with standard 5-bucket breakdown (current / 1-30 / 31-60 / 61-90 / 90+) per customer
- AP aging with same bucket structure per vendor
- Unified KPI dashboard (AR outstanding, AP outstanding, cash collected YTD, burn YTD, MRR, inventory value)
- Probabilistic cash flow forecasting with conditional CDF, per-customer payment profiles, and at-risk invoice identification
- Daily snapshot runner (P&L + BS persisted to `rpt_statement_cache`)
- Admin endpoints: cache rebuild, projection status, consistency check
- Prometheus metrics with SLO counters
- BackfillRunner for full cache reconstruction via checkpoint reset

### Not yet implemented

- Cash Flow Statement: investing and financing activity sections (stubs exist)
- Cash Flow Statement: working-capital adjustments
- Comparative reports (period-over-period, budget-vs-actual)
- Report export (PDF, CSV)
- Scheduled report delivery
- Custom date-range aggregation for KPIs
- Historical KPI trend queries

---

## 6. Technology Summary

| Concern | Choice |
|---------|--------|
| Language | Rust (async, tokio runtime) |
| HTTP framework | Axum 0.8 |
| Database | PostgreSQL via sqlx (compile-time checked queries) |
| Event bus | NATS JetStream via `event-bus` crate |
| Auth | JWT (optional_claims_mw) + permission-based guards via `security` crate |
| Metrics | Prometheus (prometheus crate) exposed at `/metrics` |
| Projections admin | Shared `projections` crate for standardized admin interface |
| Serialization | serde + serde_json |
| CORS | tower-http CorsLayer |

---

## 7. Structural Decisions

### 7.1 CQRS Projection Architecture

**Decision:** Reporting is a purely read-side service — it subscribes to events and builds denormalized caches. It never issues commands to other modules.

**Rationale:** Decouples read-heavy dashboard workloads from write-optimized source schemas. Caches can be tuned independently (indexes, denormalization) without affecting upstream modules.

### 7.2 Two-Layer Idempotency

**Decision:** Each `IngestConsumer` checks the `rpt_ingestion_checkpoints` table before forwarding to the handler. The handler additionally uses `ON CONFLICT DO UPDATE` on its target cache.

**Rationale:** The checkpoint layer prevents duplicate processing during normal operation. The SQL-level upsert provides safety during backfills or when checkpoints are deliberately reset for cache reconstruction.

### 7.3 Account Prefix Classification

**Decision:** Financial statements are derived by classifying accounts based on their code prefix:
- `1xxx` = Assets (debit-normal)
- `2xxx` = Liabilities (credit-normal)
- `3xxx` = Equity (credit-normal)
- `4xxx` = Revenue (credit-normal)
- `5xxx` = COGS (debit-normal)
- `6xxx` = Expenses (debit-normal)

**Rationale:** Follows standard chart-of-accounts convention. The GL module owns account classification; Reporting trusts the prefix mapping for statement generation.

### 7.4 Minor Units for All Monetary Amounts

**Decision:** All `amount_minor`, `debit_minor`, `credit_minor` columns are `BIGINT` storing the smallest currency unit (e.g., cents for USD).

**Rationale:** Eliminates floating-point rounding errors in financial computations. The GL ingestion handler converts major units (dollars) to minor units (cents) via `*100` on receipt.

### 7.5 Conditional CDF for Forecast Probability

**Decision:** Cash forecast uses conditional probability: `P(pay in N days | age = A) = (F(A+N) - F(A)) / (1 - F(A))`, where `F` is the empirical CDF of days-to-pay from historical payment data.

**Rationale:** Raw CDF underestimates collection probability for already-aged invoices. Conditioning on current age produces accurate forward-looking estimates.

### 7.6 Per-Customer Payment Profiles with Tenant Fallback

**Decision:** Payment timing profiles are built per-customer if >= 3 historical records exist; otherwise falls back to tenant-wide aggregated profile.

**Rationale:** Customer-specific profiles capture real payment behavior. The 3-record threshold prevents noisy estimates from sparse data while still personalizing where possible.

### 7.7 Cash Flow Indirect Method v1

**Decision:** Cash flow statement uses the indirect method starting from net income (GL-derived) and adds cash collections from Payments events. Investing and financing sections are stubbed but empty.

**Rationale:** The indirect method is standard for non-public companies. v1 covers the operating section which provides the most value; investing/financing will be added as those source modules emit the required events.

---

## 8. Domain Authority

The Reporting module is **authoritative for**:
- Pre-computed financial report data (read-model caches)
- Ingestion checkpoint state (which events have been processed)
- Cash forecast probability computations and at-risk invoice identification

The Reporting module **defers to**:
- **GL** for account balances, chart of accounts, and posting events
- **AR** for invoice lifecycle, aging snapshots, and receivable events
- **AP** for bill lifecycle, aging snapshots, and payable events
- **Payments** for payment execution and settlement events
- **Inventory** for valuation snapshots
- **Subscriptions** for MRR computation events (if available)

---

## 9. Data Ownership

All tables live in the reporting service's own database (`reporting_{app_id}_db`), prefixed `rpt_` to prevent collisions.

### 9.1 `rpt_ingestion_checkpoints`

Tracks NATS replay position per consumer per tenant.

| Column | Type | Notes |
|--------|------|-------|
| id | UUID PK | |
| consumer_name | TEXT | e.g. `gl_trial_balance`, `ar_aging` |
| tenant_id | TEXT | Tenant scope |
| last_sequence | BIGINT | NATS stream sequence number |
| last_event_id | TEXT | EventEnvelope idempotency key |
| processed_at | TIMESTAMPTZ | |
| **Unique** | | `(consumer_name, tenant_id)` |

### 9.2 `rpt_trial_balance_cache`

Point-in-time account balances for P&L and Balance Sheet computation.

| Column | Type | Notes |
|--------|------|-------|
| id | UUID PK | |
| tenant_id | TEXT | |
| as_of | DATE | Snapshot date |
| account_code | TEXT | e.g. `4000`, `5100` |
| account_name | TEXT | |
| currency | TEXT | ISO 4217 |
| debit_minor | BIGINT | >= 0, minor units |
| credit_minor | BIGINT | >= 0, minor units |
| net_minor | BIGINT | Signed: positive = net debit |
| **Unique** | | `(tenant_id, as_of, account_code, currency)` |

### 9.3 `rpt_statement_cache`

Persisted financial statement line items (daily snapshot runner output).

| Column | Type | Notes |
|--------|------|-------|
| id | UUID PK | |
| tenant_id | TEXT | |
| statement_type | TEXT | `income_statement` or `balance_sheet` |
| as_of | DATE | |
| line_code | TEXT | e.g. `4000_revenue`, `5000_cogs` |
| line_label | TEXT | |
| currency | TEXT | |
| amount_minor | BIGINT | |
| **Unique** | | `(tenant_id, statement_type, as_of, line_code, currency)` |

### 9.4 `rpt_ar_aging_cache`

AR aging buckets per customer.

| Column | Type | Notes |
|--------|------|-------|
| id | UUID PK | |
| tenant_id | TEXT | |
| as_of | DATE | |
| customer_id | TEXT | |
| currency | TEXT | |
| current_minor | BIGINT | Not yet due |
| bucket_1_30_minor | BIGINT | 1-30 days past due |
| bucket_31_60_minor | BIGINT | 31-60 days |
| bucket_61_90_minor | BIGINT | 61-90 days |
| bucket_over_90_minor | BIGINT | > 90 days |
| total_minor | BIGINT | Sum of all buckets |
| **Unique** | | `(tenant_id, as_of, customer_id, currency)` |

### 9.5 `rpt_ap_aging_cache`

AP aging buckets per vendor. Same bucket structure as AR.

| Column | Type | Notes |
|--------|------|-------|
| id | UUID PK | |
| tenant_id | TEXT | |
| as_of | DATE | |
| vendor_id | TEXT | |
| currency | TEXT | |
| current_minor | BIGINT | Not yet due |
| bucket_1_30_minor | BIGINT | 1-30 days past due |
| bucket_31_60_minor | BIGINT | 31-60 days |
| bucket_61_90_minor | BIGINT | 61-90 days |
| bucket_over_90_minor | BIGINT | > 90 days |
| total_minor | BIGINT | Sum of all buckets |
| **Unique** | | `(tenant_id, as_of, vendor_id, currency)` |

### 9.6 `rpt_cashflow_cache`

Cash flow statement lines per reporting period.

| Column | Type | Notes |
|--------|------|-------|
| id | UUID PK | |
| tenant_id | TEXT | |
| period_start | DATE | |
| period_end | DATE | Doubles as `as_of` for indexing |
| activity_type | TEXT | `operating`, `investing`, `financing` |
| line_code | TEXT | e.g. `net_income`, `cash_collections` |
| line_label | TEXT | |
| currency | TEXT | |
| amount_minor | BIGINT | Signed: positive = inflow |
| **Unique** | | `(tenant_id, period_start, period_end, activity_type, line_code, currency)` |

### 9.7 `rpt_kpi_cache`

Point-in-time KPI snapshots. Supports both monetary and rate/ratio KPIs.

| Column | Type | Notes |
|--------|------|-------|
| id | UUID PK | |
| tenant_id | TEXT | |
| as_of | DATE | |
| kpi_name | TEXT | e.g. `mrr`, `inventory_value` |
| currency | TEXT | Empty string for dimensionless KPIs |
| amount_minor | BIGINT (nullable) | For monetary KPIs |
| basis_points | BIGINT (nullable) | For rate KPIs (10000 bp = 100%) |
| **Unique** | | `(tenant_id, as_of, kpi_name, currency)` |
| **Check** | | At least one of `amount_minor` or `basis_points` must be non-null |

### 9.8 `rpt_payment_history`

Historical paid invoices for empirical CDF computation.

| Column | Type | Notes |
|--------|------|-------|
| id | BIGSERIAL PK | |
| tenant_id | TEXT | |
| customer_id | TEXT | |
| invoice_id | TEXT | |
| currency | TEXT | |
| amount_cents | BIGINT | Invoice amount in minor units |
| issued_at | TIMESTAMPTZ | Original invoice date |
| paid_at | TIMESTAMPTZ | Payment date |
| days_to_pay | INT | Computed: `paid_at - issued_at` |
| **Unique** | | `(tenant_id, invoice_id)` |

### 9.9 `rpt_open_invoices_cache`

Invoice lifecycle tracking for forecast at-risk projections.

| Column | Type | Notes |
|--------|------|-------|
| id | BIGSERIAL PK | |
| tenant_id | TEXT | |
| invoice_id | TEXT | |
| customer_id | TEXT | |
| currency | TEXT | |
| amount_cents | BIGINT | |
| issued_at | TIMESTAMPTZ | |
| due_at | TIMESTAMPTZ (nullable) | |
| status | TEXT | `open` or `paid` |
| **Unique** | | `(tenant_id, invoice_id)` |

### 9.10 `reporting_schema_version`

Bootstrap table for migration tracking.

| Column | Type | Notes |
|--------|------|-------|
| id | SERIAL PK | |
| version | TEXT | Migration identifier |
| applied_at | TIMESTAMPTZ | |

---

## 10. Events Produced

**None.** Reporting is a read-only projection. It consumes events but never publishes them.

---

## 11. Events Consumed

| Subject | Source Module | Handler | Target Cache |
|---------|-------------|---------|--------------|
| `gl.events.posting.requested` | GL | `GlTrialBalanceHandler` | `rpt_trial_balance_cache` |
| `ar.events.ar.ar_aging_updated` | AR | `ArAgingHandler` | `rpt_ar_aging_cache` |
| `ar.events.ar.invoice_opened` | AR | `InvoiceOpenedHandler` | `rpt_open_invoices_cache` |
| `ar.events.ar.invoice_paid` | AR | `InvoicePaidHandler` | `rpt_payment_history`, `rpt_open_invoices_cache` |
| `ap.events.ap.bill_created` | AP | `ApBillCreatedHandler` | `rpt_ap_aging_cache` |
| `ap.events.ap.bill_voided` | AP | `ApBillVoidedHandler` | `rpt_ap_aging_cache` |
| `ap.events.ap.payment_executed` | AP | `ApPaymentExecutedHandler` | `rpt_ap_aging_cache` |
| `payments.events.payment.succeeded` | Payments | `PaymentSucceededHandler` | `rpt_cashflow_cache` |
| `inventory.events.inventory.valuation_snapshot` | Inventory | `InventoryValuationHandler` | `rpt_kpi_cache` |

---

## 12. Integration Points

| Module | Direction | Mechanism | Purpose |
|--------|-----------|-----------|---------|
| GL | Consumes | NATS event | Trial balance data for P&L, BS, cash flow |
| AR | Consumes | NATS event | Aging snapshots, invoice lifecycle for forecasting |
| AP | Consumes | NATS event | Aging snapshots (bills, voids, payments) |
| Payments | Consumes | NATS event | Cash collections for cash flow statement |
| Inventory | Consumes | NATS event | Valuation snapshots for KPI dashboard |
| `security` crate | Uses | Middleware | JWT verification, permission guards, rate limiting |
| `health` crate | Uses | Library | Standardized health/ready responses |
| `projections` crate | Uses | Library | Admin projection-status and consistency-check interface |
| `event-bus` crate | Uses | Library | NATS consumer abstraction |

---

## 13. Invariants

1. **Tenant isolation** — Every SQL query includes a `WHERE tenant_id = $1` predicate. No endpoint returns data across tenants.
2. **Cache-only reads** — Domain queries read only from `rpt_*` tables, never from upstream module databases.
3. **Checkpoint monotonicity** — `last_sequence` in `rpt_ingestion_checkpoints` only moves forward. Events with sequence <= checkpoint are skipped.
4. **Non-negative aging buckets** — All aging bucket columns have `CHECK (column >= 0)`. Subtractive handlers (void, payment) use `GREATEST(0, ...)` to prevent underflow.
5. **At least one KPI value** — The `rpt_kpi_cache` table enforces `amount_minor IS NOT NULL OR basis_points IS NOT NULL`.
6. **Minor-unit consistency** — All monetary amounts are `BIGINT` minor units. The GL ingestion handler converts from major units at the boundary (`*100`).
7. **Idempotent ingestion** — Every cache table has a unique constraint matching its grain. Replay of the same event produces the same cache state.
8. **No cross-module writes** — The module connects only to its own `reporting_{app_id}_db` database. It has no connection to AR, AP, GL, or Payments databases.

---

## 14. API Surface

### Operational Endpoints

| Method | Path | Auth | Purpose |
|--------|------|------|---------|
| GET | `/healthz` | None | Liveness probe |
| GET | `/api/health` | None | Health check (legacy) |
| GET | `/api/ready` | None | Readiness probe (DB connectivity) |
| GET | `/api/version` | None | Module identity + schema version |
| GET | `/metrics` | None | Prometheus metrics scrape |

### Report Endpoints (Read)

| Method | Path | Auth | Query Params | Purpose |
|--------|------|------|-------------|---------|
| GET | `/api/reporting/pl` | JWT | `tenant_id`, `from`, `to` | Profit & Loss statement |
| GET | `/api/reporting/balance-sheet` | JWT | `tenant_id`, `as_of` | Balance Sheet |
| GET | `/api/reporting/cashflow` | JWT | `tenant_id`, `from`, `to` | Cash Flow Statement |
| GET | `/api/reporting/ar-aging` | JWT | `tenant_id`, `as_of` | AR Aging report |
| GET | `/api/reporting/ap-aging` | JWT | `tenant_id`, `as_of` | AP Aging report |
| GET | `/api/reporting/kpis` | JWT | `tenant_id`, `as_of` | KPI dashboard |
| GET | `/api/reporting/forecast` | JWT | `tenant_id`, `horizons` | Cash flow forecast |

### Admin Endpoints (Write / Admin)

| Method | Path | Auth | Purpose |
|--------|------|------|---------|
| POST | `/api/reporting/rebuild` | JWT + `REPORTING_MUTATE` | Trigger snapshot cache rebuild |
| POST | `/api/reporting/admin/projection-status` | Admin token | Projection health status |
| POST | `/api/reporting/admin/consistency-check` | Admin token | Cache consistency verification |
| GET | `/api/reporting/admin/projections` | Admin token | List projection consumers |

---

## 15. Open Questions

1. **Investing/financing activities** — What source events should populate these cash flow sections? Fixed-Assets module events for investing, loan/debt events for financing?
2. **Working-capital adjustments** — Should the cash flow statement include AR/AP delta adjustments for the indirect method, or is that deferred to a future version?
3. **Comparative reports** — Should period-over-period and budget-vs-actual be built as new endpoints or as query parameters on existing ones?
4. **Report export** — Will PDF/CSV generation live in this module or in a separate export service?
5. **KPI trend API** — Should historical KPI snapshots be queryable as time-series, or only point-in-time?
6. **Subscription MRR** — Currently MRR is a placeholder in the KPI aggregation. What event from Subscriptions should drive it?

---

## 16. Decision Log

| # | Decision | Date | Rationale |
|---|----------|------|-----------|
| D1 | Read-only CQRS projection, separate database | 2026-02-18 | Isolates reporting workload from OLTP modules |
| D2 | `rpt_` table prefix convention | 2026-02-18 | Prevents name collisions if schemas are ever co-located |
| D3 | BIGINT minor units for all monetary columns | 2026-02-18 | Eliminates floating-point rounding in financial math |
| D4 | Two-layer idempotency (checkpoint + ON CONFLICT) | 2026-02-18 | Safe replay during normal operation and backfills |
| D5 | Account prefix classification (1xxx-6xxx) | 2026-02-18 | Standard chart-of-accounts convention, avoids metadata lookup |
| D6 | Conditional CDF for cash forecasting | 2026-02-22 | Accurate forward-looking probability conditioned on invoice age |
| D7 | Per-customer profiles with 3-record threshold | 2026-02-22 | Balances personalization vs. statistical significance |
| D8 | Indirect method for cash flow (operating only v1) | 2026-02-18 | Standard non-public approach; investing/financing deferred until source events exist |
| D9 | KPI cache supports both monetary and basis-point values | 2026-02-18 | Single table serves diverse KPI types without schema changes |
| D10 | AP aging uses accumulative upserts with GREATEST(0,...) | 2026-02-18 | Handles out-of-order events without negative bucket balances |
