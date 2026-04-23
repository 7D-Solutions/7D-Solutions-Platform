# MODE_OUTPUT_F2 — Dependency-Mapping Analysis

**Mode:** F2 — Dependency-Mapping  
**Analyst:** LavenderGlacier  
**Date:** 2026-04-17  
**Scope:** bd-ixnbs Fireproof→Platform migration spec set — 5 new modules + 7 extensions

---

## 1. Thesis

The six new specs are well-bounded within their own domains, but seven concrete dependency seams will create pain at implementation time. The most critical: Manufacturing Costing subscribes to a retired module's event, Shop-Floor-Gates creates an enforcement obligation on Production with no mechanism defined in either spec, and the CRM-to-Sales-Orders close-won handoff has no entity linkage update path in either direction. Three additional high-severity gaps — Sales-Orders' dual customer identity pointers with no sync mechanism, event-based request/response patterns with no failure-path specification, and a blanket-release dual-write with no single transactional contract — are each individually addressable with small spec additions. These are not theoretical concerns: each is a concrete dependency edge that implementors will need to resolve before writing the first migration file, and leaving them open guarantees inconsistent implementations across beads.

---

## 2. Top Findings

### §F2-1 Manufacturing Costing subscribes to an event from a retired module
**Evidence:** `PLATFORM-EXTENSIONS-SPEC.md` §4 (Production Extension — Manufacturing Costing), "Consumed events" table:
> `shop_floor_data.labor.approved.v1` → Production computes labor cost (duration × operator rate × workcenter cost rate) and posts

`shop-floor-data` was explicitly retired from platform scope. Per the migration plan §C retirement: "kiosks + operator sessions + kiosk-driven labor capture stay in Fireproof as shop-specific hardware + bespoke workflow." The plan further states: "Only the barcode resolution portion moved to the Inventory extension."

**Reasoning chain:** F2 maps the consumer→producer edge. The edge `Production costing → shop_floor_data.labor.approved.v1` points at a Fireproof-local event — a vertical-specific producer. Platform's Production costing module would be subscribing to a vertical-specific event, making the extension only functional for Fireproof. HuberPower has no SFDC kiosk; its labor events will come from a different source. The extension as written is not vertical-neutral despite being a platform module.

**Severity:** Critical  
**Confidence:** 0.97  
**So What?** Before creating the manufacturing costing implementation bead: define what platform-level event carries approved labor data. Options: (a) Production already owns time entries — add `production.time_entry.approved.v1` as a platform event; (b) costing listens to `production.operation.completed.v1` and computes cost from stored time entries, avoiding event subscription entirely. Option (b) eliminates the external dependency. Decide before the bead is written.

---

### §F2-2 Shop-Floor-Gates enforcement contract with Production has no mechanism defined
**Evidence:** `SHOP-FLOOR-GATES-MODULE-SPEC.md` §8, Invariant 8:
> "Hold prevents operation start when active on that operation. Downstream — Production should check for active operation-scoped holds before allowing an operation to start. Platform Gates emits `hold.placed.v1`; Production is the enforcer, not Gates. (Alternative: Gates returns active holds via a GET endpoint; Production calls it. Either works; design detail for implementation bead.)"

Production is not listed as a consumer of `shop_floor_gates.hold.placed.v1` anywhere in the Gates spec. No `GET /api/shop-floor-gates/.../active-holds` endpoint exists in the Gates OpenAPI surface.

**Reasoning chain:** Authority is split: Gates owns the hold record, Production owns operation execution, but the enforcing relationship is undefined at both ends. The spec offers two implementation options but chooses neither. An implementor writing the Production bead will guess. An implementor writing the Gates bead will assume Production handles it. Both will rationalize their choice as consistent with "design detail for implementation bead" — and the invariant will be implemented differently depending on which bead author reads which spec first.

**Severity:** High  
**Confidence:** 0.95  
**So What?** Decide the mechanism now: either (a) add `shop_floor_gates.hold.placed.v1` to Production's consumed-events section with specified behavior (cache locally, check at operation-start), or (b) add `GET /api/shop-floor-gates/work-orders/:wo_id/active-holds` to Gates' OpenAPI surface and record that Production calls it synchronously at operation start. If (b), define the response payload. Record the decision in both specs before either implementation bead is created.

