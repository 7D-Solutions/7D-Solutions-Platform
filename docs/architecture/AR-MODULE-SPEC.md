# AR Module — Scope, Boundaries, Contracts (v0.1.x)

**7D Solutions Platform**
**Status:** Specification Document (Baseline Architecture)
**Date:** 2026-02-11
**Module Version:** 0.1.x

---

## 1. Mission & Non-Goals

### Mission
The Accounts Receivable (AR) module is the **authoritative system for customer billing, invoice lifecycle, and payment application tracking**. AR owns the financial truth of what customers owe and what they have paid. AR orchestrates payment collection by commanding the Payments module and reconciles results into the AR ledger. AR emits GL posting events for all financial state changes requiring general ledger recording.

### Non-Goals
AR does **NOT**:
- Execute payment processing or hold PCI-sensitive data (delegated to Payments module)
- Manage subscription logic, renewal calculations, or billing schedule generation (delegated to Subscriptions module)
- Store PII beyond what's required for billing (full CRM is out of scope)
- Send customer notifications (delegated to Notifications module)
- Directly write to GL tables (must use event-driven GL posting)
- Manage chart of accounts, account structures, or GL validation rules (delegated to GL module)

---

## 2. Domain Authority

AR is the **source of truth** for:

| Domain Entity | AR Authority |
|---------------|-------------|
| **Customers** | Billing-facing customer records: name, email, billing address, AR ledger balances, aging buckets, delinquency status |
| **Invoices** | Invoice lifecycle (draft → issued → paid/voided), amounts due, line items, service periods, issue dates, due dates |
| **AR Ledger** | Outstanding balances, aging (current, 30/60/90+ days past due), payment allocations to invoices |
| **Payment Applications** | Receipts linking payments to invoices, allocation rules, partial payment handling, over/under-payment tracking |
| **Credits & Adjustments** | Credit memos, write-offs, manual adjustments, balance corrections |
| **Refund Records** | Refund accounting artifacts (what was refunded, from which invoice/charge), not processor execution |
| **Disputes (AR View)** | Financial impact of disputes on AR ledger, chargeback amounts, dispute resolution outcomes |
| **Payment Method References** | Opaque references to payment methods (stored in Payments module), last4/brand/expiry metadata only |

AR is **NOT** authoritative for:
- Payment execution status (Payments module owns this)
- Subscription renewal logic or billing schedule generation (Subscriptions module owns this)
- GL account balances or journal entries (GL module owns this)

---

## 3. Data Ownership

### 3.1 Tables Owned by AR

All tables use `tenant_id` for multi-tenant isolation. Every query **MUST** filter by `tenant_id`.

| Table | Purpose | Key Fields |
|-------|---------|-----------|
| **ar_customers** | Billing-facing customer records | `id`, `tenant_id`, `email`, `name`, `ar_balance_cents`, `aging_current`, `aging_30`, `aging_60`, `aging_90_plus`, `delinquent_since`, `grace_period_end`, `next_retry_at`, `retry_attempt_count` |
| **ar_invoices** | Invoice records | `id`, `tenant_id`, `invoice_number`, `customer_id`, `status`, `subtotal_cents`, `tax_cents`, `total_cents`, `currency`, `issued_at`, `due_at`, `paid_at`, `voided_at` |
| **ar_invoice_line_items** | Invoice line items | `id`, `tenant_id`, `invoice_id`, `description`, `quantity`, `unit_price_cents`, `amount_cents`, `service_period_start`, `service_period_end`, `metadata` |
| **ar_payment_applications** | Payment allocations to invoices | `id`, `tenant_id`, `payment_id` (opaque ref), `invoice_id`, `allocated_cents`, `applied_at`, `allocation_type` (auto/manual) |
| **ar_credit_memos** | Credits issued to customers | `id`, `tenant_id`, `customer_id`, `credit_number`, `amount_cents`, `reason`, `issued_at`, `applied_to_invoice_id`, `applied_at` |
| **ar_adjustments** | Manual balance adjustments | `id`, `tenant_id`, `customer_id`, `invoice_id`, `adjustment_type` (write_off/dispute_adjustment/late_fee/other), `amount_cents`, `reason`, `created_by`, `created_at` |
| **ar_refund_records** | Refund accounting artifacts | `id`, `tenant_id`, `customer_id`, `invoice_id`, `charge_id`, `refund_id` (opaque ref to Payments), `amount_cents`, `reason`, `recorded_at`, `gl_posting_status` |
| **ar_dispute_records** | Dispute financial artifacts | `id`, `tenant_id`, `dispute_id` (opaque ref to Payments), `charge_id`, `invoice_id`, `status`, `amount_cents`, `reason_code`, `opened_at`, `closed_at`, `outcome` |
| **ar_payment_method_refs** | Payment method metadata (non-PCI) | `id`, `tenant_id`, `customer_id`, `payment_method_id` (opaque ref), `type`, `last4`, `brand`, `exp_month`, `exp_year`, `is_default` |
| **ar_ledger_events** | Immutable audit trail | `id`, `tenant_id`, `customer_id`, `invoice_id`, `event_type`, `amount_cents`, `balance_before_cents`, `balance_after_cents`, `occurred_at`, `event_id` (for idempotency) |
| **ar_gl_posting_queue** | GL posting reconciliation queue | `id`, `tenant_id`, `source_type`, `source_id`, `posting_event_id`, `status` (pending/accepted/rejected), `reason`, `created_at`, `resolved_at` |

