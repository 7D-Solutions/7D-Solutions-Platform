# GL Module — Product Vision & Technical Specification

**7D Solutions Platform**
**Status:** Vision Document + Technical Specification (v0.1.0)
**Last updated:** 2026-02-24

---

## Revision History

| Rev | Date | Changed By | Summary |
|-----|------|-----------|---------|
| 1.0 | 2026-02-24 | SageDesert | Initial vision doc — extracted from source code, migrations, consumers, services, and existing phase docs. Covers full GL surface: journal posting, balance engine, financial statements, period close, FX, accruals, revenue recognition, DLQ, and all NATS consumers. |
| 2.0 | 2026-02-24 | SageDesert | Review pass: fixed consumer count (9→11), added missing credit note consumer to In Scope list, corrected admin routes description (was "DLQ management", actually projection management), noted 4 consumers implemented but not wired in main.rs. |

---

## The Business Problem

Every multi-module business platform reaches the same inflection point: **individual modules know their own numbers, but nobody has the unified financial picture.**

AR knows what customers owe. AP knows what the company owes vendors. Inventory knows the cost of goods on hand. Payroll knows labor costs. But without a General Ledger, there is no single place that ties all of these together into a balanced, auditable, period-aware record of what actually happened financially.

Small businesses either run a separate accounting system (double-entry into QuickBooks, Xero, or spreadsheets) or they don't track financials at all until tax season arrives. Both approaches create reconciliation nightmares. Data drifts between systems. Entries get missed. Period close is manual and error-prone. The finance team spends days producing trial balances and financial statements that should be available instantly.

The GL module eliminates this gap by providing a **platform-native double-entry accounting engine** that consumes financial events from every other module and produces auditable, period-aware financial statements — without requiring an external accounting system.

---

## What the Module Does

The GL module is the **authoritative system for double-entry accounting** across the entire platform. It is the single place where all financial activity — revenue, expenses, assets, liabilities, equity — is recorded as balanced journal entries and rolled up into financial statements.

It answers six questions:
1. **What happened?** — Journal entries with full audit trail: who posted it, what source event triggered it, when it was recorded, and which accounts were affected.
2. **Is it balanced?** — Every journal entry enforces debits = credits. The accounting equation (Assets = Liabilities + Equity) is maintained at all times.
3. **What's the position?** — Trial balance, balance sheet, and income statement give the financial position at any point in time.
4. **What period are we in?** — Accounting periods with close lifecycle: validate, close with tamper-proof hash, reopen with audit trail.
5. **What about foreign currencies?** — FX rate store, unrealized revaluation, and realized gain/loss posting when settlements crystallize.
6. **What revenue can we recognize?** — ASC 606 / IFRS 15 revenue recognition with contracts, performance obligations, amortization schedules, and period-by-period recognition runs.

---

## Who Uses This

The GL module is a platform service. It has no frontend — it exposes HTTP APIs and NATS consumers that other modules and frontends consume.

### Controller / CFO
- Reviews trial balance, income statement, balance sheet, and cash flow statement
- Manages accounting period lifecycle (validate close, execute close, approve reopen)
- Configures close calendar with reminder schedules
- Signs off on pre-close checklists and approvals
- Runs FX revaluation at period end
- Creates and manages accrual templates for recurring entries

### Accountant / Bookkeeper
- Reviews account activity and GL detail reports
- Creates manual journal entries via posting requests
- Manages chart of accounts (create accounts, activate/deactivate)
- Monitors DLQ for failed postings that need attention
- Runs revenue recognition schedules

### System (NATS Consumers)
- Automatically posts journal entries from AR invoices, payments, credit notes, and write-offs
- Posts AP vendor bill expense entries on approval
- Posts COGS entries when inventory items are issued
- Posts depreciation entries from fixed asset depreciation runs
- Posts tax liability entries on invoice finalization and reverses them on void
- Posts labor cost accruals from timekeeping events
- Posts realized FX gain/loss on foreign currency settlement

### Other Platform Modules
- AR, AP, Inventory, Fixed Assets, Timekeeping — emit events that GL consumes
- All modules can query GL for financial position via HTTP API

---

## Design Principles

### Journal Entries Are the Source of Truth
Balances, statements, and snapshots are all derived from journal entries. The `account_balances` table is a materialized read model that can be deterministically rebuilt from journals at any time using the `rebuild_balances` admin tool. If balances ever drift, journals win.