---

### §F2-3 CRM → Sales-Orders close-won handoff has no entity linkage update path
**Evidence:** `CRM-PIPELINE-MODULE-SPEC.md` §3, `opportunities` table:
> `sales_order_id` (nullable — set when opportunity generated an SO)

`SALES-ORDERS-MODULE-SPEC.md` §4.1 — no `opportunity_id` field on any table.

`CRM-PIPELINE-MODULE-SPEC.md` §5.2:
> "If the SO references an opportunity via soft linkage (downstream populates), log context; no state change"

`CRM-PIPELINE-MODULE-SPEC.md` §9:
> "The handoff flow (opp close-won → SO create) can be implemented as an event subscriber on Sales-Orders side or as a manual operator action; either works."

**Reasoning chain:** The close-won flow requires that `opportunities.sales_order_id` gets populated after SO creation. But: (1) Sales-Orders has no `opportunity_id` field, so an SO cannot carry its origin; (2) Sales-Orders' consumed-events section shows CRM events only for "log context; no state change" — it has no subscriber that creates SOs from close-won events; (3) If the vertical orchestrates SO creation manually, the SO exists but CRM can't learn its ID unless the operator also calls `PUT /crm-pipeline/opportunities/:id`. The spec says "either works" but neither path is defined well enough to implement consistently.

**Severity:** High  
**Confidence:** 0.90  
**So What?** Add `opportunity_id` (nullable, opaque ref to CRM) to `sales_orders` table. Flow becomes: close-won → vertical creates SO with `opportunity_id` set → CRM subscribes to `sales_orders.order.booked.v1` (payload includes `opportunity_id`) → CRM updates `sales_order_id`. This makes CRM reactive, SO ignorant of CRM internals, and gives a clear, testable linkage path. The nullable column adds zero coupling.

---

### §F2-4 Sales-Orders carries dual customer identity with no synchronization mechanism
**Evidence:** `SALES-ORDERS-MODULE-SPEC.md` §3, `sales_orders` table:
> `customer_id` (ref → AR customer), `party_id` (ref → Party)

`CRM-PIPELINE-MODULE-SPEC.md` §9:
> "no automatic AR creation — vertical orchestrates this via their own event handler"

`SALES-ORDERS-MODULE-SPEC.md` §5.2: `party.party.deactivated.v1` is NOT in the consumed events list.

**Reasoning chain:** Sales-Orders needs billing identity (`customer_id` in AR) for invoice triggering and relational identity (`party_id` in Party) for ship-to addresses. But no platform mechanism links the two. Specifically: (1) who creates the AR customer record for a Party is undefined — verticals own this, but the SO spec assumes both IDs are always valid; (2) `party.party.deactivated.v1` is not consumed by Sales-Orders, so a deactivated party can still receive new orders; (3) no lookup contract is defined for how a new SO knows what `customer_id` maps to a given `party_id`. Customer-Complaints uses only `party_id` (no AR reference) — the identity split is already causing inconsistency across modules.

**Severity:** High  
**Confidence:** 0.88  
**So What?** Two small spec additions: (1) Add `party.party.deactivated.v1` to Sales-Orders consumed events with specified behavior (block new SO creation; surface warning on in-flight SOs). (2) Document the party_id→customer_id resolution contract explicitly: either SO calls AR's lookup endpoint at booking time, or verticals are responsible for ensuring customer_id is valid before SO creation. The second option is acceptable but must be stated, not implied.

---

### §F2-5 Event-based request/response in Sales-Orders has no failure-path specification
**Evidence:** `SALES-ORDERS-MODULE-SPEC.md` §5.1/5.2:
- Produces `sales_orders.reservation.requested.v1`, consumes `inventory.reservation.confirmed.v1` / `inventory.reservation.rejected.v1`
- Produces `sales_orders.shipment.requested.v1`, consumes `shipping_receiving.shipment.shipped.v1`
- Produces `sales_orders.invoice.requested.v1`, consumes `ar.invoice.issued.v1`