**Monetary Precision:** All monetary amounts use **integer cents** (e.g., `amount_cents`) to avoid floating-point errors. For currencies with different minor units (e.g., JPY with 0 decimal places), amounts should still be stored as integers. Currency handling logic must account for minor unit differences per ISO 4217.

**Tenant Isolation:** Every table includes `tenant_id` as a non-nullable field. All indexes must include `tenant_id` as the first column. CI should enforce that no query is written without a `WHERE tenant_id = ?` clause.

### 3.2 Data NOT Owned by AR

AR **MUST NOT** store:
- Raw payment card data (PAN, CVV, track data) — violates PCI DSS
- Bank account numbers, routing numbers, IBAN — PCI/PII violation
- Payment processor API keys or secrets — security violation
- Subscription renewal logic state (next bill date calculations, proration rules) — owned by Subscriptions
- Customer communication preferences, marketing consent, CRM notes — owned by CRM/Notifications

---

## 4. OpenAPI Surface

### 4.1 Customer Endpoints

**POST /api/ar/customers** — Create a new billing customer
**GET /api/ar/customers/:id** — Retrieve customer details and AR balance
**PUT /api/ar/customers/:id** — Update customer metadata (email, name, address)
**GET /api/ar/customers** — List customers (Query: `tenant_id` required, `email`, `delinquent_only`, `limit`, `offset`)

### 4.2 Invoice Endpoints

**POST /api/ar/invoices** — Create invoice in draft state
**POST /api/ar/invoices/:id/issue** — Finalize invoice (draft → issued), emit GL posting event
**POST /api/ar/invoices/:id/void** — Void an issued invoice
**GET /api/ar/invoices/:id** — Retrieve invoice details including line items
**GET /api/ar/invoices** — List invoices (Query: `tenant_id` required, `customer_id`, `status`, `issued_after`, `issued_before`, `limit`, `offset`)

### 4.3 Payment Application Endpoints (Internal Use Only)

**POST /api/ar/invoices/:id/apply-payment** (Internal) — Record payment application (called by AR event handler after receiving `payments.payment.succeeded`)

### 4.4 Credit & Adjustment Endpoints

**POST /api/ar/credit-memos** — Issue credit memo to customer
**POST /api/ar/adjustments** — Manual balance adjustment (write-off, late fee, dispute adjustment)

### 4.5 Reporting Endpoints

**GET /api/ar/reports/aging-summary** — Aggregate aging report by customer
**GET /api/ar/reports/open-invoices** — List of unpaid invoices
**GET /api/ar/reports/delinquent-customers** — Customers with >30 days overdue balance

### 4.6 Payment Method Reference Endpoints

**POST /api/ar/payment-methods** — Store payment method reference (after Payments module tokenizes)
**GET /api/ar/payment-methods** — List payment methods (Query: `tenant_id` required, `customer_id`)
**DELETE /api/ar/payment-methods/:id** — Soft-delete payment method reference (sets `deleted_at`)

---

## 5. Events Produced & Consumed

All events follow the envelope standard from `contracts/events/README.md` with required fields: `event_id`, `occurred_at`, `tenant_id`, `source_module`, `source_version`, and optional `correlation_id`, `causation_id`.

