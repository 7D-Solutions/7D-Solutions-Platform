# Event Taxonomy Standard

## Purpose
Define naming conventions and semantics for events across the platform.
This standard complements the event envelope spec in `contracts/events/README.md`.

---

## Event Name Format
**`{domain}.{entity}.{action}`**

Examples:
- `ar.invoice.issued`
- `ar.payment.collection.requested`
- `payments.payment.succeeded`
- `gl.posting.rejected`

Rules:
- lowercase
- dot-delimited
- use singular entity (`invoice`, `payment`, `posting`)
- action is a verb (prefer past tense for facts)

---

## Commands vs Facts
### Facts (domain events)
- Something **has happened** and is immutable.
- Use past tense where possible.
Examples:
- `ar.invoice.issued`
- `payments.payment.succeeded`
- `gl.posting.accepted`

### Commands (requests)
- A module is asking another module to do work.
- Use `.requested` suffix.
Example:
- `ar.payment.collection.requested`
- `gl.posting.requested`

**Rule:** commands do not imply success.

---

## Versioning
Event schema versioning is done via the schema filename and/or embedded `schema_version`.
- Schema files should include `.v{N}` in filename (v1, v2, …).
- Breaking changes require new major schema version:
  - `...v2.json` and event producers emit `schema_version: 2`.

Additive changes (new optional fields) may remain same major version.

---

## Required Metadata (Envelope)
Follow `contracts/events/README.md`.
At minimum:
- event_id
- occurred_at
- tenant_id
- source_module
- source_version
- correlation_id
- causation_id
- payload

---

## Idempotency
Consumers must be idempotent:
- De-dup by event_id
- Reject/ignore invalid or forbidden state transitions

---

## Ownership Rule
An event's domain prefix indicates the owning module:
- `ar.*` events: owned by AR
- `payments.*` events: owned by Payments
- `subscriptions.*` events: owned by Subscriptions
- `notifications.*`: owned by Notifications
- `gl.*`: owned by GL

No other module may emit events under another module's domain prefix.

---

## Anti-Patterns (Forbidden)
- Emitting `billing.*` (billing is composed, not a primitive)
- Ambiguous names like `payment.completed`
- Overloaded events that mix multiple domains in one payload
- Events that require consumers to call back synchronously to "complete" meaning
