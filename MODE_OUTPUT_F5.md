# MODE_OUTPUT_F5 — Root-Cause Analysis
**Mode:** F5 — Root-Cause Analysis  
**Analyst:** CopperBarn  
**Date:** 2026-04-16  
**Specs reviewed:** bd-ixnbs migration plan + 5 new module specs + 7 extension specs + AR template + layering/boundary/contract docs

---

## 1. Thesis

The specs are generally well-structured, but a recurring pattern undermines several of them: **boundaries were drawn at the Fireproof code-file boundary rather than at the domain-concern boundary.** This produces three specific failure modes: (A) a module that owns two concerns that will be owned by different people at different change rates (Outside-Processing); (B) two extensions that implement the same computation under different names (MRP vs Kit Readiness); and (C) state machines that carry Fireproof's existing states without always carrying the triggers that drive those states. Separately, the decision to retire Shop-Floor-Data as a platform module was made without auditing its event consumers — leaving Manufacturing Costing dependent on an event that no platform module will emit. These are not implementation details; several of them have schema implications that must be resolved before decomposing into implementation beads.

---

## 2. Top Findings

---

### §F1 — Sales-Orders: `in_fulfillment` state and blanket-release `pending→released` have no defined triggers

**Evidence:** SALES-ORDERS-MODULE-SPEC.md §6 (State Machines). The SO lifecycle is `draft → booked → in_fulfillment → shipped → closed`. The `booked → in_fulfillment` transition has no defined trigger, no endpoint, and no event. Similarly, `blanket_order_releases.status` includes `pending` and `released` as distinct states, but §4.3 has no endpoint that moves a release from `pending` to `released` (only `create` and `ship`).

**Reasoning chain:** The spec was written from domain knowledge of Fireproof's working code. Fireproof already had these states implemented, so the spec author captured the states without explicitly asking "what fires this transition?" The state machine section is *descriptive* (what states exist) not *prescriptive* (what triggers each arc). This is a code-shape residue: the states exist in Fireproof; their triggers were implicit in the Fireproof implementation; the spec preserved the states but not the triggers.

**Why this isn't just an open question:** `in_fulfillment` is not a cosmetic state — it likely corresponds to "Production has picked up the work order for this SO" or "Inventory has confirmed reservation." Whether the trigger is an incoming event (from Production? Inventory?) or an explicit operator action determines whether Sales-Orders needs an event subscription that is currently absent from §5.2. This affects the module's consumed-events table, its subscription setup, and whether the transition is automatic or manual.

**Severity:** High  
**Confidence:** 0.95  
**So What?** Before decomposing the Sales-Orders implementation bead, specify: (1) what triggers `booked → in_fulfillment`, (2) whether blanket releases are created as `released` (not `pending`), and if `pending` is meaningful, define the `pending → released` endpoint and trigger.

---

### §F2 — Outside-Processing mixes operational and financial concerns at the wrong boundary

**Evidence:** OUTSIDE-PROCESSING-MODULE-SPEC.md §3 (Data Ownership). The `op_orders` table contains both operational fields (`status`, `shipped_to_vendor`, `returned`, `review_in_progress`) AND financial fields (`estimated_cost_cents`, `actual_cost_cents`, `purchase_order_id`). §9 says OP "creates or references an AP PO for the vendor. AP owns PO lifecycle; OP tracks operational lifecycle on top." But OP also holds `actual_cost_cents` on the OP order — a financial amount with no defined update mechanism.

**Reasoning chain (5 Whys):**
1. OP conflates ship/return lifecycle with cost tracking.
2. Because Fireproof's `outside_processing/` module had both (built by a shop floor team needing cost visibility).
3. The boundary is wrong because AP owns vendor bills, and the bill from the vendor is what establishes `actual_cost_cents`. There are now two places that claim to know how much a vendor job costs: the AP bill (authoritative) and OP's `actual_cost_cents` (set by whom? via what?).
4. The spec's open question section says "cost reconciliation: OP vs. AP bill... defer — not MVP need" without recognizing this is not a feature gap but a data integrity question: who is the source of truth for the cost of a completed outside-processing job?
5. Root cause: The module boundary was drawn at the Fireproof code file boundary, not at the domain split (operational tracking for shop floor vs. financial reconciliation for accounting).

