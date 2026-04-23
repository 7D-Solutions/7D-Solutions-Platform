# MODE_OUTPUT_A1 — Deductive Analysis

**Mode:** A1 — Deductive  
**Analyst:** AgentCursor  
**Date:** 2026-04-16  
**Spec set:** `bd-ixnbs` migration specs (5 new modules + 7 extensions)

---

## 1. Thesis

Read deductively, the spec set is close on local module behavior but incomplete on the seams where one module's canonical intent becomes another module's input. The largest gaps are not in the core domain models themselves; they are in ownership of handoff contracts, lineage fields, and label scopes. That creates three classes of failure: cross-module billing and fulfillment events that no consumer is actually assigned to handle, line-item models that allow states the downstream reservation logic cannot represent, and a few "tenant-configurable" fields that are exposed as if canonical but are not actually normalized enough to be safe across verticals.

---

## 2. Top Findings

### §F1 — Sales-Orders Emits Billing Intent, But No Module Owns the Handoff

- **Evidence:** `docs/architecture/SALES-ORDERS-MODULE-SPEC.md:117-120` emits `sales_orders.invoice.requested.v1` on ship; `docs/architecture/SALES-ORDERS-MODULE-SPEC.md:183-186` says AR consumes invoice requests in the integration notes; `docs/architecture/AR-MODULE-SPEC.md:123-158` has no consumer for any `sales_orders.*` event and no invoice-request contract.
- **Reasoning chain:** Deductively, this is a contract gap, not a feature gap. Sales-Orders declares that shipping a line creates an invoice request, but AR never states that it listens for that request, nor does it define the request shape or the bundling rule for one invoice versus many. Because AR owns invoice truth, the lack of an explicit consumer means each vertical will invent its own billing bridge, and some will likely invoice per shipped line while others aggregate at order level. That is a direct ambiguity in a core money flow.
- **Severity:** High
- **Confidence:** 0.96
- **So What?:** Add a concrete AR consumer contract for `sales_orders.invoice.requested.v1` and specify whether the unit of work is a shipped line or a whole order. If AR should aggregate, say so in the schema and the handler rules now, before implementation beads split the behavior by vertical.

### §F2 — Sales-Orders Allows Ad-Hoc Lines That Reservation Cannot Process

- **Evidence:** `docs/architecture/SALES-ORDERS-MODULE-SPEC.md:55-56` allows `sales_order_lines.item_id` to be nullable for ad-hoc lines; `docs/architecture/SALES-ORDERS-MODULE-SPEC.md:117-119` says booking emits `sales_orders.reservation.requested.v1` with `item_id` per line.
- **Reasoning chain:** The schema and the event contract disagree. A booked order may legally contain a line with no `item_id`, but booking emits a reservation request that requires `item_id`. The spec does not say whether ad-hoc lines are excluded from booking, skipped in reservation, or transformed into a different request type. That leaves a representation hole: the module can store a line it cannot operationally process.
- **Severity:** High
- **Confidence:** 0.94
- **So What?:** Split order lines into at least two operational classes or add an explicit rule that only stock-bearing lines emit reservation requests. If service/ad-hoc lines are allowed, they need a separate fulfillment path, not an implicit "maybe reserve" branch.

### §F3 — Blanket Release Labeling Is Exposed in the API but Missing in Storage

- **Evidence:** `docs/architecture/SALES-ORDERS-MODULE-SPEC.md:60-61` defines label tables only for order and blanket statuses; `docs/architecture/SALES-ORDERS-MODULE-SPEC.md:95-98` exposes `GET /api/sales-orders/status-labels/:scope` and `PUT /api/sales-orders/status-labels/:scope/:canonical` with `scope = order | blanket | release`; `docs/architecture/SALES-ORDERS-MODULE-SPEC.md:149-154` defines a separate release lifecycle.
- **Reasoning chain:** The endpoint contract promises tenant labels for release scope, but the data model never gives release statuses a home. That means release labels cannot be persisted, migrated, or validated consistently. In a multi-tenant system, "API exists but backing table does not" becomes a hidden incompatibility the first time a tenant tries to rename release states.
- **Severity:** Medium
- **Confidence:** 0.91
- **So What?:** Either add a `release_status_labels` table and wire the CRUD endpoints to it, or remove `release` from the label API scope. Right now the API and schema disagree.