### Every Entry Must Balance
The accounting equation is enforced at every level: payload validation (debits = credits within penny precision), database constraints (debit_minor >= 0, credit_minor >= 0), and invariant checks (assert_all_entries_balanced). An unbalanced entry cannot exist in the system.

### Event-Driven, Never Synchronous
GL never calls other modules. It subscribes to NATS events and posts journal entries in response. This means GL has zero runtime dependencies on AR, AP, Inventory, Fixed Assets, or any other module. If a source module is down, GL simply doesn't receive events — it doesn't break.

### Idempotency Everywhere
Every posting is keyed by `source_event_id` (UNIQUE constraint). Duplicate events are silently skipped. Period close is idempotent via `closed_at` field. Accrual instances use deterministic idempotency keys. FX rates use caller-supplied dedup keys. Replay is always safe.

### Closed Periods Are Immutable
Once a period is closed, no new postings or reversals can target it. The close operation creates a SHA-256 hash of the period snapshot for tamper detection. Reopening requires an explicit request → approval workflow with full audit trail.

### Multi-Currency Native
Every journal entry carries a currency code. Balances are materialized per (tenant, period, account, currency) grain. FX rates are stored in an append-only table. Reporting-currency statements translate transaction-currency balances at period-end rates.

### No Mocking in Tests
Integration tests hit real Postgres and real NATS. Platform-wide standard.

---

## Current Scope (v0.1.0)

### In Scope
- Double-entry journal posting from NATS events with full validation
- Chart of Accounts: flat account structure with type (asset/liability/equity/revenue/expense), normal balance direction, active/inactive state
- 11 NATS event consumers: GL posting, reversal, credit note, write-off, inventory COGS, AR tax committed, AR tax voided, fixed assets depreciation, AP vendor bill approved, timekeeping labor cost, realized FX gain/loss (note: 4 consumers — credit note, realized FX, AP vendor bill, timekeeping labor cost — are implemented but not yet wired in main.rs)
- Balance engine: materialized rollups per (tenant, period, account, currency)
- Financial statements: trial balance, income statement, balance sheet, cash flow statement
- Reporting-currency statements: trial balance, income statement, balance sheet translated to tenant reporting currency
- Account activity report and GL detail report with filtering and pagination
- Period summary snapshots with pre-aggregated counts and totals
- Period close lifecycle: validate → close (with SHA-256 hash) → reopen (with approval workflow)
- Close calendar with configurable reminder schedules
- Pre-close checklist items and approval signoffs
- FX rate store (append-only snapshots) with latest-as-of lookups
- FX unrealized revaluation and realized gain/loss posting
- Accrual templates and instances with auto-reversal
- Revenue recognition (ASC 606 / IFRS 15): contracts, performance obligations, recognition schedules with versioning, period recognition runs, contract amendments
- Dead letter queue for failed events with retry tracking
- Reversal entries with chain depth enforcement (max depth = 1)
- Correlation ID propagation for cross-module audit traceability
- Prometheus metrics: journal entries total, posting errors, HTTP request latency, consumer lag
- Admin tool: `rebuild_balances` for deterministic balance recomputation

### Explicitly Out of Scope
- Account hierarchy (parent/child account groupings for sub-ledger reporting)
- Budget vs actual comparison
- Consolidation across multiple tenants or entities
- Intercompany eliminations
- Audit log viewer / UI
- Tax return preparation or tax form generation
- Multi-book accounting (GAAP + IFRS parallel books)
- GL consumer for maintenance module cost events (future integration)
- Frontend UI (consumed via API by vertical apps or TCP)

---

## Technology Summary

| Layer | Technology | Notes |
|-------|-----------|-------|
| Language | Rust | Platform standard |
| HTTP framework | Axum | Port 8090 (default) |
| Database | PostgreSQL | Dedicated database (port 5438), SQLx for queries and migrations |
| Event bus | NATS | Via platform `event-bus` crate, configurable (inmemory for testing) |
| Auth | JWT via platform `security` crate | Tenant-scoped, role-based; `gl.post` permission for mutations |
| Outbox | Platform outbox pattern | Same as all other modules |
| Metrics | Prometheus | `/metrics` endpoint via `prometheus` crate |
| Projections | Platform `projections` crate | Projection metrics integration |
| Crate | `gl-rs` | Single crate, standard module layout |

---

## Structural Decisions (The "Walls")