**Concrete impact:** There is no defined event, endpoint, or process in the spec by which `actual_cost_cents` on an OP order ever gets set. If a vendor completes work and AP processes their bill, OP's cost field stays at whatever was estimated. The spec has `actual_cost_cents` as a field but no mechanism to populate it from reality.

**Severity:** High  
**Confidence:** 0.88  
**So What?** Either (a) remove `estimated_cost_cents` and `actual_cost_cents` from OP (AP owns cost; OP just holds the PO reference) and emit `outside_processing.order.closed.v1` with final accepted qty so AP can reconcile, or (b) define the explicit event/endpoint by which AP's bill settlement drives OP's `actual_cost_cents` update, with a stated source-of-truth invariant. Currently neither path is specified.

---

### §F3 — Shop-Floor-Data retirement leaves Manufacturing Costing with a broken event dependency

**Evidence:** PLATFORM-EXTENSIONS-SPEC.md §4 "Consumed events (cost-posting triggers)": `shop_floor_data.labor.approved.v1 → Production computes labor cost`. But SHOP-FLOOR-DATA-MODULE-SPEC.md is retired from platform scope (plan doc says kiosks + sessions + kiosk-driven labor stay in Fireproof). There is no platform module named `shop_floor_data` to emit this event.

**Reasoning chain:** The retirement of Shop-Floor-Data as a platform module and the inclusion of Manufacturing Costing as a Production extension were made as two separate decisions. No dependency audit was run between them. The migration plan's scope-decision framework evaluated each module for "cross-vertical applicability" but didn't map what each module *consumes* from others before retiring source modules. This is an absence of a forward-dependency graph.