### §F4 — Invoice-Related Complaints Have No AR Event Trail

- **Evidence:** `docs/architecture/CUSTOMER-COMPLAINTS-MODULE-SPEC.md:51` allows `source_entity_type` values including `invoice`; `docs/architecture/CUSTOMER-COMPLAINTS-MODULE-SPEC.md:113-118` consumes only Sales-Orders and Shipping-Receiving events, not AR invoice events; `docs/architecture/AR-MODULE-SPEC.md:127-158` defines invoice lifecycle events but nothing is published for complaint correlation.
- **Reasoning chain:** The module explicitly says complaints can be about invoices, but it does not subscribe to any AR invoice lifecycle events. Deductively, invoice complaints will be the one source entity class with no automatic timeline enrichment, no state correlation, and no event-driven warning when the underlying invoice changes. That is a predictable support and audit gap.
- **Severity:** Medium
- **Confidence:** 0.89
- **So What?:** Add AR invoice events to the Customer-Complaints integration notes and define which ones enrich complaint timelines. At minimum, complaints referencing invoices should ingest issued/voided/paid state changes.

### §F5 — Outside-Processing Cannot Reconcile Returns Back to a Specific Outbound Shipment

- **Evidence:** `docs/architecture/OUTSIDE-PROCESSING-MODULE-SPEC.md:56-57` defines `op_ship_events.shipping_reference` but no equivalent linkage on `op_return_events`; `docs/architecture/OUTSIDE-PROCESSING-MODULE-SPEC.md:122-123` says inbound returns create a matching return-event stub from Shipping-Receiving.
- **Reasoning chain:** The outbound side has a traceable reference; the inbound side does not. If a vendor sends back multiple partial shipments, mixed-condition returns, or a rework cycle, the platform cannot tell which outbound shipment a given return came from without an extra reconciliation rule outside the spec. Deductively, that breaks lineage exactly where vendor round-trip tracking is supposed to be authoritative.
- **Severity:** Medium-High
- **Confidence:** 0.93
- **So What?:** Add an explicit return-to-shipment reference field and make it part of the event payload and data model. Without that pointer, partial and mixed returns will depend on human guesswork.

### §F6 — Outside-Processing Treats `service_type` as Open Text While Events Need It to Be Stable

- **Evidence:** `docs/architecture/OUTSIDE-PROCESSING-MODULE-SPEC.md:60-63` says `service_type` is intentionally open and may be free text; `docs/architecture/OUTSIDE-PROCESSING-MODULE-SPEC.md:108-115` publishes `service_type` in multiple events; `docs/plans/bd-ixnbs-fireproof-platform-migration.md:83-84` says canonical codes in events should stay canonical and tenants may only rename display labels.
- **Reasoning chain:** The spec simultaneously treats `service_type` as a business discriminator and as something that can be free text. That is fine for a note field, but not for an event payload that other modules may eventually consume. Deductively, a free-text `service_type` cannot serve as a reliable integration key across verticals, and the spec does not tell consumers how to distinguish "canonical code" from "tenant prose."
- **Severity:** Medium
- **Confidence:** 0.86
- **So What?:** Either freeze `service_type` to tenant-managed canonical codes or explicitly downgrade it to display-only text and add a separate canonical code field for integration. Right now the event payload implies more stability than the model guarantees.

---

## 3. Risks Identified

| Risk | Severity | Likelihood | Notes |
|------|----------|------------|-------|
| Billing handoff from Sales-Orders to AR fragments by vertical | High | Likely | §F1 |
| Booked ad-hoc lines fail reservation or are silently skipped | High | Likely | §F2 |
| Blanket-release label UI exists without persisted label state | Medium | Certain if used | §F3 |
| Invoice complaints lose event-driven context | Medium | Likely | §F4 |
| OP return lineage cannot be audited cleanly | Medium-High | Likely | §F5 |
| `service_type` becomes a non-portable free-text discriminator | Medium | Likely | §F6 |

---

## 4. Recommendations

