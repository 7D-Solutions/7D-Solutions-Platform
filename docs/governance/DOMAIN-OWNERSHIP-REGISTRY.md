# Domain Ownership Registry

**Version**: 1.0
**Last Updated**: 2026-02-16
**Phase**: 16 - Event Envelope Hardening & Production Readiness

## Purpose

This registry declares the single source of truth for each domain concept in the platform. Each domain concept has **exactly one owning module**. This prevents overlapping ownership, cross-module coupling, and inconsistent truth.

## Governance Principles

1. **Single Writer**: Only the owning module may write to its domain tables
2. **Read via API/Events**: Cross-module data access must use events or HTTP APIs
3. **No Cross-Module JOINs**: Database queries must not join tables across module boundaries
4. **Eventual Consistency**: Inter-module communication is eventually consistent via event bus
5. **Idempotent Consumers**: All event consumers must be idempotent (track processed_events)

---

## Module Domain Ownership

### AR (Accounts Receivable)

**Module Path**: `modules/ar/`
**Database**: `postgres://localhost:5432/ar_db`
**Port**: `8086`

**Domain Ownership**:
- **Customers** (`ar_customers`) - Customer master data, payment methods, delinquency status
- **Invoices** (`ar_invoices`) - Invoice lifecycle, line items, payment status
- **Invoice Line Items** (`ar_invoice_line_items`) - Detailed billing items
- **Webhook Logs** (`webhook_logs`) - Outbound webhook delivery tracking

**Domain Responsibilities**:
- Customer CRUD operations
- Invoice generation (from subscriptions or direct API)
- Invoice payment status management (from payment events)
- Revenue recognition coordination
- Webhook delivery to external systems

**External Dependencies**:
- Consumes: `subscription.bill_run.completed` (from Subscriptions)
- Consumes: `payments.payment.succeeded`, `payments.payment.failed` (from Payments)
- Produces: `ar.payment.collection.requested` (to Payments)
- Produces: `gl.posting.requested` (to GL)

---

### GL (General Ledger)

**Module Path**: `modules/gl/`
**Database**: `postgres://localhost:5432/gl_db`
**Port**: `8087`

**Domain Ownership**:
- **Chart of Accounts** (`accounts`) - Account structure, types, classifications
- **Journal Entries** (`journal_entries`) - Double-entry transaction records
- **Journal Entry Lines** (`journal_entry_lines`) - Debit/credit line items
- **Account Balances** (`account_balances`) - Period-based balance snapshots
- **Periods** (`periods`) - Accounting period definitions and close status

**Domain Responsibilities**:
- Maintain chart of accounts structure
- Record journal entries from posting requests
- Calculate and store account balances
- Period close/open management
- Financial statement generation (trial balance, P&L, balance sheet)

**External Dependencies**:
- Consumes: `gl.posting.requested` (from AR, Payments, or other modules)
- Consumes: `gl.reversal.requested` (from AR or other modules)
- Produces: None (terminal node in financial flow)

---

### Payments

**Module Path**: `modules/payments/`
**Database**: `postgres://localhost:5432/payments_db`
**Port**: `8088`

**Domain Ownership**:
- **Payment Attempts** (`payment_attempts`) - Payment transaction attempts, statuses, reconciliation
- **Payment Configurations** - Processor configuration, retry windows

**Domain Responsibilities**:
- Payment collection processing
- Payment status lifecycle (attempting → succeeded/failed/unknown)
- UNKNOWN reconciliation (timeout handling)
- Retry scheduling and gating
- Payment method processing (delegated to Tilled/Stripe)

**External Dependencies**:
- Consumes: `ar.payment.collection.requested` (from AR)
- Produces: `payments.payment.succeeded`, `payments.payment.failed` (to AR)
- External: Tilled payment processor API

---

### Subscriptions

**Module Path**: `modules/subscriptions/`
**Database**: `postgres://localhost:5432/subscriptions_db`
**Port**: `8085`

**Domain Ownership**:
- **Subscription Plans** (`subscription_plans`) - Plan definitions, pricing, schedules
- **Subscriptions** (`subscriptions`) - Customer subscription instances
- **Subscription Invoice Attempts** (`subscription_invoice_attempts`) - Billing cycle tracking

**Domain Responsibilities**:
- Subscription lifecycle (active, paused, cancelled)
- Billing cycle management (next_bill_date advancement)
- Bill run execution (trigger invoice generation)
- Cycle gating (prevent duplicate invoices per cycle)

**External Dependencies**:
- Produces: `subscription.bill_run.completed` (to AR)
- Foreign Key Reference: `ar_customer_id` (string, not enforced at DB level)

---

### Notifications

**Module Path**: `modules/notifications/`
**Database**: `postgres://localhost:5432/notifications_db`
**Port**: `8089`

**Domain Ownership**:
- **Notification Logs** (`notification_logs`) - Delivery tracking, templates, recipient info

**Domain Responsibilities**:
- Email/SMS notification delivery
- Notification template management
- Delivery status tracking

**External Dependencies**:
- Consumes: `invoice.issued`, `payment.succeeded`, `payment.failed` (from AR/Payments)
- Produces: `notifications.delivery.succeeded`, `notifications.delivery.failed`
- External: Email/SMS service providers

---

## Inter-Module Command Registry

This section declares all cross-module commands (events) and their write degradation characteristics.

### Event: `subscription.bill_run.completed`

