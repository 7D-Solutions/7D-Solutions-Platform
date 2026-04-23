# MODE_OUTPUT_H2 — Adversarial Review

**Mode:** H2 — Adversarial Review  
**Analyst:** HazyMill  
**Date:** 2026-04-17  
**Scope:** `MODES_CONTEXT_PACK.md` + listed specs/context docs

---

## 1. Thesis

The spec set is strong on module boundaries, but adversarially it is weak at control points where business-state transitions are allowed without proof artifacts (book, cancel, return, verify), and where ownership/visibility semantics are left to handler discretion. At pre-launch stage this is not a "platform collapse" problem; it is a "first-customer trust and operational integrity" problem: a malicious or sloppy actor can create financially and operationally inconsistent states that remain contract-valid, are hard to detect in real time, and become painful to unwind later.

---

## 2. Top Findings

### §F1 — Sales-Orders Can Be Booked Without Any Payability Gate

**Evidence**
- `docs/architecture/SALES-ORDERS-MODULE-SPEC.md:21` (`credit checks` are out of scope)
- `docs/architecture/SALES-ORDERS-MODULE-SPEC.md:71` (`POST /orders/:id/book` books and triggers downstream work)
- `docs/architecture/SALES-ORDERS-MODULE-SPEC.md:168` (booking invariant only requires at least one line)

**Reasoning chain (adversarial)**
- Malicious-actor angle: a rep can book orders for a customer with known payment risk, triggering reservation/shipping work before any affordability control.
- Lazy-implementer angle: implementation will satisfy listed invariants and still ship this gap, because no stronger gate is specified.
- Stress angle: at 100x order volume, this becomes an expensive queue of orders that consume downstream capacity and only fail at invoicing/collection.

**Severity:** High  
**Confidence:** 0.95

**So What? (next-day action)**
Add a required `book_precheck` step in contract/state machine: either AR credit status, explicit override reason, or tenant policy `allow_unchecked_booking=false` by default.

---

### §F2 — Cancellation/Release Semantics Allow Shipment Stranding and Counter Drift

**Evidence**
- `docs/architecture/SALES-ORDERS-MODULE-SPEC.md:72` (`POST /orders/:id/cancel` with no state guard in surface)
- `docs/architecture/SALES-ORDERS-MODULE-SPEC.md:136-139` (diagram allows cancel path from non-terminal flow including shipped path)
- `docs/architecture/SALES-ORDERS-MODULE-SPEC.md:185` (invoice is requested on ship)
- `docs/architecture/SALES-ORDERS-MODULE-SPEC.md:90-93` (release create/cancel endpoints)
- `docs/architecture/SALES-ORDERS-MODULE-SPEC.md:58` (`blanket_order_lines` fields omit `cancelled_qty`)
- `docs/architecture/SALES-ORDERS-MODULE-SPEC.md:171-172` (invariant depends on `cancelled_qty` and trigger/app-level maintenance)

**Reasoning chain (adversarial)**
- Malicious-actor angle: cancel shipped-but-not-yet-invoiced orders to create "physically shipped, contractually cancelled" ambiguity.
- Lazy-implementer angle: `cancelled_qty` is referenced but not modeled in table shape; easy to implement inconsistent counters.
- Stress angle: concurrent release/cancel operations can overrun commitment math unless lock strategy is explicit.

**Severity:** High  
**Confidence:** 0.91

**So What? (next-day action)**
Specify hard transition guards: `shipped -> cancelled` forbidden unless explicit return/credit workflow. Define release accounting fields and transaction/locking rule (single-row lock on blanket line during release/cancel).

---

### §F3 — Complaint Transparency Can Be Suppressed While Still Satisfying Workflow Rules

**Evidence**
- `docs/architecture/CUSTOMER-COMPLAINTS-MODULE-SPEC.md:52` (`visible_to_customer` controls portal visibility)
- `docs/architecture/CUSTOMER-COMPLAINTS-MODULE-SPEC.md:79` (`customer-communication` endpoint)
- `docs/architecture/CUSTOMER-COMPLAINTS-MODULE-SPEC.md:136` (`investigating -> responded` only requires a customer_communication log)
- `docs/architecture/CUSTOMER-COMPLAINTS-MODULE-SPEC.md:158` / `:182` (portal shows only `visible_to_customer=true` entries)