**Concrete impact:**
- For **Fireproof**: Fireproof's local SFDC would emit a labor event, but with `source_module = "sfdc"` (or similar local name) — not `"shop_floor_data"`. That event name in the platform consumption table will never match.
- For **HuberPower**: HuberPower has no SFDC, no kiosk module. If they want manufacturing costing (they're a manufacturer), where does their `shop_floor_data.labor.approved.v1` come from? It doesn't. Their labor entries go through Production's time entries (the plan doc says "Platform's Production already owns Time Entries for cross-vertical labor tracking"). But Production's time-entry events are not listed as a trigger for manufacturing costing. So HuberPower's labor costs would never post.
- The correct trigger for labor cost posting is `production.time_entry.approved.v1` (or similar Production event) — not a retired module's event.

**Severity:** Critical  
**Confidence:** 0.92  
**So What?** Replace `shop_floor_data.labor.approved.v1` with `production.time_entry.approved.v1` (or whatever Production's time-entry approval event is called). Verify Production's existing time-entry events carry the fields needed for labor cost computation (operator rate, duration, workcenter). If Production doesn't yet emit a time-entry approval event, add it as part of the manufacturing costing bead — it's a Production event, not an SFDC event.

---

### §F4 — BOM MRP Explosion and Kit Readiness are the same computation with different names

**Evidence:** PLATFORM-EXTENSIONS-SPEC.md §1 (MRP Explosion) and §5 (Kit Readiness).
- MRP: `POST /api/bom/mrp/explode` — body: `{bom_id, demand_quantity, on_hand: [{item_id, quantity}]}` → net requirements per component
- Kit Readiness: `POST /api/bom/kit-readiness/check` — body: `{bom_id, required_quantity}` → per-component readiness (required_qty, on_hand_qty, available_qty, status)

The core computation is identical: BOM explosion times available on-hand equals net requirements/shortages. The differences are purely operational:
1. **On-hand sourcing**: MRP takes caller-supplied on-hand (deterministic); Kit Readiness queries Inventory live (fresh)
2. **Output vocabulary**: MRP says "net_quantity"; Kit Readiness says "ready/short/expired/quarantined"

These are two API styles for one domain operation, not two domain operations.

**Reasoning chain:** Fireproof has two separate modules (`mrp/` and `kit_readiness/`) that evolved independently for different workflows. The spec author preserved this two-module structure. Asking "if there were no Fireproof code, would you design two extensions?" — the answer is no. You'd design one endpoint with a `mode` parameter or an `on_hand_source` parameter (`caller_supplied | query_inventory`) and an `output_format` parameter (`requirements | readiness`).

**Why this matters for implementation:** Two separate snapshot tables (`mrp_snapshots` + `kit_readiness_snapshots`), two event types, two sets of SDK client methods. When a user asks "has anyone checked if we have enough material for work order 123?" they'd need to know which of two different systems to query. The distinction creates accidental complexity with no domain benefit.

**Severity:** Medium  
**Confidence:** 0.85  
**So What?** Merge into a single `POST /api/bom/kit-check` endpoint with parameters `on_hand_source: caller_supplied | query_inventory` and `output_format: net_requirements | readiness_status`. One snapshot table, one event. If the Fireproof `mrp/` module had different output fields worth preserving, capture them as additional fields on the unified response, not as a separate API.

---

### §F5 — CRM `contact_role_attributes` has no defined cross-module consistency mechanism

**Evidence:** CRM-PIPELINE-MODULE-SPEC.md §3: `contact_role_attributes` table has `party_contact_id` (FK to Party contacts). §8 Invariant #10: "`party_contact_id` must exist in Party." §5.2 Events Consumed: `party.contact.deactivated.v1 → detach from opportunities (nullify primary_party_contact_id)`. But there is no equivalent subscription or mechanism for write-time existence validation — no event subscription for `party.contact.created.v1`, no HTTP call specified, no consistency model named.

**Reasoning chain:**
- The spec correctly recognizes that CRM cannot take a Cargo path dependency on Party (boundary enforcement rule).
- The spec correctly handles the deactivation case (subscription to Party event).
- But the write-time existence check ("party_contact_id must exist in Party") is stated as an invariant with no enforcement mechanism.
- This is not an implementation detail — it requires choosing between: (A) synchronous HTTP call to Party at write time (creates availability dependency), (B) accept eventual inconsistency (write first, validate later), or (C) pre-load known Party contacts into CRM's own projection (event-sourced read model).

The spec enforces the reference on deactivation (event subscription) but not on creation (nothing specified). This asymmetry is a latent bug: you can write a `contact_role_attribute` row pointing to a non-existent Party contact, and the system has no path to detect or reject it.

**Severity:** High  
**Confidence:** 0.87  
**So What?** Specify the consistency model explicitly. Recommended: synchronous HTTP validation at write time (CRM calls `GET /api/party/contacts/:id` before writing a `contact_role_attributes` row). Document this as a synchronous cross-module call in the spec's §9 integration notes. This creates a mild availability dependency but is the simplest correct approach for an infrequent write operation.

---

### §F6 — Shop-Floor-Gates distributed hold enforcement has an undefined consistency model

**Evidence:** SHOP-FLOOR-GATES-MODULE-SPEC.md §8 Invariant #8: "Hold prevents operation start when active on that operation. Downstream — Production should check for active operation-scoped holds before allowing an operation to start. Platform Gates emits `hold.placed.v1`; Production is the enforcer, not Gates." §11 open questions: "Either Gates returns active holds via a GET endpoint; Production calls it. Either works; design detail for implementation bead."

**Reasoning chain:** This is described as a design detail, but it is actually a consistency model decision with different failure modes:

- **Option A (Production subscribes to hold events, maintains projection):** Eventual consistency. Race condition window where Gates places a hold, but Production hasn't consumed the event yet — an operation starts before the hold is enforced. In an aerospace shop, this is a quality escape.
- **Option B (Production calls Gates synchronously at operation-start):** Strong consistency within the operation-start transaction. Availability dependency on Gates. Correct for safety-critical environments.

"Either works" is incorrect for safety-critical manufacturing. They have different correctness guarantees. For an aerospace shop floor where a hold exists to prevent quality escapes, eventual consistency is not acceptable. For HuberPower, it might be fine. But the platform spec cannot say "either works" and leave it to each vertical's implementation bead to choose — that produces inconsistent enforcement across verticals using the same module.

A second issue: the spec lists `GET /api/shop-floor-gates/holds` and `GET /shop-floor-gates/work-orders/:wo_id/holds` but there is no endpoint specifically optimized for "does this operation have any active holds right now?" — which is the query Production needs. The existing list endpoint is general-purpose; the Production enforcement path needs a targeted query.

**Severity:** High  
**Confidence:** 0.82  
**So What?** Specify: "Production enforces holds via synchronous API call to Gates at operation-start time." Add `GET /api/shop-floor-gates/holds/active?work_order_id=X&operation_id=Y` to the spec. This call belongs in Production's implementation spec when written. Event subscriptions on Production's side are for UI notifications only, not enforcement.

---

### §F7 — CRM closed-won to Sales-Orders handoff creates a broken event loop for `sales_order_id` linkage

**Evidence:** CRM-PIPELINE-MODULE-SPEC.md §5.1: `crm_pipeline.opportunity.closed_won.v1` includes `sales_order_id (if set)`. §9: "The handoff flow (opp close-won → SO create) can be implemented as an event subscriber on Sales-Orders side or as a manual operator action." The `opportunities` table has a `sales_order_id` column. §5.2 (Events Consumed by CRM) contains no Sales-Orders event.

**Reasoning chain:** If the automated path is chosen (Sales-Orders subscribes to `crm_pipeline.opportunity.closed_won.v1` and auto-creates a SO), then:
1. CRM emits `opportunity.closed_won.v1` with `sales_order_id = null`
2. Sales-Orders creates a SO, emits `sales_orders.order.created.v1`
3. CRM needs to receive this event and update `opportunities.sales_order_id`

Step 3 requires CRM to subscribe to a Sales-Orders event. This subscription is absent from the spec. Without it, `opportunities.sales_order_id` is always null in the automated path. The only way it gets populated is the manual path — a human creates the SO and then updates the opportunity. The spec doesn't make this manual dependency explicit, leaving implementers to discover the gap when the field never populates.

**Severity:** Medium  
**Confidence:** 0.80  
**So What?** Clarify one of two options: (A) Declare the linkage is manual-only: `opportunities.sales_order_id` is set by an operator after creating the SO, not automatically. Remove the "event subscriber" option from §9. (B) If automated linking is desired, add `sales_orders.order.created.v1` to CRM's consumed events (§5.2) with behavior "if the SO was created from a closed-won opportunity matched via `correlation_id`, update `opportunities.sales_order_id`." Make the correlation mechanism explicit.

---

### §F8 — Polymorphic source-entity references have inconsistent enforcement across modules

**Evidence:**
- OUTSIDE-PROCESSING-MODULE-SPEC.md §8 Invariant #7: "If `source_entity_type = work_order`, `source_entity_id` must reference a platform Production work order that exists and belongs to the same tenant. For non-manufacturing source types, platform does not enforce FK."
- CUSTOMER-COMPLAINTS-MODULE-SPEC.md §2: `source_entity_type` + `source_entity_id` (nullable). §9: "Platform enforces nothing about the reference (it's a soft link across modules)."

Both modules use `source_entity_type + source_entity_id` (polymorphic reference). OP enforces the reference for `work_order` type via a synchronous Production API call. Complaints enforces nothing. This is the same pattern applied with two different consistency models, with no documented reason for the difference.

**Reasoning chain:** The OP spec is stricter because work orders are load-bearing (the OP's operational clock is tied to the work order). The complaints spec is looser because a complaint can exist without a resolvable source entity. But the consistency model choice was made implicitly by each spec author independently. There is no platform standard for "validated cross-module reference" vs "opaque cross-module reference." A third module adding this pattern in the future will make the same implicit choice with no guidance, producing further divergence.

**Severity:** Medium  
**Confidence:** 0.78  
**So What?** Add a brief section to CONTRACT-STANDARD.md defining: "Type-validated cross-module reference: the referencing module MUST make a synchronous call to verify existence at write time when `source_entity_type` maps to a platform module. Type-opaque cross-module reference: no validation required; the reference is stored as metadata. Each spec must declare which model it uses per reference field." This removes the per-spec implicit choice.

---

## 3. Risks Identified

| Risk | Severity | Likelihood | Root Cause |
|------|----------|------------|------------|
| SO `in_fulfillment` never transitions — orders stuck in `booked` | High | High | §F1: undefined trigger |
| OP `actual_cost_cents` always wrong (stuck at estimated) | High | High | §F2: no cost update mechanism |
| HuberPower manufacturing costing emits zero labor postings | Critical | High | §F3: broken event dependency |
| MRP and Kit Readiness implementations diverge over time | Medium | Medium | §F4: duplicate logic |
| Stale `contact_role_attributes` rows referencing non-existent Party contacts | Medium | Medium | §F5: asymmetric consistency |
| Hold placed → operation starts anyway before event propagates | High | Medium | §F6: undefined consistency model |
| `opportunities.sales_order_id` always null in practice | Low | High | §F7: broken event loop |
| Inconsistent source-entity validation surprises consumers | Low | Medium | §F8: no platform standard |

---

## 4. Recommendations

| ID | Priority | Effort | Finding | Action |
|----|----------|--------|---------|--------|
| R1 | P0 | Low | §F3 | Replace `shop_floor_data.labor.approved.v1` with `production.time_entry.approved.v1`; verify Production emits it |
| R2 | P0 | Low | §F1 | Define what triggers `booked → in_fulfillment`; clarify blanket release initial status |
| R3 | P1 | Med | §F2 | Remove `actual_cost_cents` from OP OR define the AP bill settlement → OP cost update mechanism with source-of-truth invariant |
| R4 | P1 | Low | §F6 | Specify synchronous Gates hold-check at Production operation-start; add `active holds` query endpoint |
| R5 | P1 | Low | §F5 | Specify synchronous Party contact validation on `contact_role_attributes` write |
| R6 | P2 | Med | §F4 | Merge MRP and Kit Readiness into one endpoint with `on_hand_source` parameter |
| R7 | P2 | Low | §F7 | Clarify whether SO linkage is manual or automated; add consumed event if automated |
| R8 | P3 | Low | §F8 | Add validated vs. opaque reference standard to CONTRACT-STANDARD.md |

---

## 5. New Ideas and Extensions

**Incremental:**
- Add `GET /api/shop-floor-gates/holds/active?work_order_id=X&operation_id=Y` to the Gates spec (needed for R4 regardless) — small targeted query endpoint.
- Add `parent_complaint_id` to the Customer-Complaints `complaints` table now (cheap column, no behavior) — prevents the "create a new complaint, lose the history link" problem when a complaint is re-opened after the customer pushes back on a resolution.

**Significant:**
- **Platform "soft reference" standard** (R8): Two paragraphs in CONTRACT-STANDARD.md prevent this class of inconsistency for all future modules. Low effort, high long-term value.
- **Unified BOM check endpoint** (R6): Collapsing MRP + Kit Readiness to one endpoint reduces surface area, reduces test burden, reduces future divergence.

**Radical:**
- **Signoff as a platform package (not a module-local table):** The signoff pattern already appears in Shop-Floor-Gates and is anticipated in Quality-Inspection. If it recurs in 3+ modules, consider `packages/signoff/` (a shared Rust library implementing the signoff data model and append-only write logic) that multiple modules import. This would require a formal shared-package approval per BOUNDARY-ENFORCEMENT.md policy but may be worth it. Flag for architecture review; do not act on now.

---

## 6. Assumptions Ledger

1. NATS is a platform-level event bus — events from Fireproof-local modules (SFDC) are NOT automatically on the same bus as platform module events.
2. "Sample data only" remains true through all implementation — no migration cost for schema changes.
3. HuberPower is a genuine manufacturing vertical that will want shop-floor-gates and manufacturing costing (the "cross-vertical test" assumes HuberPower is in scope for these).
4. The Production module currently has a time-entry approval flow with an associated event. If it doesn't, §F3's recommended fix requires adding it to Production first.
5. The shop-floor-data retirement decision is final per the MODES_CONTEXT_PACK.

---

## 7. Questions for Project Owner

1. **What triggers `booked → in_fulfillment` on a Sales Order?** Is it automatic (when Inventory confirms reservation, or when Production creates a work order for the SO line) or manual? This determines whether Sales-Orders needs an additional consumed event.

2. **Who is the source of truth for how much an Outside-Processing job costs — OP or AP?** If the vendor bills a different amount than estimated, which number is authoritative for reporting and GL purposes?

3. **Does the NATS event bus include Fireproof-local module events, or is it platform-modules-only?** This determines whether Fireproof's SFDC emitting a labor event is reachable by the platform Production module.

4. **For Shop-Floor-Gates holds in a safety-critical environment: is eventual consistency acceptable, or must Production call Gates synchronously before allowing an operation to start?** The answer is likely "synchronous for Fireproof/aerospace" — the spec should say so explicitly.

5. **Is the CRM `closed_won → SO` linkage expected to be automatic or manual?** If manual, the `sales_order_id` column exists only for human-populated metadata. If automatic, a consumed event must be added to the CRM spec.

---

## 8. Points of Uncertainty

- I cannot confirm from the specs whether Production currently emits `time_entry.approved.v1` or any equivalent. If it doesn't, §F3's fix requires a Production spec change beyond what this analysis covers.
- The Fireproof-local NATS architecture is not specified. It's possible Fireproof's services already publish onto the same NATS infrastructure as the platform. If so, the `shop_floor_data.labor.approved.v1` event might work for Fireproof specifically — but it still wouldn't work for HuberPower, and the event's `source_module` value would violate the platform envelope standard.
- It's unclear whether the barcode resolver returning `work_order` refs in the Inventory extension creates a long-term maintainability concern (every new platform entity type that can be barcoded requires an Inventory extension change) or whether the open `entity_type` field handles this gracefully.

---

## 9. Agreements and Tensions with Other Perspectives

**Likely agreements:**
- A Systems-Thinking analysis would probably also surface the broken event loop (§F7) and the distributed hold enforcement problem (§F6) — these are systems-level coupling issues.
- A Risk-Register analysis would flag §F3 as critical given it affects production billing for a paying customer.
- A Contracts/Boundaries analysis would likely agree on §F5 and §F8 — both are contract-enforcement gaps.

**Likely tensions:**
- An Occam's Razor / minimalism perspective might push back on §F4 (MRP vs Kit Readiness unification), arguing that the two endpoints serve different user workflows and should stay separate even if the computation overlaps. Counter: the computation overlap is the point — different query defaults do not justify different schemas and event contracts.
- A Migration-Pragmatism perspective might defend the `shop_floor_data.labor.approved.v1` reference (§F3) by saying "Fireproof will just emit this event under the right name." Counter: the spec should document this explicitly and provide a cross-vertical-compatible trigger rather than leaving HuberPower without a labor cost path.

---

## 10. Confidence

**Overall confidence: 0.82**

**Calibration note:** High confidence on §F1 (missing state transitions are verifiable from spec text), §F3 (broken event dependency is factually verifiable — the module was retired), and §F4 (duplicate computation is demonstrable by inspection). Lower confidence on §F2 and §F6, which involve domain judgments about where the "right" boundary is — these are arguments, not facts, and a domain expert with Fireproof operational knowledge might push back. The CRM findings (§F5, §F7) are at moderate confidence because the spec leaves room for "the implementation bead will sort it out" — this analysis argues that is the wrong deferral, but a pragmatist could reasonably disagree.

The failure mode I am guarding against for this mode (F5): terminating "5 Whys" chains too early at proximate causes. The findings above aim to reach genuine root causes (code-shape thinking, absent dependency audit, implicit consistency model choices) rather than stopping at "the spec doesn't say X."