### 5.1 Events Produced by AR

| Event Name | Purpose | Side Effects |
|------------|---------|--------------|
| **ar.invoice.created** | Invoice created in draft state | None |
| **ar.invoice.issued** | Invoice finalized (draft → issued) | Triggers `gl.posting.requested` for AR receivable debit |
| **ar.invoice.voided** | Invoice voided | Triggers GL reversal posting |
| **ar.payment.collection.requested** | **COMMAND EVENT** — AR requests Payments module to collect payment (Option A) | Payments module executes payment |
| **ar.payment.applied** | Payment successfully applied to invoice | Triggers `gl.posting.requested` for cash receipt |
| **ar.payment.failed_to_apply** | Payment application rejected (invoice not found, amount mismatch) | None |
| **ar.credit.issued** | Credit memo issued | Triggers `gl.posting.requested` |
| **ar.adjustment.created** | Manual adjustment recorded (write-off, late fee, dispute adjustment) | Triggers `gl.posting.requested` |
| **ar.dispute.opened** | Dispute recorded in AR (mirrored from Payments) | None |
| **ar.dispute.updated** | Dispute status changed | None |
| **ar.dispute.closed** | Dispute resolved | If lost, triggers GL adjustment posting |
| **gl.posting.requested** | Request GL to post journal entry (for invoice issued, payment applied, credit, write-off, refund, dispute) | GL module creates journal entry |

### 5.2 Events Consumed by AR

| Event Name | Source | Purpose | AR Behavior |
|------------|--------|---------|-------------|
| **payments.payment.succeeded** | Payments | Apply payment to invoice(s) in AR ledger | 1. Validate invoice exists<br>2. Apply payment<br>3. Update invoice status<br>4. Update customer AR balance<br>5. Emit `ar.payment.applied`<br>6. Emit `gl.posting.requested` |
| **payments.payment.failed** | Payments | Record payment failure, schedule retry if applicable | 1. Record failure<br>2. Increment `retry_attempt_count`<br>3. Calculate `next_retry_at`<br>4. Emit `ar.payment.collection.requested` if retry limit not exceeded |
| **payments.refund.succeeded** | Payments | Record refund in AR ledger, adjust customer balance | 1. Create refund record<br>2. Update customer AR balance<br>3. Emit `gl.posting.requested` |
| **payments.refund.failed** | Payments | Record refund failure | Record failure for audit |
| **payments.dispute.created** | Payments | Create AR dispute record, freeze invoice | 1. Create `ar_dispute_records` entry<br>2. Update invoice status to `disputed`<br>3. Emit `ar.dispute.opened` |
| **payments.dispute.updated** | Payments | Sync dispute status to AR | Update `ar_dispute_records` |
| **payments.dispute.closed** | Payments | Resolve dispute, trigger GL adjustment if lost | 1. Update `ar_dispute_records`<br>2. If outcome = lost, emit `gl.posting.requested` for dispute loss |
| **gl.posting.accepted** | GL | Confirm GL posting succeeded | 1. Update `ar_gl_posting_queue` status to `accepted`<br>2. Mark source record as `gl_posted: true` |
| **gl.posting.rejected** | GL | Report GL posting failure | 1. Update `ar_gl_posting_queue` status to `rejected`<br>2. Log error<br>3. DO NOT roll back AR financial truth<br>4. Surface in reconciliation dashboard |

**Idempotency:** All event handlers check `ar_ledger_events.event_id` before processing to prevent duplicate state changes.

---

## 6. State Machines

### 6.1 Invoice Lifecycle

```
draft ──┬──> issued ──┬──> partially_paid ──> paid
        │             │
        └──> voided   └──> disputed ──┬──> paid (if dispute won)
                                      └──> written_off (if dispute lost)
```

**Forbidden Transitions:**
- `paid` → any other state (immutable once paid)
- `voided` → any other state (immutable once voided)
- `written_off` → any other state (immutable once written off)

### 6.2 Payment Application Lifecycle

```
pending_apply ──┬──> applied
                └──> rejected
```

**Rejection Reasons (Enum):**
- `INVOICE_NOT_FOUND` — Invoice ID does not exist
- `INVOICE_VOIDED` — Invoice was voided before payment applied
- `INVOICE_PAID` — Invoice already fully paid
- `AMOUNT_MISMATCH` — Payment amount exceeds outstanding invoice balance
- `CURRENCY_MISMATCH` — Payment currency differs from invoice currency