### 1. Journal entries are the source of truth — balances are a derived read model
Balances in `account_balances` are a materialized rollup that can be rebuilt from journals at any time. This means balance corruption is recoverable, schema changes to the balance table are non-destructive, and audit integrity is anchored to the append-only journal record. The `rebuild_balances` admin tool exists specifically for this scenario.

### 2. GL is a pure consumer — it never calls other modules
All financial data enters GL via NATS events. GL never makes outbound HTTP calls to AR, AP, Inventory, or any other module. This means GL has zero runtime dependencies, can be deployed and tested independently, and its availability is not affected by the health of source modules.

### 3. Amounts stored as integer minor units (cents)
All monetary amounts in the database use `BIGINT` minor units (e.g., $25.99 = 2599). This eliminates floating-point rounding errors that plague f64 arithmetic in financial systems. The posting request contract uses f64 for JSON compatibility, but conversion to minor units happens at the validation boundary with penny-precision epsilon checks.

### 4. One-level reversal chain maximum
A reversal entry can reverse an original entry, but you cannot reverse a reversal. The `assert_max_reversal_chain_depth` invariant enforces this. This prevents ambiguous projection states where it's unclear which entries are "active" and which have been compensated.

### 5. Period close is atomic with tamper-proof hash
Closing a period validates balances, creates a snapshot, computes a SHA-256 hash, and sets `closed_at` — all in one transaction. The hash is stored and can be verified later to detect tampering. Reopening requires a separate request → approval workflow that records the prior close hash for audit purposes.

### 6. FX rates are append-only — never updated or deleted
Each FX rate insert is a new snapshot. The `fx_rates` table is append-only with an idempotency key. Latest-as-of queries find the most recent rate for a currency pair at or before a given timestamp. This preserves rate history for audit and revaluation.

### 7. Accrual instances are append-only with deterministic IDs
Accrual `accrual_id` = `Uuid::v5(template_id, period)`. Idempotency key = `"accrual:{template_id}:{period}"`. Each accrual can be reversed at most once (UNIQUE constraint on `original_accrual_id`). This makes accrual processing fully deterministic and replay-safe.

### 8. Revenue recognition schedules are versioned, never rewritten
When a contract is amended, a new schedule version is created with a `previous_schedule_id` link. Old schedule lines are never modified. This preserves the full recognition history and supports ASC 606 cumulative catch-up adjustments.

### 9. Tenant isolation via tenant_id on every table
Standard platform multi-tenant pattern. Every table has `tenant_id` as a non-nullable field. Every query filters by `tenant_id`. No exceptions.

---

## Domain Authority

GL is the **source of truth** for:

| Domain Entity | GL Authority |
|---------------|-------------|
| **Journal Entries** | Double-entry records with source tracing, correlation IDs, and reversal links. Append-only. |
| **Journal Lines** | Individual debit/credit lines per entry with account reference, amounts in minor units, and optional memo. |
| **Chart of Accounts** | Flat account register per tenant: code, name, type (asset/liability/equity/revenue/expense), normal balance direction, active/inactive. |
| **Account Balances** | Materialized rollup per (tenant, period, account, currency): cumulative debits, credits, and net balance. Derived from journals. |
| **Accounting Periods** | Period definitions with non-overlapping date ranges (EXCLUDE constraint) and full close lifecycle state. |
| **Period Close State** | Close workflow: open → close_requested → closed. Includes SHA-256 hash, close reason, audit trail. Reopen tracking with count and timestamps. |
| **Close Calendar** | Expected close dates, reminder schedules, owner roles per period. Idempotent reminder tracking. |
| **Close Checklists & Approvals** | Pre-close items (pending/complete/waived) and approval signoffs per period. |
| **Period Reopen Requests** | Append-only audit trail: requested → approved/rejected, with prior close hash captured. |
| **Period Summary Snapshots** | Pre-aggregated journal/line counts and debit/credit totals per (tenant, period, currency). |
| **FX Rates** | Append-only exchange rate snapshots per (tenant, base, quote) with effective timestamp and source. |
| **Accrual Templates** | Recurring accrual patterns: accounts, amount, currency, reversal policy, cash flow classification. |
| **Accrual Instances** | Per-period accrual postings linked to templates. Append-only with deterministic IDs. |
| **Accrual Reversals** | Exactly-once reversal records linked to original accrual instances. |
| **Revenue Contracts** | ASC 606 contract headers: customer, dates, total transaction price, status. |
| **Performance Obligations** | Distinct promises within a contract: allocated amount, recognition pattern, satisfaction dates. |
| **Recognition Schedules** | Versioned amortization tables per obligation with period-by-period line items. |
| **Contract Modifications** | Append-only amendment ledger: modification type, effective date, price changes, catch-up flags. |
| **Cash Flow Classifications** | Account-to-category mappings: operating, investing, or financing. |
| **Failed Events (DLQ)** | Events that failed processing after retries: envelope, error, retry count. |
| **Processed Events** | Idempotency tracking: event_id deduplication for all consumers. |

