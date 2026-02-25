# Treasury Module — Vision & Roadmap

**Version**: 0.1.0
**Last Updated**: 2026-02-25
**Status**: Active

---

## 1. Mission

Treasury is the **cash management authority** — it tracks bank accounts, records bank transactions, processes statement imports, and reconciles bank activity against internal records. Treasury answers "what is our cash position?" and "are all payments accounted for?"

### Non-Goals

Treasury does **NOT**:
- Execute payments (owned by Payments module)
- Own vendor payables (owned by AP)
- Own customer receivables (owned by AR)
- Post GL journal entries directly (uses `gl.posting.requested`)

---

## 2. Domain Authority

| Domain Entity | Treasury Authority |
|---|---|
| **Bank Accounts** | Account definitions (checking, savings, credit card), balances |
| **Bank Transactions** | Individual transaction records from payments and statements |
| **Statement Imports** | Uploaded bank statement data with content hash dedup |
| **Reconciliation** | Statement line matching to internal transactions |

---

## 3. Data Ownership

| Table | Purpose |
|---|---|
| `bank_accounts` | Account definitions with type and currency |
| `bank_transactions` | Transaction records (deposits, withdrawals, fees) |
| `statement_imports` | Statement file uploads with content hash |
| `statement_lines` | Parsed lines from imported statements |
| `reconciliation_state` | Match status per statement line |
| `events_outbox` | Module outbox for NATS |
| `processed_events` | Consumer idempotency |

---

## 4. Events

**Produces:**
- None currently (internal state tracking)

**Consumes:**
- `payments.payment.succeeded` — auto-create bank transaction for AR payments
- `ap.payment_executed` — auto-create bank transaction for vendor payments

---

## 5. Key Invariants

1. Statement imports are deduplicated by content hash
2. Bank transactions created from events are idempotent
3. Reconciliation matches are permanent once confirmed
4. Tenant isolation on every table and query

---

## 6. Integration Map

- **Payments** → Treasury consumes payment success events to record deposits
- **AP** → Treasury consumes payment execution events to record withdrawals
- **GL** → future: bank reconciliation triggers GL adjustments
- **Reporting** → future: cash position data for dashboards

---

## 7. Roadmap

### v0.1.0 (current)
- Bank account CRUD
- Bank transaction recording (manual + event-driven)
- Statement import with content hash dedup
- Statement line parsing
- Bank reconciliation workflow
- Cash position reporting

### v1.0.0 (proven)
- Multi-currency account support with FX
- Automated reconciliation matching rules
- Cash flow forecasting
- Bank feed integration (plaid/direct API)
- GL posting for bank fees and adjustments