### 6.3 Dispute Lifecycle

```
opened ──┬──> evidence_submitted ──> closed (won/lost/accepted)
         │
         └──> expired (no evidence submitted)
```

---

## 7. GL Posting Integration

### 7.1 Journal Intent Model

AR emits `gl.posting.requested` events using the schema defined in `contracts/events/gl-posting-request.v1.json`.

**Key Fields:** `posting_date`, `currency`, `source_doc_type`, `source_doc_id`, `description`, `lines[]`

**Balance Rule:** GL module MUST validate that `sum(debits) == sum(credits)`. If unbalanced, GL emits `gl.posting.rejected` with reason `UNBALANCED_ENTRY`.

### 7.2 GL Posting Triggers

| AR Event | Source Doc Type | Debit Account | Credit Account | Notes |
|----------|----------------|---------------|----------------|-------|
| Invoice Issued | `AR_INVOICE` | 1200 (AR) | 4000 (Revenue) | Creates receivable |
| Payment Applied | `AR_PAYMENT` | 1000 (Cash) | 1200 (AR) | Clears receivable |
| Credit Issued | `AR_CREDIT_MEMO` | 4100 (Sales Returns) | 1200 (AR) | Reduces receivable |
| Write-Off | `AR_ADJUSTMENT` | 5200 (Bad Debt) | 1200 (AR) | Removes uncollectible balance |
| Refund Recorded | `AR_ADJUSTMENT` | 4100 (Sales Returns) | 1000 (Cash) | Reverses revenue & cash |
| Dispute Lost | `AR_ADJUSTMENT` | 5300 (Dispute Loss) | 1200 (AR) | Chargeback expense |

**Account Codes (Example — Actual codes owned by GL module):**
- `1000` — Cash
- `1200` — Accounts Receivable
- `4000` — Service Revenue
- `4100` — Sales Returns & Allowances
- `5200` — Bad Debt Expense
- `5300` — Dispute Loss Expense

### 7.3 GL Rejection Handling

**If GL emits `gl.posting.rejected`:**

1. **DO NOT** roll back AR financial truth silently (invoice remains `issued`, payment remains `applied`)
2. **Record** rejection in `ar_gl_posting_queue`
3. **Surface** in reconciliation dashboard (`/api/ar/reports/gl-reconciliation-queue`)
4. **Alerting:** Critical GL rejections should trigger alerts to finance team

**Common Rejection Reasons:**
- `UNBALANCED_ENTRY` — Sum of debits != sum of credits (AR bug)
- `INVALID_ACCOUNT` — Account code not found in GL chart of accounts (configuration error)
- `PERIOD_CLOSED` — Posting date is in a closed accounting period (timing issue)
- `INVALID_CURRENCY` — Currency not supported by GL module

---

## 8. Security & Compliance

### 8.1 PCI DSS Boundaries

**FORBIDDEN in AR module:**
- ❌ Raw payment card data (PAN, CVV, track data)
- ❌ Bank account numbers, routing numbers, IBAN
- ❌ Payment processor API keys, secrets, webhooks signing keys
- ❌ Any data classified as PCI DSS cardholder data (CHD) or sensitive authentication data (SAD)

**ALLOWED in AR module:**
- ✅ Payment method references (opaque IDs like `pm_tilled_abc123`)
- ✅ Non-sensitive metadata: last4, brand, expiry, card type
- ✅ Customer name, email, billing address (not considered PCI data)
- ✅ Transaction amounts, invoice numbers, payment status

**Rationale:** AR module operates in PCI DSS scope Level 4 (lowest) by never touching cardholder data.

### 8.2 PII Handling

**PII Stored in AR:** Customer name, email, billing address (required for invoicing)

**Data Retention Policy:**
- Customer records must be retained for 7 years after last transaction (IRS requirement for financial records)
- Invoices and payment records must be retained for 7 years
- Soft-delete pattern: set `deleted_at`, do not physically delete for audit trail

**GDPR Compliance:**
- Right to erasure: Redact PII fields (name, email, address) after retention period
- Right to access: Provide customer data export endpoint (future scope)

### 8.3 Tenant Isolation

**Multi-Tenancy Model:** Shared database, row-level isolation via `tenant_id`