**Reasoning chain (adversarial)**
- Malicious-actor angle: log customer communication as hidden/internal, satisfy transition invariant, close complaint with customer seeing little/no evidence.
- Hostile-vertical angle: pressure to default `visible_to_customer=false` for reputational shielding.
- Time-travel angle: incident review sees a formally valid state machine, but customer-facing audit trail is intentionally opaque.

**Severity:** Medium-High  
**Confidence:** 0.90

**So What? (next-day action)**
Add invariant: transitioning to `responded` requires at least one `customer_communication` entry with `visible_to_customer=true` (or explicit policy override + reason + audit event).

---

### §F4 — CRM Ownership Is Mutable Enough to Enable Opportunity Takeover

**Evidence**
- `docs/architecture/CRM-PIPELINE-MODULE-SPEC.md:56-57` (both leads and opportunities store `owner_id`)
- `docs/architecture/CRM-PIPELINE-MODULE-SPEC.md:96` (`PUT /opportunities/:id` updates non-stage fields)
- `docs/architecture/CRM-PIPELINE-MODULE-SPEC.md:192` (ownership scoping is conditional: "can be scoped" and handler-enforced)

**Reasoning chain (adversarial)**
- Malicious-actor angle: rep updates `owner_id` to claim a deal they should not control.
- Lazy-implementer angle: role checks may be implemented, ownership checks skipped, because spec does not make anti-takeover invariant explicit.
- Hostile-vertical angle: sales leadership may push permissive reassignment, creating policy drift from secure defaults.

**Severity:** High  
**Confidence:** 0.94

**So What? (next-day action)**
Define immutable-by-default ownership rules: `owner_id` change only through dedicated `reassign` endpoint with reason, actor, previous owner, and optional approval policy.

---

### §F5 — Outside-Processing Accepts "Returned" Material Without Strong Proof-of-Physical Flow

**Evidence**
- `docs/architecture/OUTSIDE-PROCESSING-MODULE-SPEC.md:81-82` (manual ship-event and return-event writes)
- `docs/architecture/OUTSIDE-PROCESSING-MODULE-SPEC.md:56` (`shipping_reference` is "FK-like"; not mandated)
- `docs/architecture/OUTSIDE-PROCESSING-MODULE-SPEC.md:146` (`at_vendor -> returned` on first return event)
- `docs/architecture/OUTSIDE-PROCESSING-MODULE-SPEC.md:173` (only quantity-bounded invariant)
- `docs/architecture/OUTSIDE-PROCESSING-MODULE-SPEC.md:123` (shipping module can create return-event stub, but not required path)

**Reasoning chain (adversarial)**
- Malicious-actor angle: create ship + return events administratively to mark round-trip complete when vendor never physically received/worked goods.
- Lazy-implementer angle: quantity checks pass, so system accepts forged lifecycle.
- Stress angle: high-volume partial shipments/returns amplify reconciliation debt.

**Severity:** High  
**Confidence:** 0.88

**So What? (next-day action)**
Require provenance on return events (`received_via_shipment_receiving_ref` or explicit manual override code path with stronger role + reason). Disallow `returned` transition from purely manual events without trace reference.

**Confirmed known (not new discovery):** `review_in_progress -> at_vendor` has no max cycle count (`docs/architecture/OUTSIDE-PROCESSING-MODULE-SPEC.md:210`).

---

### §F6 — Shop-Floor-Gates Allows Separation-of-Duties Bypass and Hold-as-DoS

**Evidence**
- `docs/architecture/SHOP-FLOOR-GATES-MODULE-SPEC.md:87-88` (confirm + verify API split)
- `docs/architecture/SHOP-FLOOR-GATES-MODULE-SPEC.md:177` (verify requires checks complete, but no distinct-person constraint)
- `docs/architecture/SHOP-FLOOR-GATES-MODULE-SPEC.md:208` (explicitly allows operator and verifier to be same person) 
- `docs/architecture/SHOP-FLOOR-GATES-MODULE-SPEC.md:53` (`release_authority` includes restrictive values like `owner_only`)
- `docs/architecture/SHOP-FLOOR-GATES-MODULE-SPEC.md:69-70` (any authorized actor can place hold; release depends on authority)
- `docs/architecture/SHOP-FLOOR-GATES-MODULE-SPEC.md:180` (active hold blocks operation start)