GL is **NOT** authoritative for:
- Customer or vendor master data (AR/AP modules own this)
- Invoice or bill details (AR/AP modules own the source documents)
- Inventory stock levels or cost layers (Inventory module owns this)
- Fixed asset register, depreciation schedules, or net book value (Fixed Assets module owns this)
- Tax rules, rates, or filing obligations (AR tax engine calculates, GL only records the liability)
- Labor hours or timekeeping entries (Timekeeping module owns this)

---

## Data Ownership

### Tables Owned by GL

All tables use `tenant_id` for multi-tenant isolation. Every query **MUST** filter by `tenant_id`.

| Table | Purpose | Key Fields |
|-------|---------|------------|
| **journal_entries** | Journal entry headers with source tracking | `id`, `tenant_id`, `source_module`, `source_event_id` (UNIQUE), `source_subject`, `posted_at`, `currency`, `description`, `reference_type`, `reference_id`, `reverses_entry_id` (nullable, self-FK), `correlation_id` (nullable) |
| **journal_lines** | Debit/credit lines per entry | `id`, `journal_entry_id` (FK), `line_no`, `account_ref`, `debit_minor` (BIGINT >= 0), `credit_minor` (BIGINT >= 0), `memo`. UNIQUE(journal_entry_id, line_no) |
| **accounts** | Chart of Accounts | `id`, `tenant_id`, `code`, `name`, `type` (account_type enum), `normal_balance` (normal_balance enum), `is_active`. UNIQUE(tenant_id, code) |
| **accounting_periods** | Fiscal period definitions with close lifecycle | `id`, `tenant_id`, `period_start`, `period_end`, `is_closed`, `close_requested_at`, `closed_at`, `closed_by`, `close_reason`, `close_hash`, `reopen_count`, `last_reopened_at`. EXCLUDE non-overlapping per tenant |
| **account_balances** | Materialized balance rollups | `id`, `tenant_id`, `period_id` (FK), `account_code`, `currency`, `debit_total_minor`, `credit_total_minor`, `net_balance_minor`, `last_journal_entry_id`. UNIQUE(tenant_id, period_id, account_code, currency) |
| **period_summary_snapshots** | Pre-aggregated period stats | `id`, `tenant_id`, `period_id` (FK), `currency`, `journal_count`, `line_count`, `total_debits_minor`, `total_credits_minor`, `checksum`. UNIQUE(tenant_id, period_id, currency) |
| **close_calendar** | Period close scheduling | `id`, `tenant_id`, `period_id` (FK), `expected_close_date`, `owner_role`, `reminder_offset_days` (integer[]), `overdue_reminder_interval_days`, `notes`. UNIQUE(tenant_id, period_id) |
| **close_calendar_reminders_sent** | Idempotent reminder tracking | `id`, `tenant_id`, `calendar_entry_id` (FK), `reminder_type`, `reminder_key`. UNIQUE(tenant_id, calendar_entry_id, reminder_key) |
| **close_checklist_items** | Pre-close checklist | `id`, `tenant_id`, `period_id` (FK), `label`, `status` (pending\|complete\|waived), `completed_by`, `completed_at`, `waive_reason` |
| **close_approvals** | Close signoffs | `id`, `tenant_id`, `period_id` (FK), `actor_id`, `approval_type`, `notes`. UNIQUE(tenant_id, period_id, approval_type) |
| **period_reopen_requests** | Reopen audit trail | `id`, `tenant_id`, `period_id` (FK), `requested_by`, `reason`, `prior_close_hash`, `status` (requested\|approved\|rejected), `approved_by`, `approved_at`, `rejected_by`, `rejected_at`, `reject_reason` |
| **fx_rates** | Append-only FX rate store | `id`, `tenant_id`, `base_currency`, `quote_currency`, `rate`, `inverse_rate`, `effective_at`, `source`, `idempotency_key` (UNIQUE) |
| **gl_accrual_templates** | Recurring accrual definitions | `template_id` (UNIQUE), `tenant_id`, `name`, `debit_account`, `credit_account`, `amount_minor`, `currency`, `reversal_policy` (JSONB), `cashflow_class`, `active` |
| **gl_accrual_instances** | Per-period accrual postings | `instance_id` (UNIQUE), `template_id` (FK), `tenant_id`, `accrual_id` (UNIQUE), `period`, `posting_date`, `amount_minor`, `currency`, `journal_entry_id`, `status`, `idempotency_key` (UNIQUE). UNIQUE(template_id, period) |
| **gl_accrual_reversals** | Exactly-once reversal records | `reversal_id` (UNIQUE), `original_accrual_id` (UNIQUE), `original_instance_id` (FK), `tenant_id`, `reversal_period`, `reversal_date`, `amount_minor`, `currency`, `journal_entry_id`, `idempotency_key` (UNIQUE) |
| **revrec_contracts** | ASC 606 revenue contracts | `contract_id` (PK), `tenant_id`, `customer_id`, `contract_name`, `contract_start`, `contract_end`, `total_transaction_price_minor`, `currency`, `status` |
| **revrec_obligations** | Performance obligations | `obligation_id` (PK), `contract_id` (FK), `tenant_id`, `name`, `allocated_amount_minor`, `recognition_pattern` (JSONB), `satisfaction_start`, `satisfaction_end`, `status` |
| **revrec_schedules** | Versioned recognition schedules | `schedule_id` (PK), `contract_id` (FK), `obligation_id` (FK), `tenant_id`, `total_to_recognize_minor`, `currency`, `first_period`, `last_period`, `version`, `previous_schedule_id` (FK, nullable) |
| **revrec_schedule_lines** | Amortization table entries | `id`, `schedule_id` (FK), `period`, `amount_to_recognize_minor`, `deferred_revenue_account`, `recognized_revenue_account`, `recognized`, `recognized_at`. UNIQUE(schedule_id, period) |
| **revrec_contract_modifications** | Amendment ledger | `modification_id` (PK), `contract_id` (FK), `tenant_id`, `modification_type`, `effective_date`, `new_transaction_price_minor`, `reason`, `requires_cumulative_catchup` |
| **cashflow_classifications** | Account → cash flow category | `id`, `tenant_id`, `account_code`, `category` (cashflow_category enum: operating\|investing\|financing). UNIQUE(tenant_id, account_code) |
| **events_outbox** | Transactional outbox | Standard platform schema with envelope metadata (tenant_id, source_module, trace_id, correlation_id, mutation_class, etc.) |
| **processed_events** | Event deduplication | `event_id` (UNIQUE), `event_type`, `processor` |
| **failed_events** | Dead letter queue | `event_id` (UNIQUE), `subject`, `tenant_id`, `envelope_json`, `error`, `retry_count` |