**Enforcement Mechanisms:**
1. **Database:** Every table includes `tenant_id` (non-nullable)
2. **Indexes:** All indexes must include `tenant_id` as first column for query efficiency
3. **Application:** All queries MUST filter by `tenant_id`
4. **Middleware:** Request context extracts `tenant_id` from JWT or service-to-service auth token
5. **CI Check:** Lint rule to detect queries without `WHERE tenant_id = ?` clause

**Forbidden:**
- Cross-tenant queries without explicit admin permission
- Storing `tenant_id` in JWT without signature verification
- Using `tenant_id = NULL` as a "global" record

---

## 9. Error Taxonomy & Retry Rules

### 9.1 Error Categories

| Category | HTTP Status | Retry Safe? | Examples |
|----------|-------------|-------------|----------|
| **Client Errors** | 400-499 | No | Invalid request, invoice not found, unauthorized |
| **Server Errors** | 500-599 | Yes (with backoff) | Database timeout, service unavailable |
| **Transient Failures** | 502, 503, 504 | Yes (aggressive) | Gateway timeout, service temporarily unavailable |
| **Business Rule Violations** | 422 | No | Cannot void paid invoice, insufficient balance |

### 9.2 Payment Collection Retries

When `payments.payment.failed` is received:

1. **Categorize Failure:**
   - **Hard Decline:** `card_declined`, `insufficient_funds`, `do_not_honor` → Retry per schedule
   - **Soft Decline:** `issuer_unavailable`, `processing_error` → Retry immediately, then per schedule
   - **Terminal Failure:** `card_expired`, `invalid_card`, `fraudulent` → Do not retry, notify customer

2. **Retry Schedule (Hard/Soft Declines):**
   - Attempt 1: Immediate (at invoice issue)
   - Attempt 2: +1 day
   - Attempt 3: +3 days (total 4 days)
   - Attempt 4: +7 days (total 11 days)
   - Attempt 5: +7 days (total 18 days)
   - **Max Attempts:** 5

3. **Customer Status Updates:**
   - After 3 failed attempts: Mark customer as `delinquent`, calculate `grace_period_end`
   - After `grace_period_end`: Mark customer as `suspended` (emit event for downstream services)

### 9.3 GL Posting Retries

**Transient GL Failures (503, 504):**
- Retry immediately once, then after 5 minutes, then alert

**Business Rule Failures (`gl.posting.rejected`):**
- Do **NOT** retry automatically
- Surface in reconciliation dashboard

**Idempotency:** GL module deduplicates on `event_id`, so retries are safe

---

## 10. Required Invariants

### 10.1 Financial Invariants

1. **Customer Balance Accuracy:**
   ```
   customer.ar_balance_cents = SUM(invoices.total_cents WHERE status IN ('issued', 'partially_paid'))
                               - SUM(payment_applications.allocated_cents)
                               - SUM(credit_memos.amount_cents WHERE applied_at IS NOT NULL)
                               + SUM(adjustments.amount_cents WHERE amount_cents > 0)
                               - SUM(adjustments.amount_cents WHERE amount_cents < 0)
   ```

2. **Invoice Total Accuracy:**
   ```
   invoice.total_cents = invoice.subtotal_cents + invoice.tax_cents
   invoice.subtotal_cents = SUM(line_items.amount_cents)
   ```

3. **Aging Bucket Consistency:**
   ```
   customer.aging_current + customer.aging_30 + customer.aging_60 + customer.aging_90_plus = customer.ar_balance_cents
   ```

4. **Payment Allocation:**
   ```
   SUM(payment_applications.allocated_cents WHERE invoice_id = X) <= invoice.total_cents
   ```

5. **Invoice Status Consistency:**
   ```
   IF SUM(payment_applications.allocated_cents WHERE invoice_id = X) = invoice.total_cents
   THEN invoice.status = 'paid'
   ```

### 10.2 Data Integrity Invariants

6. **Tenant Isolation:**
   ```
   For any query involving multiple tables:
   invoice.tenant_id = customer.tenant_id = payment_application.tenant_id
   ```

7. **No PCI Data:**
   ```
   No table in AR module contains columns matching patterns:
   - *card_number*, *cvv*, *track*, *pan*
   - *bank_account*, *routing_number*, *iban*
   ```

8. **Monetary Precision:**
   ```
   All *_cents columns are integers (no floating point)
   All monetary calculations use integer arithmetic
   ```