**Reasoning chain (adversarial)**
- Malicious-actor angle: single user with combined roles self-verifies and self-signs critical gates.
- DoS angle: place hold with restrictive release authority, block operation start, force escalation bottleneck.
- Lazy-implementer angle: because same-person verification is accepted in spec, this bypass becomes default behavior.

**Severity:** High  
**Confidence:** 0.93

**So What? (next-day action)**
Make two-person verification default policy with tenant opt-out behind explicit config + audit. Add hold TTL/escalation and emergency release protocol to limit hold-based denial-of-work.

**Confirmed known (not new discovery):** self-verify allowance is documented as open design choice (`:208`).

---

### §F7 — Hostile Vertical Vocabulary Capture Is Enabled in Core Workflow Fields

**Evidence**
- `docs/architecture/OUTSIDE-PROCESSING-MODULE-SPEC.md:63` (`service_type` intentionally open/free text)
- `docs/architecture/CRM-PIPELINE-MODULE-SPEC.md:68` (pipeline stages are tenant-defined, not canonical)
- `docs/architecture/CRM-PIPELINE-MODULE-SPEC.md:102-105` (tenants can add/update/reorder stages)

**Reasoning chain (adversarial)**
- Hostile-vertical angle: one vertical can pressure platform consumers/reporting to adopt its semantics because core codes are unconstrained.
- Lazy-implementer angle: downstream analytics/policy engines will silently hardcode tenant-specific values.
- Time-travel angle: 18 months later, cross-tenant reporting and automation become brittle because "same concept" has incompatible code vocabularies.

**Severity:** Medium  
**Confidence:** 0.86

**So What? (next-day action)**
Define a small canonical interoperability layer for cross-tenant analytics/events (e.g., normalized stage buckets and OP service class taxonomy) while preserving tenant-local labels/codes.

---

### §F8 — Forensic and Replay Semantics Are Too Weak for 2AM Incident Recovery

**Evidence**
- `contracts/events/README.md:46-47` (`correlation_id` and `causation_id` are optional)
- `platform/platform-sdk/README.md:89-91` (consumer retries max 3, then message is skipped)
- `contracts/events/README.md:27-33` (idempotency is event-level only)

**Reasoning chain (adversarial)**
- Time-travel angle: when state diverges, optional causality metadata and skipped messages make root-cause reconstruction and replay incomplete.
- Stress angle: at 1000x writes, transient faults produce more skipped events; without DLQ/replay contract, data drift becomes chronic.
- Lazy-implementer angle: teams assume "logged and skipped" is acceptable because SDK default behavior permits it.

**Severity:** Medium-High  
**Confidence:** 0.89

**So What? (next-day action)**
Require `correlation_id` for mutating flows, add DLQ + replay contract as non-optional platform behavior for critical domain events.

---

## 3. Risks Identified

| Risk | Severity | Likelihood | Notes |
|------|----------|------------|-------|
| Booking orders with no payability gate causes downstream operational waste | High | High | §F1 |
| Cancel/ship/release counter ambiguity causes stranded fulfillment states | High | Medium-High | §F2 |
| Complaint process can be formally complete but customer-visible trail suppressed | Medium-High | High | §F3 |
| Unauthorized ownership reassignment in CRM affects revenue attribution/control | High | Medium | §F4 |
| OP lifecycle can be marked complete without strong physical traceability | High | Medium | §F5 |
| Self-verify + hold abuse can bypass controls or halt production flow | High | Medium | §F6 |
| Cross-tenant semantic drift from open vocab fields weakens platform consistency | Medium | High | §F7 |
| Event skip behavior without replay guarantees creates hard-to-repair drift | Medium-High | Medium | §F8 |

---

## 4. Recommendations

