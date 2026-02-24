# Treasury Module — Vision & Specification

> **Module:** `treasury` · **Version:** 0.1.0 · **Port:** 8094
> **Status:** MVP complete — unproven module

---

## Revision History

| Rev | Date | Author | Summary |
|-----|------|--------|---------|
| 1.0 | 2026-02-24 | SageDesert (agent) | Initial vision doc from source audit |
| 1.1 | 2026-02-24 | CopperRiver (agent) | Fresh-eyes review: fix 18 inaccuracies against source code |

---

## 1. Business Problem

Businesses that collect payments (AR) and pay vendors (AP) need a unified view of
their actual bank and credit-card positions. Without a treasury layer the
platform can tell you *what is owed* but not *what is in the bank*. Three pain
points drive the module:

1. **Cash visibility** — "How much cash do we actually have right now?" requires
   aggregating balances across bank accounts and credit cards, not just ledger
   entries.
2. **Reconciliation** — Imported bank statements must be matched against
   internally recorded transactions so discrepancies surface quickly.
3. **Forecasting** — Combining AR aging (money coming in) with AP aging (money
   going out) produces a forward-looking cash forecast that informs spending
   decisions.

Treasury bridges the gap between *internal records* (GL, AR, AP) and *external
reality* (bank statements).

---

## 2. What the Module Does

| Capability | Description |
|------------|-------------|
| **Account management** | Create, update, list, deactivate bank and credit-card accounts per tenant |
| **Statement import** | Parse CSV bank/CC statements with issuer-specific adapters (Chase, Amex) and auto-format detection |
| **Bank reconciliation** | Auto-match imported statement lines to recorded transactions using pluggable strategies with confidence scoring |
| **Manual reconciliation** | Human-driven matching when auto-match cannot find a pair |
| **GL linkage** | Soft-reference link between matched bank transactions and GL journal entries (Treasury never queries GL) |
| **Cash position** | Real-time balance reporting: bank cash vs credit-card liability, per account and aggregate |
| **Cash forecast** | Deterministic projection from AR/AP aging buckets and scheduled payment runs |
| **Event consumers** | Listens for `payment.succeeded` (AR) and `ap.payment_executed` (AP) to auto-record bank transactions |

---

## 3. Who Uses This

| Actor | Interaction |
|-------|-------------|
| **Finance team** | Imports statements, reviews reconciliation, checks cash position |
| **AR module** | Publishes `payment.succeeded` events consumed by Treasury |
| **AP module** | Publishes `ap.payment_executed` events consumed by Treasury |
| **GL module** | Receives soft-reference links from Treasury for journal-entry reconciliation |
| **Reports / dashboards** | Reads cash-position and forecast endpoints |

---

## 4. Design Principles

1. **Guard → Mutation → Outbox atomicity** — Every write operation validates
   first (guard), then performs the mutation and enqueues the domain event in a
   single database transaction.
2. **Three-layer idempotency** — API-level (idempotency keys table),
   import-level (UUID v5 statement content hash), event-level (`processed_events`
   table). No duplicate side effects.
3. **Pluggable match strategies** — The reconciliation engine delegates scoring
   to a `MatchStrategy` trait so bank and credit-card matching use different
   heuristics without forking the engine.
4. **Append-only reconciliation** — Matches are never deleted. Re-matching
   supersedes old matches via a `superseded_by` pointer.
5. **Soft references for GL** — Treasury stores GL entry IDs as opaque BIGINTs.
   It never queries the GL database, preserving module boundaries.
6. **Multi-tenant isolation** — Every table row carries `app_id`. Database
   naming convention: `treasury_{app_id}_db`.
7. **Pure-function forecasting** — The forecast computation is a pure function
   over AR/AP aging inputs and assumptions, making it deterministic and testable.

---

## 5. MVP / Current Scope

### Shipped