**Monetary Precision:** All monetary amounts use **integer minor units** (e.g., cents as BIGINT). Currency stored as ISO 4217 code.

### Data NOT Owned by GL

GL **MUST NOT** store:
- Customer or vendor master records
- Invoice, bill, or payment document details (only references via source_doc_id)
- Inventory quantities, lot codes, or cost layers
- Fixed asset acquisition cost, useful life, or depreciation method (only the resulting journal entries)
- Tax rates, tax rules, or jurisdiction configuration
- Employee or contractor master data
- Timesheet entries or labor hour details

---

## Events Consumed

| Event Subject | Source Module | Action |
|---------------|-------------|--------|
| `gl.events.posting.requested` | Any (via platform contract) | Create balanced journal entry with COA validation, period enforcement, balance update |
| `gl.events.entry.reverse.requested` | Any | Create reversal entry (negate original lines) with chain depth check |
| `ar.invoice_written_off` | AR | DR Bad Debt Expense / CR AR |
| `ar.credit_note_issued` | AR | DR Revenue / CR AR |
| `ar.invoice_settled_fx` | AR | DR/CR AR and FX Realized Gain/Loss (based on settlement vs recognition rate) |
| `tax.committed` | AR | DR Tax Collected / CR Tax Payable |
| `tax.voided` | AR | DR Tax Payable / CR Tax Collected (reverse committed tax) |
| `inventory.item_issued` | Inventory | DR COGS / CR Inventory |
| `ap.vendor_bill_approved` | AP | DR Expense (or AP Clearing for PO-backed) / CR AP. Multi-currency via fx_rate_id |
| `fa_depreciation_run.depreciation_run_completed` | Fixed Assets | DR Depreciation Expense / CR Accumulated Depreciation (per schedule period) |
| `timekeeping.labor_cost` | Timekeeping | DR Labor Expense / CR Accrued Labor |