9. **Event Idempotency:**
   ```
   ar_ledger_events.event_id is unique (enforced by DB constraint)
   ```

10. **GL Posting Completeness:**
    ```
    For every invoice with status = 'issued':
    EXISTS(ar_gl_posting_queue WHERE source_type = 'invoice' AND source_id = invoice.id)
    ```

### 10.3 State Machine Invariants

11. **No Backward Transitions:**
    ```
    Invoice status cannot go from 'paid' to 'issued'
    Invoice status cannot go from 'voided' to 'draft'
    ```

12. **Payment Method Validity:**
    ```
    IF customer.default_payment_method_id IS NOT NULL
    THEN EXISTS(ar_payment_method_refs WHERE id = customer.default_payment_method_id AND deleted_at IS NULL)
    ```

### 10.4 Enforcement Strategy

**How to Enforce Invariants:**
1. **Database Constraints:** Foreign keys, check constraints, unique indexes
2. **Application Logic:** Pre-condition checks before state transitions
3. **Background Jobs:** Nightly reconciliation jobs that verify invariants and alert on violations
4. **Integration Tests:** Test suite includes invariant checks after each operation
5. **Audit Queries:** SQL queries to detect violations (run weekly)

---

## 11. Testing Strategy

### 11.1 Unit Tests
**Scope:** Pure functions, business logic, state machine transitions
**Tools:** Rust `#[cfg(test)]` modules, `cargo test`
**Coverage Target:** 80% line coverage for core business logic

### 11.2 Integration Tests
**Scope:** Database interactions, event emission, idempotency
**Tools:** `sqlx::test`, in-memory Postgres, `testcontainers`

### 11.3 Contract Tests
**Scope:** Event schema validation, OpenAPI contract compliance
**Tools:** JSON Schema validator, OpenAPI contract tester

### 11.4 End-to-End Workflow Tests
**Scope:** Multi-module flows, happy path + failure scenarios
**Examples:**
1. Invoice → Payment → GL (happy path)
2. Payment declined → retry (failure path)
3. GL rejection → reconciliation queue (failure path)

### 11.5 Invariant Tests
**Scope:** Verify financial invariants hold after operations
**Frequency:** Run after every state-changing operation in integration tests

---

## 12. Versioning & Breaking Changes

### 12.1 Module Versioning
**AR Module Version:** Semantic versioning (`MAJOR.MINOR.PATCH`)
**Current Version:** `0.1.0` (pre-release, contracts may change freely until `1.0.0`)

### 12.2 Contract Breaking Changes
**Breaking:** Removing event field, changing field type, renaming event, removing endpoint
**Non-Breaking:** Adding optional event field, adding new event type, adding new endpoint

### 12.3 Event Versioning
**Approach:** Filename versioning (`ar-invoice-issued.v1.json`, `v2.json`)
**Deprecation:** 3-month notice, 6-month support overlap

### 12.4 OpenAPI Versioning
**Approach:** URL path versioning (`/api/ar/v1/invoices`, `/api/ar/v2/invoices`)
**Deprecation:** 6-month support overlap

---

## 13. Explicit "Out of Scope"

### 13.1 Subscriptions Module
- Subscription lifecycle, billing schedule generation, proration, trials
- **Integration:** Subscriptions emits `subscriptions.invoice.requested`; AR creates invoice

### 13.2 Payments Module
- Processor integration, tokenization, payment execution, refunds, disputes
- **Integration:** AR emits `ar.payment.collection.requested`; Payments executes

### 13.3 Notifications Module
- Invoice emails, payment receipts, dunning emails, SMS
- **Integration:** Notifications subscribes to AR events

### 13.4 GL Module
- Chart of accounts, journal entries, period close, financial statements
- **Integration:** AR emits `gl.posting.requested`; GL validates and posts

### 13.5 CRM/Customer Module
- Full customer profile, marketing consent, support tickets
- **Integration:** CRM syncs billing data to AR

### 13.6 Reporting/Analytics Module
- Custom reports, dashboards, forecasting
- **Integration:** Analytics reads AR data via replica

---

## 14. Implementation Notes (Additive Only)

### 14.1 Suggested Folder Layout