- Bank and credit-card account CRUD with validation
- CSV statement import with generic, Chase CC, and Amex CC adapters
- Auto-format detection from CSV headers
- Auto-match engine with `BankStrategy` and `CreditCardStrategy`
- Manual match with supersede semantics
- GL soft-reference linkage
- Cash position (bank cash + CC liability buckets)
- Cash forecast from AR/AP aging with configurable assumptions
- NATS event consumers for payment succeeded and AP payment executed (handlers implemented; not yet wired into main.rs startup)
- Transactional outbox with 1-second publisher loop
- Prometheus metrics (import, recon, SLO, latency)
- JWT authentication with `TREASURY_MUTATE` permission for writes
- Rate limiting, request timeout, CORS, tracing context middleware

### Not Yet Built

- OFX / QFX / MT940 statement import formats
- Multi-currency forecast aggregation (currently per-currency)
- Recurring / scheduled reconciliation runs
- Bank feed API integrations (Plaid, MX, Yodlee)
- Statement archival and retention policies
- Reconciliation approval workflow

---

## 6. Technology Summary

| Concern | Choice |
|---------|--------|
| Language | Rust (edition 2021) |
| HTTP framework | Axum 0.8 (multipart enabled) |
| Database | PostgreSQL via SQLx 0.8 (compile-time checked queries off, runtime) |
| Migrations | SQLx migrate (`./db/migrations/`) |
| Event bus | NATS (async-nats 0.33) or in-memory, selected by `BUS_TYPE` env |
| Auth | `security` platform crate — JWT verification, `RequirePermissionsLayer` |
| Health checks | `health` platform crate — readiness with DB latency |
| Metrics | Prometheus 0.13 — counters, gauges, histograms |
| CSV parsing | `csv` crate 1.x |
| Decimal math | `rust_decimal` 1.x (serde-with-str) |
| Async runtime | Tokio (full features) |

---

## 7. Structural Decisions

### 7.1 Unified Account Table for Bank and Credit Card

**Decision:** A single `treasury_bank_accounts` table holds both bank and
credit-card accounts, distinguished by `account_type` enum.

**Rationale:** Bank and CC accounts share 90% of their fields (name,
institution, last-4, currency, balance). CC-specific fields (`credit_limit_minor`,
`statement_closing_day`, `cc_network`) are nullable columns. This avoids table
proliferation and simplifies list/get queries. The reconciliation engine selects
strategy by account type, so no polymorphism leaks into storage.

### 7.2 Pluggable Match Strategies via Trait

**Decision:** `MatchStrategy` trait with `score(statement_line, transaction) -> Option<Decimal>`.
Two implementations: `BankStrategy` (amount + date proximity + reference
similarity) and `CreditCardStrategy` (amount + auth/settle date window +
merchant name matching).

**Rationale:** Bank statements and credit-card statements have fundamentally
different matching signals. Banks expose references and dates; credit cards
expose auth dates, settle dates, and merchant names. The trait boundary lets
each strategy encapsulate its domain without conditional logic in the engine.

### 7.3 Append-Only Reconciliation Matches

**Decision:** `treasury_recon_matches` rows are never updated or deleted. A
rematch inserts a new row and sets `superseded_by` on the old one. The
`statement_line_id` column (added in migration 5) enables this tracking.

**Rationale:** Reconciliation is an audit-sensitive operation. Append-only
history ensures every match decision is preserved for audit trail. Supersede
semantics allow corrections without losing the original record.

### 7.4 UUID v5 Content Hash for Statement Dedup

**Decision:** On import, the raw CSV content is hashed with UUID v5 (using a
fixed namespace). The hash is stored in `treasury_bank_statements.statement_hash`
with a unique index. Re-importing the same file is a no-op.

**Rationale:** Users commonly re-upload the same statement file. Content hashing
catches exact duplicates at the storage layer, independent of filename or upload
timestamp. UUID v5 is deterministic and collision-resistant for this purpose.

### 7.5 Issuer-Specific CSV Adapters with Auto-Detection

**Decision:** `CsvFormat` enum (Generic, ChaseCredit, AmexCredit) with
`detect_format()` that inspects CSV headers. Each adapter normalises to a
common `ParsedLine` model.