---

## Events Produced

| Event | Trigger | Key Payload Fields |
|-------|---------|-------------------|
| `gl.accrual_created` | Accrual instance posted | `accrual_id`, `template_id`, `period`, `debit_account`, `credit_account`, `amount_minor`, `currency`, `cashflow_class`, `reversal_policy` |
| `gl.accrual_reversed` | Accrual reversal posted | `reversal_id`, `original_accrual_id`, `reversal_period`, `amount_minor`, `currency` |
| `fx.rate_updated` | FX rate created | `rate_id`, `base_currency`, `quote_currency`, `rate`, `inverse_rate`, `effective_at`, `source` |
| `gl.fx_revaluation_posted` | Unrealized FX gain/loss posted | Revaluation details with journal entry reference |
| `gl.fx_realized_posted` | Realized FX gain/loss posted | Settlement FX details with journal entry reference |
| `revrec.contract_created` | Revenue contract created | `contract_id`, `customer_id`, obligations list |
| `revrec.schedule_created` | Recognition schedule generated | `schedule_id`, `contract_id`, `obligation_id`, schedule lines |
| `revrec.recognition_posted` | Revenue recognized for a period | `contract_id`, `obligation_id`, `period`, `amount_minor` |
| `revrec.contract_modified` | Contract amended | `modification_id`, `contract_id`, `modification_type`, allocation changes |

---

## Integration Points

### AR (Event-Driven, Inbound)
GL consumes AR events for invoice posting, credit note posting, write-off posting, tax liability tracking, and realized FX gain/loss. GL never calls AR.

### AP (Event-Driven, Inbound)
GL consumes `ap.vendor_bill_approved` to post expense/liability entries. Supports multi-currency via FX rate lookup. GL never calls AP.

### Inventory (Event-Driven, Inbound)
GL consumes `inventory.item_issued` to post COGS entries (DR COGS / CR Inventory). GL never calls Inventory.

### Fixed Assets (Event-Driven, Inbound)
GL consumes depreciation run events to post depreciation journal entries. Account refs (expense and accumulated depreciation) come from the Fixed Assets module in the event payload. GL never calls Fixed Assets.

### Timekeeping (Event-Driven, Inbound)
GL consumes `timekeeping.labor_cost` events to post labor cost accruals. GL never calls Timekeeping.

### Maintenance (Future, Not Yet Implemented)
A future GL consumer will subscribe to `maintenance.work_order.completed` to post maintenance cost journal entries. Not part of the current GL module — this would be a new consumer bead.

### Notifications (Event-Driven, Outbound)
GL emits events that Notifications can subscribe to for close reminders, overdue alerts, and accrual notifications. GL never calls Notifications.

---

## Invariants

1. **Every journal entry is balanced.** SUM(debit_minor) = SUM(credit_minor) per entry. Enforced at validation, tested by `assert_all_entries_balanced`.
2. **No duplicate postings.** `source_event_id` is UNIQUE on `journal_entries`. Duplicate events silently skip via `processed_events` check.
3. **All account references are valid.** Every `journal_lines.account_ref` must exist in `accounts` and be active for the tenant. Enforced in-transaction by `validate_accounts_against_coa`.
4. **No posting into closed periods.** Journal entries with `posted_at` inside a closed period are rejected. Enforced by `period_repo` validation, tested by `assert_no_closed_period_postings`.
5. **Line numbers are unique per entry.** UNIQUE(journal_entry_id, line_no) constraint.
6. **Reversal chain depth ≤ 1.** If entry A reverses B, then B must not itself be a reversal. Enforced by `assert_max_reversal_chain_depth`.
7. **Tenant isolation is unbreakable.** Every query filters by `tenant_id`. No cross-tenant data leakage.
8. **Accounting periods never overlap.** EXCLUDE constraint with `btree_gist` on `(tenant_id, daterange)`.
9. **Closed period requires hash.** Database CHECK constraint: if `closed_at IS NOT NULL` then `close_hash IS NOT NULL`.
10. **Accrual exactly-once.** UNIQUE(template_id, period) on instances. UNIQUE(original_accrual_id) on reversals.
11. **FX rates are append-only.** No UPDATE or DELETE on `fx_rates`. Idempotency via `idempotency_key`.
12. **Balances are deterministically rebuildable.** `rebuild_balances` can reconstruct all balances from journals. Same inputs always produce same outputs.