| Priority | Recommendation | Effort | Expected Benefit |
|----------|----------------|--------|------------------|
| P0 | Add explicit `book_precheck` policy and override audit to Sales-Orders booking | Medium | Blocks unfunded order churn early |
| P0 | Define immutable ownership + controlled reassignment endpoint in CRM | Low | Prevents opportunity hijack |
| P0 | Require physical-trace provenance on OP return transitions | Medium | Prevents phantom returns and reconciliation fraud |
| P1 | Add cancellation transition guards and transactional lock rules for blanket release counters | Medium | Prevents stranded shipment and over-release races |
| P1 | Enforce visible customer communication requirement for `responded` transition | Low | Prevents hidden-resolution abuse |
| P1 | Make two-person verification default; add hold TTL/escalation controls | Medium | Reduces SoD bypass and hold-based DoS |
| P2 | Publish interoperability taxonomy layer for CRM stages / OP service types | Medium | Limits hostile-vertical semantic drift |
| P2 | Add DLQ + replay + required causality fields for critical events | High | Improves incident recovery and data convergence |

---

## 5. New Ideas and Extensions

### Incremental
- **Policy Profiles per Tenant:** keep base canonical model, but require explicit per-tenant policy manifest for risky transitions (book, cancel, verify, return).
- **Risky-Action Audit Feed:** emit a unified `platform.security_sensitive_action.v1` event for overrides and manual exception paths.

### Significant
- **Cross-Module Transition Guard Library:** standardized guard functions (state + permission + provenance) reused by module handlers to prevent copy-paste security drift.

### Radical
- **Platform Integrity Graph:** background service that continuously validates causal chain completeness (e.g., booked -> shipped -> invoice requested -> invoice issued) and flags broken chains before month-end close.

---

## 6. Assumptions Ledger

1. These specs are authoritative for MVP behavior and no hidden stronger guardrails already exist.
2. Tenant admins can configure roles/permissions with enough flexibility that permissive policies are realistic.
3. No external compensating control currently guarantees strict SoD in shop-floor operations.
4. Event consumer skip behavior in SDK applies to these new modules unless explicitly overridden.
5. Pre-launch means blast radius is lower than mature production, but first-customer trust risk is still material.

---

## 7. Questions for Project Owner

1. Do you want **book_precheck** to be mandatory by platform default, or opt-in per tenant?
2. Should `sales-orders` cancellation be forbidden after any shipped quantity unless a return/credit workflow exists?
3. For complaints, should customer-facing visibility be a hard invariant or a policy toggle with compliance audit?
4. For CRM ownership, do you want reassignment to require manager approval or just audit logging?
5. For OP returns, is Shipping-Receiving reference mandatory, or can manual overrides remain with elevated permission?
6. For shop-floor verification, is two-person control a platform default requirement?
7. Should DLQ/replay be in SDK baseline before implementation beads start?

---

## 8. Points of Uncertainty

- Exact authz model granularity is not fully specified (role strings exist, but assignment governance detail is outside these docs).
- The eventual reporting/analytics architecture is not specified; taxonomy drift impact could be mitigated elsewhere.
- Some mitigations may already be intended for implementation beads but are not present in architecture text.

---

## 9. Agreements and Tensions with Other Perspectives

**Likely agreements**
- Systems-thinking and operations-focused reviews should agree on causal-chain gaps (`book -> ship -> invoice`, `ship -> return -> review`).
- Security/compliance perspectives should agree on ownership mutation, visibility suppression, and SoD bypass risks.

**Likely tensions**
- Product velocity perspective may argue these controls are too strict pre-launch; adversarial view argues these are the cheapest stage to define hard invariants.
- Vertical-flexibility perspective may resist canonical interoperability taxonomies; adversarial view treats unconstrained vocabulary as long-run control debt.

---

## 10. Confidence: 0.90

**Calibration note:** Confidence is high on exploit feasibility because findings are contract-level permission/state gaps, not implementation bugs. Severity is intentionally calibrated to pre-launch context (mostly High/Medium, no blanket Critical), reflecting limited current blast radius but high probability of costly operational inconsistency if left unspecified.