**Rationale:** Every bank/issuer uses a different CSV layout. Chase includes
Post Date + Category + Type columns; Amex flips the sign convention (positive =
charge). Auto-detection from headers removes the burden of format selection from
users while adapters encapsulate issuer quirks (e.g., Amex sign flip).

### 7.6 Cross-Module Reads for Forecasting

**Decision:** The forecast endpoint accepts optional `AR_DATABASE_URL` and
`AP_DATABASE_URL` environment variables. It reads directly from `ar_aging_buckets`,
`vendor_bills`, `ap_allocations`, and `payment_runs` tables in those databases.

**Rationale:** Forecasting requires AR and AP data but does not own it. Rather
than duplicating data via events, the module performs read-only cross-database
queries. This is acceptable because forecasting is a *read* operation with no
write side effects, and the data is point-in-time anyway.

### 7.7 Soft GL References

**Decision:** `treasury_recon_matches.gl_entry_id` is a nullable BIGINT column
with no foreign key. Treasury stores the ID but never joins to or queries the GL
database.

**Rationale:** Module boundaries must be respected. Treasury links a matched
bank transaction to a GL entry for downstream reporting, but the GL module owns
the journal entries. A soft reference keeps Treasury decoupled — if GL is
unavailable, Treasury still functions. The GL module or a reporting layer can
join on these IDs when needed.

---

## 8. Open Questions

| # | Question | Impact |
|---|----------|--------|
| 1 | Should forecast assumptions be configurable per tenant? | Currently hardcoded defaults; tenant-specific overrides would need a table |
| 2 | How should partial statement overlaps be handled? | Current dedup is all-or-nothing by content hash; overlapping date ranges in different files could create duplicate lines |
| 3 | Should recon matches have an approval workflow? | Current model is immediate — auto-match runs and results are final until superseded |
| 4 | Bank feed API integration approach (Plaid vs MX vs direct)? | Determines whether statement import remains CSV-only or gains real-time feeds |

---

## 9. Domain Authority

Treasury is the **single authority** for:

- Bank and credit-card account metadata and balances
- Imported bank statement data (raw lines and parsed transactions)
- Reconciliation match decisions (who matched what, when, with what confidence)
- Cash position calculations
- Cash forecast computations

Treasury does **not** own:

- GL journal entries (owned by GL module — soft references only)
- AR invoices or payment records (owned by AR module — consumed via events)
- AP vendor bills or payment runs (owned by AP module — read for forecasting)
- Payment processing (owned by Payments module)

---

## 10. Data Ownership

### Tables

| Table | Purpose | Key Columns |
|-------|---------|-------------|
| `treasury_bank_accounts` | Bank and CC account registry | `id`, `app_id`, `account_name`, `account_type`, `institution`, `account_number_last4`, `currency`, `current_balance_minor`, `status`, `credit_limit_minor`, `statement_closing_day`, `cc_network` |
| `treasury_bank_statements` | Imported statement metadata | `id`, `app_id`, `account_id`, `period_start`, `period_end`, `source_filename`, `opening_balance_minor`, `closing_balance_minor`, `currency`, `status`, `statement_hash`, `imported_at` |
| `treasury_bank_transactions` | Parsed transaction lines | `id`, `app_id`, `account_id`, `statement_id`, `amount_minor`, `currency`, `transaction_date`, `description`, `reference`, `external_id`, `status`, `auth_date`, `settle_date`, `merchant_name`, `merchant_category_code` |
| `treasury_recon_matches` | Reconciliation decisions | `id`, `app_id`, `statement_line_id`, `bank_transaction_id`, `match_type` (auto/manual/suggested), `status` (pending/confirmed/rejected), `confidence_score`, `gl_entry_id` (BIGINT), `superseded_by`, `matched_by` |
| `events_outbox` | Transactional outbox for domain events | `id` (BIGSERIAL), `event_id` (UUID), `event_type`, `aggregate_type`, `aggregate_id`, `payload`, `created_at`, `published_at` |
| `processed_events` | Idempotent event consumption tracking | `id` (BIGSERIAL PK), `event_id` (UUID UNIQUE), `event_type`, `processed_at`, `processor` |
| `treasury_idempotency_keys` | API-level idempotency | `id` (BIGSERIAL PK), `app_id`, `idempotency_key` (unique with app_id), `request_hash`, `status_code`, `response_body`, `expires_at` |