| Priority | Recommendation | Effort | Expected Benefit |
|----------|---------------|--------|-----------------|
| P0 | Define and implement the AR consumer for `sales_orders.invoice.requested.v1`, including bundling semantics | Low-Med | Removes the most important unowned handoff in the set |
| P0 | Add an explicit line-kind rule to Sales-Orders so ad-hoc lines do not participate in inventory reservation by accident | Low | Prevents invalid booking states |
| P1 | Add `release_status_labels` or remove `release` from the Sales-Orders label API | Low | Keeps API and schema aligned |
| P1 | Add AR invoice lifecycle events to Customer-Complaints enrichment | Low | Restores complaint traceability for invoice-driven issues |
| P1 | Add a return-to-shipment reference on `op_return_events` and emit it in the return stub payload | Low-Med | Preserves vendor round-trip lineage |
| P2 | Replace open-text `service_type` with tenant-canonical codes or split code vs display label explicitly | Low-Med | Makes OSP event consumers safe across verticals |

---

## 5. New Ideas and Extensions

### Incremental

- Add a dedicated `invoice.requested.v1` contract under the billing handoff path, with one owner and one bundling rule.
- Add a `line_kind` field to Sales-Orders lines, so stock, service, freight, and blanket-release lines can take different downstream paths without guessing.

### Significant

- Add a lineage pointer model for round-trip operational records, so outbound shipment, return, review, and close can be traced with the same reference chain.
- Add complaint enrichment subscriptions for billing events, not just logistics events, so the complaint timeline is complete for all major source types.

### Radical

- Introduce a thin platform-wide "document lineage" contract for cross-module operational references, limited to stable IDs and event links, so modules can correlate order, invoice, complaint, and vendor round-trip records without inventing their own foreign-key-like conventions.

---

## 6. Assumptions Ledger

1. `sales_orders.invoice.requested.v1` is intended to be a real cross-module contract, not just a Fireproof-local convention.
2. Sales-Orders can legally contain mixed line types, including non-stock service lines.
3. AR is expected to own invoice creation behavior, not Sales-Orders.
4. Complaint source entities are meant to include invoices in production use, not only as illustrative examples.
5. Outside-Processing is expected to support partial and multi-stage vendor round trips in real workflows.
6. `service_type` is meant to be consumed beyond the immediate OSP module, otherwise the event payload does not need to expose it so prominently.

---

## 7. Questions for Project Owner

1. Should `sales_orders.invoice.requested.v1` represent a shipped line or a whole order?
2. Are nullable Sales-Orders lines intended to be non-stock service lines, and if so should they skip reservation automatically?
3. Should blanket releases get their own persisted label table, or should the `release` label API scope be removed?
4. Should Customer-Complaints subscribe to AR invoice lifecycle events for invoice-related complaints?
5. Do Outside-Processing returns need a direct reference to the outbound ship event or shipment receipt?

---

## 8. Points of Uncertainty

- The spec set does not show whether AR already has an unpublished consumer for `sales_orders.invoice.requested.v1`.
- It is unclear whether `service_type` is intended to be a stable code registry or merely tenant-editable prose.
- The return-event lineage problem may already be solved in Fireproof source code, but the platform spec does not carry that solution forward.
- The complaint module may rely on manual operator lookup for invoice context, but the spec does not state that as a fallback.

---

## 9. Agreements and Tensions with Other Perspectives

**Agreements**
- A data-architecture review would likely agree that the biggest defects are missing lineage and missing contract ownership, not local field shape.
- A security/compliance review would likely agree that return lineage and invoice complaint correlation matter because they affect auditability.

**Tensions**
- A minimalism-first review might argue that some of these seams can be left to implementation beads. The deductive counterpoint is that each seam already crosses a module boundary, so leaving it implicit guarantees divergent implementations.
- A "keep it flexible" perspective might favor open-text `service_type`. The tension is that open text is only flexible until another module needs to consume it; then it becomes an interoperability defect.

---

## 10. Confidence: 0.88

Calibration note: confidence is high because the findings come directly from explicit mismatches between table shapes, endpoint contracts, event lists, and integration notes. It is not higher because a few conclusions depend on unstated implementation intent, especially around AR billing ownership and whether `service_type` is meant for cross-module consumption.
