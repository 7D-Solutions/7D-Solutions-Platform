# Module Authority Matrix — 7D Solutions Platform

## Purpose
This document defines **domain authority**, **state ownership**, and **allowed mutations** per module.
It is the single source of truth for: "Who owns what?" and "Who is allowed to change what?"

**Non-negotiable rules**
- Modules communicate only via **OpenAPI contracts** and **event contracts**.
- No cross-module DB access (read or write).
- No cross-module source imports.
- Modules are **plug-and-play** and **independently versioned**.
- **Option A locked:** AR drives payment collection via event command; Payments executes; AR applies results.
- **GL posting is event-driven only:** modules emit `gl.posting.requested`; only GL writes journal entries.

---

## Legend
- **Owns**: definitive source of truth (DB + invariants + lifecycle)
- **May mutate**: allowed to change this state (by definition of ownership)
- **Produces**: events this module emits as facts/commands
- **Consumes**: events this module ingests (must be idempotent)

---

## Authority Table (Current + Planned)

### Platform (Tier 1)

| Module | Owns | May mutate | Produces | Consumes |
|---|---|---|---|---|
| `platform/identity-auth` | tenants, users, roles, permissions, auth sessions | yes | `auth.*` (as defined by auth contracts) | none |

### Modules (Tier 2)

| Module | Owns | May mutate | Produces | Consumes |
|---|---|---|---|---|
| `modules/ar` | customers (billing context only), invoices, invoice lines, AR ledger state, allocations/payment applications, credits/adjustments, AR disputes (financial artifacts), AR reporting views | yes | `ar.invoice.*`, `ar.payment.collection.requested` (command), `ar.payment.applied`, `ar.adjustment.*`, `ar.dispute.*`, `gl.posting.requested` | `payments.payment.*`, `payments.refund.*`, `payments.dispute.*`, `gl.posting.accepted`, `gl.posting.rejected` |
| `modules/subscriptions` (planned) | subscriptions/service agreements, schedules, proration policy flags, bill-run state, plan templates | yes | `subscriptions.*` (facts) + **OpenAPI command to AR** to create/issue invoice | none |
| `modules/payments` (planned) | processor integrations, payment intents, payment captures, refunds execution state, webhook ingestion + verification, customer/payment method references (no secrets) | yes | `payments.payment.succeeded|failed`, `payments.refund.succeeded|failed`, `payments.dispute.*` | `ar.payment.collection.requested` (command) |
| `modules/notifications` (planned) | notification preferences, templates, outbox, delivery attempts, provider routing | yes | `notifications.delivery.succeeded|failed` (optional) | `ar.invoice.*`, `ar.payment.*`, `payments.payment.*`, `payments.dispute.*` |

### External / Future

| Module | Owns | May mutate | Produces | Consumes |
|---|---|---|---|---|
| `modules/gl` (future) | chart of accounts, journal entries, posting rules | yes | `gl.posting.accepted`, `gl.posting.rejected` | `gl.posting.requested` |

---

## Hard Boundary Rules

### AR (Accounts Receivable)
AR is the **financial authority** for invoices and receivables:
- Only AR may change invoice state (draft/issued/paid/etc.)
- Payments may never "mark invoice paid" directly
- AR stores **payment method references only** (opaque ids), no secrets/PCI

### Subscriptions
Subscriptions owns scheduling only:
- Subscriptions never stores invoice truth
- Subscriptions creates invoices by calling AR OpenAPI (contract-driven)

### Payments
Payments owns processor truth only:
- Payments never mutates AR state
- Payments emits results (`payments.payment.*`) and AR applies them
- Payments owns webhook verification and idempotency for processor events

### Notifications
Notifications is delivery only:
- No financial decisions
- No coupling to internal DB of other modules
- Reacts to facts and sends messages

---

## Required Invariants (Boundary-Level)
1. No module writes to another module's tables.
2. No module imports another module's source code.
3. All cross-module coordination is by **contract** (OpenAPI/events).
4. `gl.posting.requested` is the only way to request GL changes.
5. Payment secrets never enter AR.
6. Every event consumer is idempotent (event_id uniqueness).
7. Tenant isolation is mandatory: every record and event carries tenant_id.

---

## Notes on "Billing"
"Billing" is a **composed capability**, not a module:
- TrashTech billing = Subscriptions + AR + Payments + Notifications (composed at product layer)