### Enums (SQL)

| Enum | Values |
|------|--------|
| `treasury_account_status` | `active`, `inactive`, `closed` |
| `treasury_account_type` | `bank`, `credit_card` |
| `treasury_statement_status` | `pending`, `imported`, `reconciled` |
| `treasury_txn_status` | `unmatched`, `matched`, `excluded` |
| `treasury_recon_match_status` | `pending`, `confirmed`, `rejected` |
| `treasury_recon_match_type` | `auto`, `manual`, `suggested` |

---

## 11. State Machines

### Account Lifecycle

```
  ┌──────────┐   create    ┌────────┐
  │ (none)   │ ──────────▶ │ Active │
  └──────────┘             └────┬───┘
                                │ deactivate
                                ▼
                           ┌──────────┐
                           │ Inactive │
                           └────┬─────┘
                                │ close
                                ▼
                           ┌────────┐
                           │ Closed │
                           └────────┘
```

Deactivation and closure are one-way operations. The `treasury_account_status`
enum defines three states: `active`, `inactive`, `closed`. Inactive and closed
accounts cannot receive new transactions or statement imports.

### Statement Import

```
  Upload CSV ──▶ Parse ──▶ Insert statement + lines (tx) ──▶ Enqueue event
                   │
                   ▼ (parse errors)
              Partial result with error list
```

Import supports partial success — the result includes `lines_imported`,
`lines_skipped`, and an `errors` list for lines that could not be parsed. If the
content hash already exists, the import returns the previous result (idempotent
replay).

### Reconciliation Match

```
  ┌─────────┐  auto/manual   ┌───────────┐
  │ Pending │ ──────────────▶ │ Confirmed │
  └─────────┘                └─────┬─────┘
       │                           │ rematch (supersede)
       │ reject                    ▼
       ▼                     old row gets `superseded_by` set
  ┌──────────┐               new row inserted as `confirmed`
  │ Rejected │
  └──────────┘
```

Matches are never deleted. Rematch creates a new `confirmed` row and sets
`superseded_by` on the old one. The `treasury_recon_match_status` enum defines
three states: `pending`, `confirmed`, `rejected`.

---

## 12. Events

### Produced (via transactional outbox)

| Event Type | NATS Subject | Trigger | Payload |
|------------|-------------|---------|---------|
| `bank_account.created` | `treasury.events.bank_account.created` | Account creation | Account ID, app_id, account_type, name |
| `bank_account.updated` | `treasury.events.bank_account.updated` | Account update | Account ID, changed fields |
| `bank_account.deactivated` | `treasury.events.bank_account.deactivated` | Account deactivation | Account ID, app_id |
| `bank_statement.imported` | `treasury.events.bank_statement.imported` | Statement import | Statement ID, account_id, line count, total amount |
| `recon.auto_matched` | `treasury.events.recon.auto_matched` | Auto-match run | `matches_created` count, account_id |
| `recon.manual_matched` | `treasury.events.recon.manual_matched` | Manual match creation | Match ID, line ID, txn ID |
| `recon.gl_linked` | `treasury.events.recon.gl_linked` | GL entry linked | Match ID, bank_txn_id, gl_entry_id |

### Consumed (via NATS subscribers)

> **Note:** Consumer handler code exists in `consumers/payments.rs` but the
> subscriber tasks are not yet spawned in `main.rs`. Wiring is a pending task.

| Source Subject | Handler | Effect |
|----------------|---------|--------|
| `payments.events.payment.succeeded` | `handle_payment_succeeded` | Creates credit transaction (money in, +amount) on default bank account |
| `ap.events.ap.payment_executed` | `handle_ap_payment_executed` | Creates debit transaction (money out, −amount) on default bank account |

Both consumers use two-layer idempotency: `processed_events` table guard +
`external_id` unique constraint on transactions.

---