---

## API Surface (Summary)

### Operational
- `GET /healthz` — Liveness check
- `GET /api/health` — Health check
- `GET /api/ready` — Readiness check
- `GET /api/version` — Version info
- `GET /metrics` — Prometheus metrics

### Financial Statements (Read)
- `GET /api/gl/trial-balance` — Trial balance (query: tenant_id, period_id, currency)
- `GET /api/gl/income-statement` — Income statement (P&L)
- `GET /api/gl/balance-sheet` — Balance sheet
- `GET /api/gl/cash-flow` — Cash flow statement

### Reporting Currency Statements (Read)
- `GET /api/gl/reporting/trial-balance` — Trial balance translated to reporting currency
- `GET /api/gl/reporting/income-statement` — Income statement in reporting currency
- `GET /api/gl/reporting/balance-sheet` — Balance sheet in reporting currency

### Account Activity & Detail (Read)
- `GET /api/gl/accounts/{account_code}/activity` — Transaction-level detail for one account
- `GET /api/gl/detail` — Multi-account transaction detail with filtering

### Period Management
- `GET /api/gl/periods/{period_id}/summary` — Period summary snapshot
- `GET /api/gl/periods/{period_id}/close-status` — Current close state
- `POST /api/gl/periods/{period_id}/validate-close` — Pre-flight close validation (requires gl.post)
- `POST /api/gl/periods/{period_id}/close` — Atomic close with hash (requires gl.post)
- `GET /api/gl/periods/{period_id}/checklist` — Close checklist status
- `POST /api/gl/periods/{period_id}/checklist` — Create checklist item (requires gl.post)
- `POST /api/gl/periods/{period_id}/checklist/{item_id}/complete` — Complete item (requires gl.post)
- `POST /api/gl/periods/{period_id}/checklist/{item_id}/waive` — Waive item (requires gl.post)
- `GET /api/gl/periods/{period_id}/approvals` — List approvals
- `POST /api/gl/periods/{period_id}/approvals` — Create approval (requires gl.post)
- `POST /api/gl/periods/{period_id}/reopen` — Request reopen (requires gl.post)
- `GET /api/gl/periods/{period_id}/reopen` — List reopen requests
- `POST /api/gl/periods/{period_id}/reopen/{request_id}/approve` — Approve reopen (requires gl.post)
- `POST /api/gl/periods/{period_id}/reopen/{request_id}/reject` — Reject reopen (requires gl.post)

### FX Rates
- `POST /api/gl/fx-rates` — Create FX rate snapshot (requires gl.post)
- `GET /api/gl/fx-rates/latest` — Get latest rate for a currency pair

### Accruals
- `POST /api/gl/accruals/templates` — Create accrual template (requires gl.post)
- `POST /api/gl/accruals/create` — Create accrual instance for a period (requires gl.post)
- `POST /api/gl/accruals/reversals/execute` — Execute accrual reversals (requires gl.post)

### Revenue Recognition
- `POST /api/gl/revrec/contracts` — Create revenue contract with obligations (requires gl.post)
- `POST /api/gl/revrec/schedules` — Generate recognition schedule (requires gl.post)
- `POST /api/gl/revrec/recognition-runs` — Run recognition for a period (requires gl.post)
- `POST /api/gl/revrec/amendments` — Amend a contract (requires gl.post)

### Admin
- Admin routes for projection management and consistency checks (via `admin_router`): `POST /api/gl/admin/projection-status`, `POST /api/gl/admin/consistency-check`, `GET /api/gl/admin/projections` — all require `X-Admin-Token` header
- `rebuild_balances` CLI tool for balance recomputation

---

## Document Standards Reference

This document follows the revision and decision log standards defined at:
`docs/frontend/DOC-REVISION-STANDARDS.md`

---

## Decision Log

Every significant product, architecture, or standards decision is recorded here. Do not re-open a decision without adding a new row that supersedes the old one.