The AR spec's consumed-events list (`AR-MODULE-SPEC.md` §5.2) does not include `sales_orders.invoice.requested.v1`.

**Reasoning chain:** These patterns are request/response over the event bus. SO fires a request, then waits for a response event to advance state. The spec defines happy paths but says nothing about: (a) what happens when `inventory.reservation.confirmed.v1` never arrives; (b) the timeout before surfacing a "reservation pending" alert; (c) whether SO can be manually advanced if Inventory is down; (d) the AR consumed-events gap means `invoice.requested.v1` has no defined consumer at all. Additionally the AR spec shows no consumer for SO invoice requests — an S-grade billing gap for a B2B ERP.

**Severity:** High  
**Confidence:** 0.93  
**So What?** (1) Add `sales_orders.invoice.requested.v1` to AR's consumed-events section with specified behavior (create draft invoice, idempotent on `sales_order_id` + `line_id`). (2) Add to Sales-Orders spec: a `stock_status` column on `sales_order_lines` (the open-questions section already anticipates this for the backorder case — same mechanism), a timeout policy for reservation response ("if no confirmation within N hours, surface as unconfirmed"), and a manual override permission for operators to advance state when dependent modules are unavailable.

---

### §F2-6 Blanket release creation is a dual-write with no single transactional contract
**Evidence:** `SALES-ORDERS-MODULE-SPEC.md` §3 `blanket_order_releases` table:
> `sales_order_id` (ref back to the SO this release generated)

`SALES-ORDERS-MODULE-SPEC.md` §4.3:
> `POST /api/sales-orders/blankets/:id/lines/:line_id/releases` — Create a release (draw down against commitment); generates a child SO

**Reasoning chain:** A blanket release creation writes two authoritative rows in one business action: a `blanket_order_releases` row and a child `sales_orders` row. Both are in the same module's DB, so they can be transactional. But Invariant 4 (`SUM(releases.release_qty) <= committed_qty`) must be checked and the child SO must be created atomically. If the SO creation fails after the release row is written (e.g., numbering service unavailable), the committed-qty balance is wrong. No idempotency key or compensating transaction is specified.

**Severity:** Medium-High  
**Confidence:** 0.84  
**So What?** Since both writes are in the same module's DB, this is solvable with a single Postgres transaction and an idempotency key. Add: (1) `idempotency_key` column to `blanket_order_releases`; (2) explicit spec statement that release creation and child SO creation are one atomic transaction; (3) note that Numbering service unavailability should cause the entire operation to reject cleanly, not partially commit.

---

### §F2-7 BOM extensions use divergent on-hand patterns for similar computations
**Evidence:** `PLATFORM-EXTENSIONS-SPEC.md` §1 (MRP):
> "The on_hand input is caller-supplied. Keeps the computation deterministic and auditable."

§5 (Kit Readiness):
> "Pulls on-hand from Inventory (uses Inventory's availability query) rather than taking it as input like MRP does."

**Reasoning chain:** Both extensions live in the same BOM module, both compute BOM explosion × on-hand arithmetic. MRP takes a caller-supplied snapshot; Kit Readiness queries Inventory synchronously. This means the BOM module needs an Inventory HTTP client for Kit Readiness but not for MRP — an unplanned outbound dependency for BOM. An implementor writing both beads faces an inconsistency in BOM's dependency graph. The choice also affects testing: MRP is trivially deterministic; Kit Readiness requires Inventory to be running.