## 13. Integration Points

| Module | Direction | Mechanism | Detail |
|--------|-----------|-----------|--------|
| **AR** | AR → Treasury | NATS event `payment.succeeded` | Treasury records credit txn |
| **AP** | AP → Treasury | NATS event `ap.payment_executed` | Treasury records debit txn |
| **GL** | Treasury → GL | Soft reference (BIGINT) | `gl_entry_id` on recon matches; no FK, no query |
| **AR** (forecast) | Treasury reads AR | Cross-database SQL | `ar_aging_buckets` table for forecast inputs |
| **AP** (forecast) | Treasury reads AP | Cross-database SQL | `vendor_bills`, `ap_allocations`, `payment_runs` tables |
| **Security** | Platform → Treasury | Middleware | JWT verification, `TREASURY_MUTATE` permission, rate limiting |
| **Health** | Platform → Treasury | Crate | Readiness probe with DB latency check |
| **Projections** | Platform → Treasury | Crate | Admin endpoints for projection status |

---

## 14. Invariants

1. **Every write enqueues an outbox event** — No mutation commits without its
   corresponding domain event in the same transaction.
2. **Statement import is idempotent** — Re-uploading identical CSV content
   returns the same result; `statement_hash` unique index enforces this.
3. **Recon matches are append-only** — Old matches are superseded, never deleted
   or updated.
4. **Event consumption is idempotent** — `processed_events` table prevents
   duplicate processing; `external_id` unique constraint prevents duplicate
   transactions.
5. **Account deactivation is irreversible** — No path from `inactive` back to
   `active`.
6. **Currency consistency** — Manual match guards verify that statement line
   currency matches transaction currency before creating a match.
7. **Confidence range** — Auto-match scores are in `[0.5, 1.0]`; below 0.5 is
   not considered a match.
8. **App-id isolation** — All queries filter by `app_id`; no cross-tenant data
   leakage.

---

## 15. API Surface

### Health & Ops

| Method | Path | Auth | Description |
|--------|------|------|-------------|
| GET | `/healthz` | None | Liveness probe (platform health crate) |
| GET | `/api/health` | None | Legacy liveness — returns service name + version |
| GET | `/api/ready` | None | Readiness — verifies DB connectivity with latency |
| GET | `/api/version` | None | Module name, version, schema version |
| GET | `/metrics` | None | Prometheus metrics scrape endpoint |

### Accounts

| Method | Path | Auth | Description |
|--------|------|------|-------------|
| GET | `/api/treasury/accounts` | JWT | List accounts (query: `include_inactive`) |
| GET | `/api/treasury/accounts/:id` | JWT | Get single account by ID |
| POST | `/api/treasury/accounts/bank` | JWT + TREASURY_MUTATE | Create bank account |
| POST | `/api/treasury/accounts/credit-card` | JWT + TREASURY_MUTATE | Create credit-card account |
| PUT | `/api/treasury/accounts/:id` | JWT + TREASURY_MUTATE | Update account |
| POST | `/api/treasury/accounts/:id/deactivate` | JWT + TREASURY_MUTATE | Deactivate account |

### Statement Import

| Method | Path | Auth | Description |
|--------|------|------|-------------|
| POST | `/api/treasury/statements/import` | JWT + TREASURY_MUTATE | Upload CSV (multipart: `file` + optional `format`) |

### Reconciliation

| Method | Path | Auth | Description |
|--------|------|------|-------------|
| POST | `/api/treasury/recon/auto-match` | JWT + TREASURY_MUTATE | Run auto-match for an account |
| POST | `/api/treasury/recon/manual-match` | JWT + TREASURY_MUTATE | Create manual match |
| GET | `/api/treasury/recon/matches` | JWT | List matches (query filters) |
| GET | `/api/treasury/recon/unmatched` | JWT | List unmatched statement lines and transactions |

### GL Linkage