| Date | Decision | Rationale | Decided By |
|------|----------|-----------|-----------|
| 2026-02-12 | Journal entries are source of truth — balances are derived read model | Balances can be rebuilt from journals; audit integrity anchored to append-only record; balance schema changes are non-destructive | Platform Orchestrator |
| 2026-02-12 | GL is a pure event consumer — never calls other modules synchronously | Zero runtime dependencies; independent deployment and testing; availability not coupled to source module health | Platform Orchestrator |
| 2026-02-12 | Amounts stored as integer minor units (BIGINT cents) | Eliminates floating-point rounding errors in financial calculations; industry standard for accounting systems | Platform Orchestrator |
| 2026-02-12 | Idempotency via UNIQUE source_event_id + processed_events table | Duplicate events silently skipped; replay is always safe; no client-side dedup burden | Platform Orchestrator |
| 2026-02-12 | DLQ with structured retry classification | Validation errors (non-retriable) go to DLQ immediately; database errors retry with exponential backoff; prevents infinite retry loops on bad data | Platform Orchestrator |
| 2026-02-13 | Flat Chart of Accounts (no hierarchy in v1) | Keeps account model simple; hierarchy adds query complexity and UI requirements; sub-ledger rollups can be added later without schema change | Platform Orchestrator |
| 2026-02-13 | Accounting periods with EXCLUDE constraint for non-overlap | Database-enforced guarantee that no two periods overlap per tenant; uses btree_gist extension; eliminates application-level overlap checking | Platform Orchestrator |
| 2026-02-13 | Balance grain = (tenant, period, account, currency) | Multi-currency native from day one; per-period rollups enable fast trial balance queries; avoids redesign when currency support is needed | Platform Orchestrator |
| 2026-02-13 | Period close creates SHA-256 hash of snapshot | Tamper detection for audit compliance; hash verifiable at any later point; proves period data hasn't changed since close | Platform Orchestrator |
| 2026-02-13 | One-level reversal chain maximum | Prevents ambiguous "reversal of a reversal" scenarios; keeps it clear which entries are active vs compensated; enforced by invariant check | Platform Orchestrator |
| 2026-02-14 | Period close lifecycle: validate → close (atomic) with idempotency via closed_at | Validation is a separate non-mutating pre-flight; close is atomic with snapshot + hash; already-closed periods return existing status without mutation | Platform Orchestrator |
| 2026-02-16 | Full EventEnvelope metadata on outbox (trace_id, correlation_id, mutation_class) | Cross-module distributed tracing; correlation_id propagated from source events enables "show all GL entries for invoice X" queries | Platform Orchestrator |
| 2026-02-17 | Revenue recognition (ASC 606) lives inside GL module, not a separate service | RevRec schedules post journal entries to GL — tight coupling is intentional; separate service would require distributed transactions for recognition runs | Platform Orchestrator |
| 2026-02-17 | FX rates are append-only with idempotency key | Preserves full rate history for audit; latest-as-of lookups find most recent rate; no update/delete prevents accidental history rewriting | Platform Orchestrator |
| 2026-02-17 | Accrual templates and instances with deterministic UUIDs | template_id + period deterministically produces accrual_id via Uuid::v5; makes accrual processing fully idempotent and replay-safe | Platform Orchestrator |
| 2026-02-17 | Recognition schedules are versioned, never rewritten | Append-only schedule versioning with previous_schedule_id lineage; preserves full recognition history for ASC 606 compliance | Platform Orchestrator |
| 2026-02-18 | Close calendar with configurable reminder schedules | Reminder offset days as integer array (e.g., {7,3,1}); overdue reminders on configurable interval; idempotent reminder tracking prevents spam | Platform Orchestrator |
| 2026-02-18 | Pre-close checklist and approval signoffs | Configurable quality gate before period close; items can be completed or waived (with reason); approvals are typed and unique per period | Platform Orchestrator |
| 2026-02-18 | Period reopen requires explicit request → approval workflow | Reopening is exceptional and auditable; prior close hash captured at request time; append-only audit trail; reopen_count monotonically increases | Platform Orchestrator |
| 2026-02-18 | AP consumer supports multi-currency via fx_rate_id lookup | When bill is in foreign currency, rate is looked up from fx_rates by UUID; amounts converted to reporting currency; same-currency bills posted as-is | Platform Orchestrator |
| 2026-02-24 | No mocking in tests — integrated tests against real services | Platform-wide standard; all verification hits real Postgres and real NATS | Platform Orchestrator |
| 2026-02-24 | Tenant isolation via tenant_id on every table | Standard platform multi-tenant pattern; all indexes include tenant_id as leading column | Platform Orchestrator |