**Severity:** Medium  
**Confidence:** 0.85  
**So What?** Settle the pattern in the spec: either (a) accept that BOM acquires an Inventory HTTP client (document this explicitly in BOM's cross-module integration notes), or (b) standardize on caller-supplied on-hand for Kit Readiness too (callers supply fresh on-hand from a prior Inventory query). Option (b) is architecturally cleaner — BOM stays a pure computation module. Option (a) is more ergonomic for the "check now" use case. Either is defensible; the current silence is not.

---

## 3. Risks Identified

| Risk | Severity | Likelihood |
|------|----------|------------|
| Manufacturing Costing blocked on missing platform labor event | Critical | High — SFDC retired; no alternative event named |
| Shop-Floor-Gates hold enforcement not implemented in Production | High | High — neither spec defines the mechanism |
| SO-to-invoice handoff has no AR consumer defined | High | High — AR spec has no entry for `invoice.requested.v1` |
| CRM→SO linkage implemented inconsistently across verticals | High | Medium — ambiguity invites divergence |
| Sales-Orders stuck in `booked` permanently if Inventory is partitioned | High | Medium — event loss is normal under partition |
| Party↔AR customer identity drift silently accepted | High | Low-Medium — no enforcement, no sync event |
| Blanket release partial-commit corrupts committed-qty balance | Medium-High | Medium — retry without idempotency key is the failure mode |
| BOM gains unplanned Inventory HTTP client dependency | Medium | High — Kit Readiness as specced requires it |

---

## 4. Recommendations

| # | Priority | Effort | Recommendation | Expected Benefit |
|---|----------|--------|----------------|-----------------|
| R1 | P0 | Low | Replace `shop_floor_data.labor.approved.v1` in Manufacturing Costing with `production.time_entry.approved.v1` or eliminate the subscription entirely (compute from stored time entries on WO close) | Unblocks costing bead; makes extension vertical-neutral for HuberPower |
| R2 | P0 | Low | Add `sales_orders.invoice.requested.v1` to AR's consumed-events section with specified behavior (idempotent invoice creation keyed on sales_order_id + line_id) | Closes the billing gap; prevents missed invoices |
| R3 | P0 | Low | Decide and document the Gates-Production hold enforcement mechanism; add to both specs | Makes the invariant concrete before either module bead is written |
| R4 | P1 | Low | Add `opportunity_id` (nullable) to `sales_orders` table; add `sales_orders.order.booked.v1` to CRM consumed-events with "update opp.sales_order_id if payload.opportunity_id matches" behavior | Closes CRM-SO linkage gap with one column and one consumer entry |
| R5 | P1 | Low | Add `party.party.deactivated.v1` to Sales-Orders consumed events; document party_id→customer_id resolution contract | Prevents SO creation against deactivated parties; makes identity model explicit |
| R6 | P1 | Low | Add timeout/manual-override spec for reservation and shipment event-response patterns in Sales-Orders | Prevents permanent stuck state under partition; gives operators an escape hatch |
| R7 | P2 | Low | Wrap blanket release creation + child SO creation in one explicit atomic transaction with an idempotency key | Prevents committed-qty corruption on partial failure |
| R8 | P2 | Low | Settle BOM on-hand pattern; document Inventory HTTP client in BOM's integration notes if Kit Readiness keeps querying Inventory directly | Prevents implementor confusion; makes BOM's coupling model explicit |

---

## 5. New Ideas and Extensions

**Incremental:**
- Add `GET /api/shop-floor-gates/work-orders/:wo_id/active-holds` to Gates' OpenAPI surface — this endpoint is implicitly required whether Production uses event-subscription or synchronous-check enforcement, and should be explicit.
- Add `opportunity_id` to `sales_orders.sales_orders` as a nullable opaque string — zero coupling cost, prevents future backfill.
- Add `GET /api/sales-orders/orders/:id/reservation-status` — lets UIs poll line-level stock confirmation without depending on event delivery.

**Significant:**
- Define a platform-level `labor.cost_eligible.v1` event that any vertical's labor-capture system can emit with standardized fields (operator_id, work_order_id, operation_id, duration_minutes, workcenter_id, approved_at). Manufacturing Costing subscribes to this, not to Fireproof's SFDC event. Fireproof emits it from its kiosk module; HuberPower emits it from whatever its timekeeping system is. This makes the costing extension genuinely vertical-neutral without requiring a platform labor-capture module.

**Radical:**
- The reservation→confirm, shipment→shipped, and invoice→issued patterns are all request/response over the event bus. A lightweight platform-level saga helper (not a saga coordinator service — just a pattern library) that standardizes timeout, retry, and compensate for these intra-platform async handoffs would prevent each module from inventing its own timeout logic. Defer unless implementation reveals the pattern proliferating.

---

## 6. Assumptions Ledger

1. Production module has time entries and a `production.time_entry.*` event family — not in scope for this review but Manufacturing Costing depends on it.
2. NATS event delivery is at-least-once with consumer-side deduplication — relevant to the severity of Sales-Orders event-response gaps.
3. Party module emits `party.party.deactivated.v1` and `party.contact.deactivated.v1` — inferred from CRM and Customer-Complaints consumed-events lists.
4. AR module supports a `GET /api/ar/customers?party_id=X` lookup or equivalent — needed for party_id→customer_id resolution but not confirmed in AR spec.
5. CRM's `PUT /api/crm-pipeline/opportunities/:id` allows setting `sales_order_id` post-creation — implied but not stated.
6. Blanket release creation and child SO creation are in the same Postgres DB (same module, same service) — if not, the transaction scope analysis changes.

---

## 7. Questions for Project Owner

1. **Labor event for costing:** Does `production.time_entry.approved.v1` exist? Or should Manufacturing Costing compute from stored time entries on WO close rather than subscribing to an event?

2. **Gates enforcement mechanism:** Synchronous hold-check endpoint, or event-driven Production projection? Needs to be settled before either bead is created.

3. **AR invoice consumer:** Is the intent that AR creates invoices in response to `sales_orders.invoice.requested.v1`? If so, why is this missing from AR's consumed-events spec?

4. **CRM→SO linkage:** Should `sales_orders` get an `opportunity_id` column? Or is the vertical handler responsible for calling `PUT /crm-pipeline/opportunities/:id` after SO creation?

5. **Party↔AR customer binding:** Is there a platform rule that every `party_id` on a Sales Order must have a corresponding AR customer? If not, what happens at invoicing when `customer_id` is absent?

---

## 8. Points of Uncertainty

- Production module's current event contract is not in scope. Manufacturing Costing's viability depends on Production events that may or may not exist.
- The Shipping-Receiving spec is not reviewed here; the OP and SO shipment.requested → shipment.shipped interactions depend on how Shipping-Receiving matches source references.
- Whether NATS provides guaranteed delivery or best-effort affects the severity of §F2-5.
- The blanket-release dual-write (§F2-6) is only a risk if the Numbering service is a separate process; if numbering is in-process, the transaction is trivial.

---

## 9. Agreements and Tensions with Other Perspectives

**Agreements with F7 (Systems-Thinking):** The labor event gap (§F2-1) and Gates-Production enforcement gap (§F2-2) are likely to surface from a whole-system integration view as well — they are the most structurally visible missing edges in the dependency graph.

**Agreements with H2 (Adversarial-Review):** The Sales-Orders stuck-in-booked scenario (§F2-5) and the AR invoice consumer gap (§F2-5) are billing attack surfaces that H2 would likely target.

**Agreements with A1 (Deductive):** The CRM close-won handoff ambiguity (§F2-3) is a logical inconsistency: both sides of the boundary define a field that the other side must populate, but neither defines the write path.

**Potential tension with F5 (Root-Cause):** F5 might find the dual identity problem (§F2-4) to be an inherent consequence of separating AR from Party — not a spec defect. F2's view is that this is a missing linkage specification, addressable with a small consumed-event and an explicit contract.

**Potential tension with I4 (Perspective-Taking):** An implementor perspective might treat the blanket-release gap (§F2-6) as a trivially obvious transaction — "of course you wrap it." F2 flags it because the spec does not state this, and implementation consistency requires it to be explicit.

---

## 10. Confidence

**Overall confidence: 0.88**

Calibration: The critical finding (§F2-1) is 0.97 confident because SHOP-FLOOR-DATA is explicitly retired by name and the consumed event points at it by name — no inference needed. The AR invoice consumer gap (§F2-5) is 0.93 confident because the AR spec is in scope and its consumed-events table is complete enough to verify the absence. The CRM-SO linkage finding (§F2-3) is 0.90 — slightly lower because "event subscriber on Sales-Orders side" could be an intended implementation detail that the spec author knows but didn't write down. The identity gap (§F2-4) is 0.88 because a gap in the spec doesn't prove a gap in the team's mental model.

**Caveats:** If Production already has a `production.time_entry.approved.v1` event and a hold-check endpoint, §F2-1 and §F2-2 severity drop significantly. Reading Production's spec would resolve both.