| Method | Path | Auth | Description |
|--------|------|------|-------------|
| POST | `/api/treasury/recon/gl-link` | JWT + TREASURY_MUTATE | Link bank transaction to GL entry |
| GET | `/api/treasury/recon/gl-unmatched-txns` | JWT | Bank transactions without GL link |
| POST | `/api/treasury/recon/gl-unmatched-entries` | JWT + TREASURY_MUTATE | Filter GL entry IDs to find unlinked ones |

### Reports

| Method | Path | Auth | Description |
|--------|------|------|-------------|
| GET | `/api/treasury/cash-position` | JWT | Cash position by account with summary |
| GET | `/api/treasury/forecast` | JWT | Cash forecast from AR/AP aging |

### Admin

| Method | Path | Auth | Description |
|--------|------|------|-------------|
| GET | `/api/treasury/admin/projections` | X-Admin-Token | List projection status |
| POST | `/api/treasury/admin/projection-status` | X-Admin-Token | Query projection status by name |
| POST | `/api/treasury/admin/consistency-check` | X-Admin-Token | Run consistency check |

---

## 16. Request/Response Headers

| Header | Direction | Purpose |
|--------|-----------|---------|
| `X-App-Id` | Request | Tenant identification (required on all business endpoints) |
| `X-Correlation-Id` | Request | Distributed tracing correlation |
| `X-Idempotency-Key` | Request | API-level idempotency for mutations |
| `Authorization` | Request | Bearer JWT token |

---

## 17. Metrics

| Metric | Type | Description |
|--------|------|-------------|
| `treasury_accounts_created_total` | Counter | Total accounts created |
| `treasury_transactions_recorded_total` | Counter | Total transactions recorded |
| `treasury_statements_imported_total` | Counter | Total statements imported |
| `treasury_open_transactions_count` | Gauge | Unreconciled transactions |
| `treasury_accounts_count` | Gauge | Total active accounts |
| `treasury_import_success_total` | Counter | Successful imports |
| `treasury_import_fail_total` | Counter | Failed imports |
| `treasury_recon_matched_total` | Gauge | Matched recon pairs (refreshed on scrape) |
| `treasury_recon_unmatched_lines` | Gauge | Unmatched statement lines |
| `treasury_recon_unmatched_txns` | Gauge | Unmatched transactions |
| `treasury_recon_match_rate` | Gauge | Match rate percentage |
| `treasury_http_request_duration_seconds` | Histogram | Per-endpoint latency |
| `treasury_http_requests_total` | Counter | Total HTTP requests (SLO) |
| `treasury_event_consumer_lag_messages` | Gauge | Event consumer lag |

---

## 18. Decision Log

| # | Decision | Date | Rationale |
|---|----------|------|-----------|
| 1 | Unified bank/CC account table | Pre-v0.1.0 | 90% field overlap; account_type enum differentiates; avoids table proliferation |
| 2 | Pluggable MatchStrategy trait | Pre-v0.1.0 | Bank vs CC matching use fundamentally different signals; trait boundary keeps engine generic |
| 3 | Append-only recon matches | Pre-v0.1.0 | Audit-sensitive domain; supersede semantics preserve full history |
| 4 | UUID v5 content hash for statement dedup | Pre-v0.1.0 | Deterministic, collision-resistant duplicate detection independent of filename |
| 5 | Issuer-specific CSV adapters | Pre-v0.1.0 | Each issuer has unique CSV layout and sign conventions; adapter pattern isolates quirks |
| 6 | Cross-database reads for forecast | Pre-v0.1.0 | Forecast is read-only; duplicating AR/AP data via events adds complexity without benefit |
| 7 | Soft GL references (no FK) | Pre-v0.1.0 | Preserves module boundary; Treasury functions even when GL is unavailable |
| 8 | 1-second outbox poll interval | Pre-v0.1.0 | Balance between event delivery latency and database load |
| 9 | Amex sign flip in adapter | Pre-v0.1.0 | Amex convention: positive = charge; platform convention: negative = debit. Adapter normalises at import boundary |
| 10 | Default bank account for event consumers | Pre-v0.1.0 | `payment.succeeded` and `ap.payment_executed` events don't specify a bank account; Treasury uses first active bank account for the app_id. Graceful skip if none configured |