**Producer**: Subscriptions
**Consumer**: AR
**Trigger**: Monthly/annual billing cycle execution
**Payload**: Subscription ID, customer ID, billing period, amount
**Degradation Class**: **Critical**

**Failure Mode**: If AR cannot process this event, customer invoices are not generated, blocking revenue recognition.
**Recovery**: Replay from outbox, manual invoice creation via AR API.

---

### Event: `ar.payment.collection.requested`

**Producer**: AR
**Consumer**: Payments
**Trigger**: Invoice creation or retry scheduler
**Payload**: Invoice ID, customer ID, amount, due date
**Degradation Class**: **Critical**

**Failure Mode**: If Payments cannot process this event, payment collection is delayed, impacting cash flow.
**Recovery**: Replay from outbox, manual payment initiation.

---

### Event: `payments.payment.succeeded`

**Producer**: Payments
**Consumer**: AR
**Trigger**: Successful payment processing or UNKNOWN reconciliation
**Payload**: Payment ID, invoice ID, amount, transaction ID
**Degradation Class**: **Critical**

**Failure Mode**: If AR cannot process this event, invoice status remains 'open', customer may be incorrectly charged again.
**Recovery**: Replay from outbox, manual invoice status update.

---

### Event: `payments.payment.failed`

**Producer**: Payments
**Consumer**: AR
**Trigger**: Payment decline or final retry exhaustion
**Payload**: Payment ID, invoice ID, failure reason
**Degradation Class**: **High**

**Failure Mode**: If AR cannot process this event, customer delinquency status is not updated, dunning process may fail.
**Recovery**: Replay from outbox, manual status reconciliation.

---

### Event: `gl.posting.requested`

**Producer**: AR (or other modules)
**Consumer**: GL
**Trigger**: Invoice payment, manual journal entry
**Payload**: Journal entry lines (account, debit/credit, amount), idempotency key
**Degradation Class**: **Critical**

**Failure Mode**: If GL cannot process this event, financial records are incomplete, financial statements are inaccurate.
**Recovery**: Replay from outbox, manual journal entry via GL API.

---

### Event: `invoice.issued` / `payment.succeeded` / `payment.failed`

**Producer**: AR / Payments
**Consumer**: Notifications
**Trigger**: Invoice creation, payment status change
**Payload**: Recipient email, event type, metadata
**Degradation Class**: **Low** (Degraded Mode Acceptable)

**Failure Mode**: If Notifications cannot process this event, customer does not receive email, but business operations continue.
**Recovery**: Replay from outbox, notifications can be delayed without financial impact.

---

## Write Degradation Classification

**P0 - Critical**: System cannot function correctly without this write. Requires immediate recovery.
- GL journal entries
- Invoice status updates (payment succeeded/failed)
- Payment attempt records
- Subscription → Invoice mapping (prevents duplicate billing)

**P1 - High**: System can operate but with significant degradation. Requires recovery within hours.
- Customer delinquency status
- Retry scheduling updates
- Period close operations

**P2 - Degraded Mode Acceptable**: System operates normally, user experience slightly degraded. Can retry indefinitely.
- Notification delivery
- Webhook logs
- Audit trail writes

---

## Cross-Module Access Patterns

### ✅ Allowed Patterns

1. **Event-Driven Updates**: Module A emits event → Module B consumes via event bus
2. **HTTP API Calls**: Module A calls Module B's REST API (synchronous, for reads only)
3. **Foreign Key References (Logical)**: Store string IDs from other modules (e.g., `ar_customer_id` in subscriptions) without DB-level foreign key constraints

### ❌ Prohibited Patterns

1. **Cross-Module Database JOINs**: `SELECT * FROM ar.invoices JOIN payments.payment_attempts` ❌
2. **Direct Table Writes**: Module A writing to Module B's tables ❌
3. **Shared Database**: Multiple modules writing to the same database ❌
4. **Synchronous Blocking Calls**: Module A waiting for Module B's write before proceeding (use eventual consistency) ❌

---

## Enforcement Mechanisms

1. **Database Separation**: Each module has its own PostgreSQL database
2. **Code Review**: All PRs must respect domain ownership boundaries
3. **Integration Tests**: Cross-module tests verify event flows, not direct table access
4. **Linting**: Custom linters detect cross-schema SQL references
5. **Invariant Assertions**: `e2e-tests/tests/oracle.rs` validates module boundaries

---

## Domain Ownership Disputes

If a domain concept's ownership is unclear:

1. **Consult This Registry First**: Check if ownership is already declared
2. **Propose Amendment**: Open GitHub issue with proposal for ownership assignment
3. **Review Criteria**:
   - Which module is the natural source of truth?
   - Which module needs to enforce invariants?
   - Which module has the highest write frequency?
4. **Update This Registry**: After approval, update this document and increment version

---

## Version History

| Version | Date       | Author        | Changes                                      |
|---------|------------|---------------|----------------------------------------------|
| 1.0     | 2026-02-16 | ChartreuseFox | Initial domain ownership registry for Phase 16 |

---

## References

- [Phase 16 Architecture](../../MEMORY.md#current-status-2026-02-16)
- [EventEnvelope Constitutional Metadata](../../platform/event-bus/src/envelope.rs)
- [Cross-Module Invariant Oracle](../../e2e-tests/tests/oracle.rs)
- [Module-Level Invariants](../bd-35x/) (AR, Payments, Subscriptions, GL)