```
modules/ar/
├── Cargo.toml
├── migrations/
├── src/
│   ├── main.rs
│   ├── api/           # REST endpoints
│   ├── domain/        # Business logic, state machines
│   ├── events/        # Event producers/consumers
│   ├── gl/            # GL posting logic
│   ├── models/        # Data models
│   └── db/            # DB queries
├── tests/
│   ├── integration/
│   ├── contracts/
│   └── e2e/
└── README.md
```

### 14.2 Migration Strategy
**Tool:** `sqlx migrate`
**Rules:** Never modify committed migrations, include rollback comments, test before commit

### 14.3 CI Enforcement Checks (Proposed)
- Contract validation (events match schemas)
- Cross-module import lint (no `use payments::`, `use gl::`)
- Tenant isolation lint (queries have `WHERE tenant_id`)
- Migration compatibility
- OpenAPI spec validation
- PCI data scan (forbidden column names)

### 14.4 Event Bus Integration
**Technology:** NATS or RabbitMQ
**Idempotency:** Check `ar_ledger_events.event_id` before processing

### 14.5 Monitoring & Observability
**Metrics:** invoices_issued, payments_applied, payment_failures, gl_rejections, customer_balance, delinquent_count
**Logs:** Invoice issued, payment applied/failed, GL rejection
**Traces:** OpenTelemetry with `correlation_id`

### 14.6 Reconciliation Jobs
**Daily (2 AM UTC):** Verify invariants, alert on violations
**Every 15 min:** Retry pending GL postings, alert if stuck >1 hour

---

## 15. TrashTech-Specific Requirements

### 15.1 Recurring Service Billing
Subscriptions generates invoice commands; AR creates with service period line items

### 15.2 Service Period Metadata
Line items include `service_period_start` and `service_period_end`

### 15.3 Partial Payments
Invoice status → `partially_paid`, track remaining balance

### 15.4 Late Fees
Daily job checks overdue invoices (15 days after due date), creates adjustment

### 15.5 Delinquency Reporting
`GET /api/ar/reports/delinquent-customers` — customers >30 days overdue

### 15.6 Aging Buckets
Track in 30-day buckets (current, 30, 60, 90+), recalculated nightly based on `due_at` date

---

## 16. Summary & Next Steps

### 16.1 What This Spec Defines
✅ AR domain authority (customers, invoices, payment applications, credits, adjustments, disputes)
✅ Complete OpenAPI surface (customers, invoices, credits, reports, payment method refs)
✅ Event contracts (produced: 12 events, consumed: 8 events)
✅ State machines (invoice, payment application, dispute)
✅ GL posting integration model (journal intent, triggers, rejection handling)
✅ Security boundaries (PCI, PII, tenant isolation)
✅ Error taxonomy & retry policies (payment retries, GL retries)
✅ Financial invariants (12 invariants + enforcement strategy)
✅ Testing strategy (unit, integration, contract, e2e, invariant tests)
✅ Versioning rules (module, event, OpenAPI)
✅ Explicit out-of-scope (Subscriptions, Payments, Notifications, GL, CRM)
✅ Implementation recommendations (folder structure, CI checks, monitoring)
✅ TrashTech-specific requirements (recurring billing, service periods, late fees, aging)

### 16.2 What Is NOT Defined (Future Work)
- Concrete database schema (only table names + key fields specified)
- Exact GL account codes (example codes provided, actual codes owned by GL)
- Event bus technology selection (NATS vs RabbitMQ)
- Observability tooling (Prometheus, Grafana, OpenTelemetry)
- Multi-currency exchange rate handling
- Tax calculation logic
- Advanced reporting (forecasting, cohort analysis)

### 16.3 Next Steps for Implementation
1. **Review & Approval:** Stakeholders review this spec, propose changes
2. **Contracts First:** Finalize event schemas in `contracts/events/ar-*.v1.json`
3. **OpenAPI Update:** Expand `contracts/ar/ar-v1.yaml` with missing endpoints
4. **Database Schema:** Create migrations for new tables
5. **Event Bus Integration:** Implement event producers/consumers
6. **GL Integration:** Implement `gl.posting.requested` emission for all triggers
7. **Contract Tests:** Add tests validating event schemas + OpenAPI responses
8. **Invariant Tests:** Add nightly reconciliation job checking all 12 invariants
9. **CI Enforcement:** Add lint rules for tenant isolation, cross-module imports, PCI data scan

---

**End of AR Module Specification v0.1.x**
