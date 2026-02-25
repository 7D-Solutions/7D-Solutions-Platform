# General Ledger (GL) Module — Vision & Roadmap

**Version**: 0.1.0
**Last Updated**: 2026-02-25
**Status**: Active

---

## 1. Mission

The General Ledger is the **financial backbone** of the platform — the authoritative system for chart of accounts, journal entries, account balances, and period management. GL receives posting requests from other modules (AR, AP, Fixed Assets, Maintenance) via events and records double-entry journal entries. GL never initiates financial transactions; it only records them.

### Non-Goals

GL does **NOT**:
- Initiate invoices, payments, or billing (those modules emit `gl.posting.requested`)
- Store customer or vendor master data (owned by AR, AP, Party)
- Execute payment processing (owned by Payments)
- Own subscription logic (owned by Subscriptions)

---

## 2. Domain Authority

| Domain Entity | GL Authority |
|---|---|
| **Chart of Accounts** | Account structure, types (asset/liability/equity/revenue/expense), classifications, account hierarchies |
| **Journal Entries** | Double-entry transaction records with debit/credit lines |
| **Account Balances** | Period-based balance snapshots for reporting |
| **Periods** | Accounting period definitions, open/close status |
| **Accruals** | Accrual creation and automated reversal |
| **FX Rates** | Foreign exchange rate management, revaluation, realized gain/loss |
| **Revenue Recognition** | RevRec contracts, schedules, recognition posting |
| **Financial Statements** | Trial balance, income statement, balance sheet, cash flow |

---

## 3. Data Ownership

| Table | Purpose |
|---|---|
| `accounts` | Chart of accounts with type, classification, currency |
| `journal_entries` | Journal entry headers (date, description, status, source) |
| `journal_entry_lines` | Debit/credit line items per journal entry |
| `account_balances` | Period-based balance snapshots |
| `periods` | Accounting period definitions and close status |
| `accruals` | Accrual entries with auto-reversal scheduling |
| `fx_rates` | Exchange rate pairs with effective dates |
| `revrec_contracts` | Revenue recognition contract definitions |
| `revrec_schedules` | Recognition schedule per contract |
| `events_outbox` | Module outbox for NATS |
| `processed_events` | Consumer idempotency |

---

## 4. Events

**Produces:**
- `gl.posting.accepted` — journal entry successfully recorded
- `gl.posting.rejected` — posting request failed validation
- `gl.accrual_created` — new accrual entry posted
- `gl.accrual_reversed` — accrual auto-reversal posted
- `gl.fx_revaluation_posted` — FX revaluation journal entry created
- `gl.fx_realized_posted` — realized FX gain/loss posted
- `fx.rate_updated` — new exchange rate recorded
- `revrec.contract_created` — new RevRec contract
- `revrec.schedule_created` — recognition schedule generated
- `revrec.recognition_posted` — scheduled recognition posted to GL
- `revrec.contract_modified` — contract terms changed

**Consumes:**
- `gl.posting.requested` — from AR, AP, Fixed Assets, Maintenance
- `gl.reversal.requested` — reversal requests from other modules

---

## 5. Key Invariants

1. Every journal entry must balance (sum of debits == sum of credits)
2. Closed periods reject new postings
3. All consumers idempotent on event_id
4. Tenant isolation on every table and query
5. GL is a terminal node — never initiates cross-module writes

---

## 6. Integration Map

- **AR** → emits `gl.posting.requested` for invoice/payment GL entries
- **AP** → emits `gl.posting.requested` for vendor bill/payment GL entries
- **Fixed Assets** → emits `gl.posting.requested` for depreciation/disposal
- **Maintenance** → emits `gl.posting.requested` for work order costs
- **Consolidation** → reads GL data via HTTP API for multi-entity consolidation
- **Reporting** → consumes GL events for dashboard/report caching

---

## 7. Roadmap

### v0.1.0 (current)
- Chart of accounts CRUD
- Journal entry posting (from events)
- Period management (open/close)
- Trial balance, income statement, balance sheet, cash flow reports
- Accrual creation and auto-reversal
- FX rate management and revaluation
- Revenue recognition (contracts, schedules, posting)
- DLQ validation for rejected postings

### v1.0.0 (proven)
- Multi-currency consolidation support
- Intercompany elimination integration
- Audit trail with full journal entry history
- Period close checklist automation
- High-volume posting performance baselines
