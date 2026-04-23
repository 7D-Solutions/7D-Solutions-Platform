# Modes of Reasoning Analysis — Fireproof→Platform Migration Specs

**Report date:** 2026-04-16
**Bead:** bd-ixnbs
**Synthesis by:** LavenderWaterfall (orchestrator / lead agent)
**Swarm:** 10 agents across 10 reasoning modes on project `bd-ixnbs-review`

---

## 1. Executive Summary

Ten reasoning modes independently analyzed the six specs plus plan doc. Cross-mode convergence was strong: 6 of 10 modes independently flagged the broken labor-event dependency; 6 of 10 flagged the undefined hold-enforcement mechanism; 4–5 modes converged on each of six other critical issues. The analysis found no showstopping architectural flaws in the overall migration approach, but surfaced **11 concrete spec-level gaps that must be resolved before implementation beads are decomposed.** Ten of the eleven are small edits (add field, add consumer event, add endpoint, clarify contract). One (aerospace vocabulary leakage) is larger and requires field-by-field review.

**Top 5 issues the agent swarm surfaced that I missed:**

1. **The Manufacturing Costing extension consumes `shop_floor_data.labor.approved.v1` — but `shop-floor-data` is retired.** HuberPower will have no labor-cost trigger at all. The extension is Fireproof-only as written.
2. **Hold enforcement is unowned.** Gates emits `hold.placed.v1` but neither Gates nor Production specifies how Production checks for active holds at operation start. "Design detail for implementation bead" in the spec is a punt, not a decision.
3. **AR has no consumer for `sales_orders.invoice.requested.v1`.** Billing handoff is unowned.
4. **Blanket-release creation has a classic over-commit race** under Postgres read-committed isolation. Not specified with `SELECT FOR UPDATE`.
5. **Outside-Processing state machine assumes round-trip material flow.** TrashTech hazwaste and RanchOrbit livestock processing cannot reach terminal states via designed path.

**Secondary convergences (4+ modes):** aerospace vocabulary leakage (verification checklist fields, signoff roles, CRM opp_type, complaint source `letter`), signoff-whitelist trap, polymorphic `source_entity_type` inconsistency, CRM→SO linkage broken, AR customer creation in identity chain unowned.

**Prioritized action list:** 11 P0/P1 spec edits before any implementation bead is written, plus one larger audit pass for aerospace vocabulary. Estimated total effort: low-medium. All edits are additive to existing specs — no spec restructuring needed.

---

## 2. Methodology

### Mode selection and axis coverage

Ten modes chosen for axis diversity and author-bias protection:

| Mode | Code | Category | Axes Covered |
|------|------|----------|--------------|
| Systems-Thinking | F7 | Causal | Ampliative, Descriptive |
| Root-Cause | F5 | Causal | Descriptive, Monotonic |
| Deductive | A1 | Formal | Non-ampliative, Monotonic |
| Adversarial-Review | H2 | Strategic | Multi-agent, Action |
| Failure-Mode | F4 | Causal | Action, Uncertainty |
| Edge-Case | A8 | Formal | Non-ampliative |
| Counterfactual | F3 | Causal | Ampliative, Belief |
| Perspective-Taking | I4 | Multi-Agent | Multi-agent, Adoption |
| Dependency-Mapping | F2 | Causal | Descriptive |
| Debiasing | L2 | Meta | Meta-reasoning |

**Axis coverage:** 4 of 7 taxonomy axes spanned. **Category coverage:** 5 of 12 (A, F, H, I, L). L2 Debiasing specifically assigned as author-calibration check (I authored all specs under review).

### Swarm execution

10 Rust-backed Claude + Codex agents via NTM, prompted with shared context pack, analyzing in parallel. All 10 produced substantive outputs (11KB–33KB each). Monitoring cron checked progress every 7 minutes; stopped when all outputs landed at ~60 minutes.

### Synthesis protocol

Applied the triangulation protocol from `SYNTHESIS_METHODOLOGY`:
- **KERNEL:** finding agreed on by 3+ modes via **distinct evidence methodologies**
- **SUPPORTED:** 2 modes
- **HYPOTHESIS:** 1 mode (valuable if evidence is strong)
- **DISPUTED:** modes actively disagree

Operator cards applied: ✂ Kill Thesis on every KERNEL, 🏗 Identity Check (no recommendations to abstract core substrate pass through), 👷 Senior Engineer Gut Check on recommendations, ΔE Evidence Delta to confirm findings survive evidence removal.

---

## 3. Convergent Findings (KERNEL — 3+ modes, distinct methodologies)

### K1 — Manufacturing Costing labor event is broken (6 modes converge)

**Supporting modes:** F7, I4, F5, F4, F2, F3
**Evidence methodologies (distinct):** systems-integration cross-reference (F7), implementation-agent lens (I4), decision-dependency audit (F5), failure-mode analysis (F4), dep-graph edge inspection (F2), counterfactual on HuberPower labor flow (F3)
**Confidence:** 0.97

`PLATFORM-EXTENSIONS-SPEC.md §4` lists `shop_floor_data.labor.approved.v1` as a consumed event for manufacturing cost posting. `shop-floor-data` is **retired** from platform scope per `bd-ixnbs-fireproof-platform-migration.md`. There is no platform producer for this event. For Fireproof, the Fireproof-local labor-capture code could publish to shared NATS under a named subject if that mechanism is agreed, but the subject name would then be wrong (`shop_floor_data.*` is named after a retired module). For HuberPower — the second manufacturing vertical — there is no equivalent local kiosk module. The extension's labor cost will be $0 for HuberPower at launch.

**Action (P0):** Replace `shop_floor_data.labor.approved.v1` with `production.time_entry.approved.v1` (platform-neutral, produced by Production when any vertical approves a labor record — whether captured via kiosk, direct entry, or mobile app). Verify that Production's time-entry flow has an approval step; if not, add one (small addition to an existing module). Document that Fireproof's local SFDC emits directly to Production's time-entry API rather than to a Fireproof-named event subject.

---

### K2 — Hold enforcement mechanism is unspecified (6 modes converge)

**Supporting modes:** F7, I4, F5, F4, F2, H2
**Evidence methodologies (distinct):** systems seam view (F7), enforcement-bead ownership analysis (I4), consistency-model analysis (F5), FMEA (F4), dep-graph (F2), adversarial self-verify angle (H2)
**Confidence:** 0.94

`SHOP-FLOOR-GATES-MODULE-SPEC.md §8 Invariant 8`: "Production should check for active operation-scoped holds before allowing an operation to start… Either works; design detail for implementation bead." The two options (synchronous GET vs. event-subscribed cache) have **different correctness guarantees** — synchronous is strong-consistency (correct for aerospace safety-critical shops); event-subscribed is eventual-consistency (a hold placed milliseconds before operation-start may not be enforced). The spec defers the choice. Neither module lists Production as a consumer of `hold.placed.v1`; neither has a GET endpoint optimized for "is this operation held right now?"

**Action (P0):** Choose synchronous GET. Add `GET /api/shop-floor-gates/work-orders/:wo_id/operations/:op_id/active-holds` to Gates spec. Add corresponding consumer contract to Production's operation-start endpoint spec: Production calls this before allowing an operation to start. Record the decision in both specs.

---

### K3 — AR has no consumer for `sales_orders.invoice.requested.v1` (3 modes converge)

**Supporting modes:** A1, F2, F4 (supplemental)
**Evidence methodologies (distinct):** deductive contract check (A1), dep-graph unowned-edge (F2), FMEA (F4)
**Confidence:** 0.95

Sales-Orders emits `sales_orders.invoice.requested.v1` when a line ships. The existing AR module spec (`AR-MODULE-SPEC.md §5.2`) has no consumer for this event. Every vertical will invent its own billing bridge — some invoicing per shipped line, others aggregating at order level — with no platform consistency.

**Action (P0):** Add `sales_orders.invoice.requested.v1` to AR's consumed-events section with specified behavior: create a draft AR invoice, idempotent on `sales_order_id + line_id`. Specify the bundling unit (per-line vs. per-order). This also requires a small amendment to AR-MODULE-SPEC.md (an existing spec), which is out of scope for this migration wave but required for the Sales-Orders extension to land.

---

### K4 — CRM→SO handoff and CRM→AR customer creation chain is unowned (5 modes converge)

**Supporting modes:** F7, F5, F2, F4, A1
**Evidence methodologies (distinct):** tri-modal identity analysis (F7), broken event loop analysis (F5), dual identity dep-graph (F2), failure contract analysis (F4), deductive contract-gap (A1)
**Confidence:** 0.92

Three concurrent problems around customer identity:

1. **Lead conversion creates a Party company but nothing creates the AR customer.** CRM-Pipeline spec §9 explicitly delegates AR customer creation to "vertical orchestration." Sales-Orders requires both `party_id` and `customer_id`. Every vertical will independently build the bridge.
2. **Opportunity close-won → SO creation is ambiguous.** CRM spec says "event subscriber or manual operator action; either works." SO spec has no opportunity_id field. `opportunities.sales_order_id` is nullable with no defined update path.
3. **`party.party.deactivated.v1` is not consumed by Sales-Orders.** A deactivated party can still receive new orders.

**Action (P0):** Three small spec additions:
- Add `sales_orders.invoice.requested.v1` and `party.party.deactivated.v1` to AR + Sales-Orders consumed events respectively.
- Add `opportunity_id` (nullable, opaque ref to CRM) on `sales_orders` table.
- Add `sales_orders.order.booked.v1` with `opportunity_id` payload to CRM's consumed events with behavior "if payload.opportunity_id matches, update opportunities.sales_order_id."
- Add AR subscription to `crm_pipeline.lead.converted.v1` to auto-create draft AR customer record (or document as a required vertical-side step with explicit trigger).

---

### K5 — Blanket-order release creation has an over-commit race (3 modes converge)

**Supporting modes:** F3, H2, F2
**Evidence methodologies (distinct):** counterfactual concurrency analysis (F3), adversarial stress angle (H2), dep-graph dual-write (F2)
**Confidence:** 0.93

`SALES-ORDERS-MODULE-SPEC.md §8 Invariant 5` says `released_qty` is "maintained by triggers or application-level update on release create/cancel" — without specifying row-level locking. Under Postgres read-committed isolation (the default), two concurrent POST requests to `/api/sales-orders/blankets/:id/lines/:line_id/releases` can both read `released_qty = 80`, both check remaining against `committed_qty = 100`, and both insert — over-releasing the blanket commitment. The invariant is deterministic-violable under normal concurrent load. This matters for Fireproof's actual use case (long-term contracts with frequent releases).

**Action (P0):** Add explicit invariant to the spec: "Release creation MUST acquire `SELECT … FOR UPDATE` on the parent `blanket_order_lines` row before checking and updating `released_qty`. Application-level update only; no DB trigger." Add idempotency key to `blanket_order_releases`. Wrap release + child SO creation in a single atomic transaction.

---

### K6 — OP state machine assumes round-trip material flow; breaks for TrashTech and RanchOrbit (4 modes converge)

**Supporting modes:** I4, F5, L2, F3
**Evidence methodologies (distinct):** vertical-persona analysis (I4), domain-boundary root-cause (F5), availability-bias debiasing (L2), counterfactual (F3)
**Confidence:** 0.88

The state machine (`draft → issued → shipped_to_vendor → at_vendor → returned → review_in_progress → closed`) has no terminal path without a `return_event`. TrashTech sends hazwaste for destruction — it does not return. RanchOrbit sends livestock for processing — returns come back as a different unit-of-measure (head → pounds). The quantity invariant `SUM(received) ≤ SUM(shipped)` fails on UoM transformations (48000 lbs ≤ 100 head is false). The `cert_ref` field encodes AS9100 certificate-of-conformance assumptions not applicable to hazwaste manifests or livestock health certs.

**Action (P1):** Add `disposition_type` field on `op_orders` (canonical: `round_trip` / `destruction` / `transformation`). When `disposition_type != round_trip`, allow `at_vendor → disposition_confirmed → closed` path via a `disposition_certificate` record instead of a `return_event`. Scope invariant 3 (quantity ≤ ship quantity) to same-UoM cases only. Keep `cert_ref` field but document it as optional and opaque (not aerospace-specific).

---

### K7 — Polymorphic source_entity_type is inconsistent across modules (4 modes converge)

**Supporting modes:** F7, F5, L2, A8
**Evidence methodologies (distinct):** cross-spec vocabulary scan (F7), consistency-model analysis (F5), embedded-vocabulary debiasing (L2), existence-check analysis (A8)
**Confidence:** 0.87

Three modules define string-valued entity-type fields with different conventions and enforcement:
- `op_orders.source_entity_type`: examples include `work_order/collection_batch/livestock_batch/standalone`. Platform enforces FK only for `work_order`.
- `complaints.source_entity_type`: nullable, examples `sales_order/shipment/invoice/service_visit`. Platform enforces nothing.
- `signoffs.entity_type`: canonical whitelist (`work_order/operation/traveler_hold/operation_handoff/operation_start_verification`). Enforces FK? Unclear — existence-check is not specified.

Each spec author will implement differently. Subscribers matching on `source_entity_type = 'sales_order'` will miss events where another module used `'so'` or `'order'`. The illustrative examples in OP (`collection_batch`, `livestock_batch`) give vertical-specific terms false canonical status.

**Action (P1):** Publish `contracts/entity-types.v1.json` with a canonical cross-module entity-type vocabulary (at minimum: `work_order, operation, sales_order, blanket_order, blanket_release, invoice, shipment, lot, serial, party, contact`). Each module using an entity-type field references this vocabulary. Add a platform standard: "type-validated cross-module reference requires synchronous existence check at write time for canonical values; all other values are opaque." Document in `CONTRACT-STANDARD.md`.

---

### K8 — Sales-Orders state machine has transitions without triggers (4 modes converge)

**Supporting modes:** F5, F4, L2, F7 (supplemental on async consistency)
**Evidence methodologies (distinct):** root-cause state analysis (F5), FMEA (F4), confirmation-bias debiasing (L2), systems async-consistency (F7)
**Confidence:** 0.90

`SALES-ORDERS-MODULE-SPEC.md §6.1` defines `booked → in_fulfillment` with **no trigger** — no endpoint, no event, no defined rule. Similarly, blanket release `pending → released` has no endpoint path (only create + ship). The open-questions section recommends adding a `stock_confirmation_status` column but does not add it to the data model.

**Action (P0):** Before implementation: specify the trigger for `booked → in_fulfillment` (recommendation: auto-advance when `inventory.reservation.confirmed.v1` arrives for all lines; alternative: explicit operator `start-fulfillment` endpoint). Decide whether blanket releases initialize as `pending` (requiring a `release` action) or directly as `released` (combining create and release). Add `stock_confirmation_status` (`pending / confirmed / rejected / waived`) column to `sales_order_lines` if the async reservation dance matters.

---

### K9 — Aerospace domain vocabulary leaked into platform modules despite user ruling (4 modes converge)

**Supporting modes:** L2, I4, F5, A1 (supplemental via `service_type` treatment)
**Evidence methodologies (distinct):** debiasing field-by-field scan (L2), HuberPower perspective (I4), code-boundary root-cause (F5), deductive consistency check (A1)
**Confidence:** 0.88

Despite the explicit ruling "no ISO-like QMS features for platform," aerospace vocabulary leaked into multiple platform module specs:
- **`operation_start_verifications`**: three hardcoded boolean columns (`drawing_verified`, `material_verified`, `instruction_verified`) are literally the AS9100 §8.5.1 setup-packet review checklist. HuberPower's power-gen pre-operation checks (torque cal, environmental params, lockout status) don't map.
- **`signoffs.role` canonical set**: `quality/engineering/supervisor/operator/planner/material` is an aerospace shop org chart. HuberPower commissioning engineer, TrashTech compliance officer, RanchOrbit livestock inspector don't fit.
- **`op_return_events.cert_ref`**: certificate-of-conformance is a formal aerospace/defense requirement. TrashTech receives hazwaste manifests, not CoCs.
- **`complaints.source = letter`**: "customer concern letter" is AS9100 formal document terminology.
- **`op_re_identifications` table**: aerospace material re-identification (raw bar-stock → heat-treated AMS 2770) presented as cross-vertical but the formalism is aerospace-specific.
- **`crm_pipeline.opp_type` canonical**: `engineering_change` and `prototype` are aerospace-flavored.

**Action (P1):** Field-by-field audit pass. Options: (a) convert `operation_start_verifications` hardcoded columns into a tenant-configurable checklist table with aerospace defaults for Fireproof tenants; (b) add `inspector` and `commissioning` to canonical signoff roles; (c) reclassify `cert_ref` as optional/opaque; (d) remove `letter` from complaint sources or rename to `formal_written`; (e) replace CRM `engineering_change`/`prototype` with neutral `project`/`service_agreement`; (f) remove `collection_batch`/`livestock_batch` examples from OP `source_entity_type` spec text.

---

### K10 — OP re-identification has no Inventory consumer, breaking lot genealogy (3 modes converge)

**Supporting modes:** F4, F5, F7 (via dep-graph)
**Evidence methodologies (distinct):** FMEA missing-consumer (F4), root-cause open-question analysis (F5), dep-graph unowned-edge (F7 adjacent)
**Confidence:** 0.89

OP emits `outside_processing.re_identification.recorded.v1` but Inventory extension's consumed events don't include it. The OP spec's open question states "Inventory creates a child lot, with lot_genealogy recording parent→child" as a recommendation, not a specified contract. Without the consumer, OP re-identification produces an orphaned record — the parent→child lot linkage in Inventory simply doesn't happen. For aerospace (Fireproof) this is traceability-critical.

**Action (P0):** Add `outside_processing.re_identification.recorded.v1` to Inventory's consumed events (either in the remnant-tracking extension or as its own Inventory extension line item). Specified behavior: create child lot via lot-split API, record genealogy edge (`parent = old_lot_id`, `child = new_lot_id`, `source = outside_processing`).

---

### K11 — Vendor disqualification doesn't propagate to in-flight work (3 modes converge)

**Supporting modes:** F7, A8, F4 (supplemental)
**Evidence methodologies (distinct):** systems emit-but-no-consume analysis (F7), edge-case in-flight-change analysis (A8), FMEA supplemental
**Confidence:** 0.83

`PLATFORM-EXTENSIONS-SPEC.md §7` produces `ap.vendor.disqualified.v1` but no module in the new spec set consumes it. An OP order already `shipped_to_vendor` or `at_vendor` will continue its lifecycle even after the vendor is disqualified. For Fireproof compliance (supplier qualification traceability), this is a silent gap.

**Action (P1):** Add `ap.vendor.qualification_changed.v1` to OP's consumed events. Specified behavior: log a warning activity on any active OP order for the affected vendor; surface in OP list views with a disqualified-vendor flag. Full recall workflow (auto-hold, etc.) can be a Fireproof overlay.

---

## 4. Supported Findings (2 modes each)

### S1 — `actual_cost_cents` on OP orders has no update mechanism (F5, H2)

The field exists on `op_orders` but no event, endpoint, or process updates it from AP bill settlement. Recommendation: either remove the field (AP owns cost truth; OP holds PO ref only) or define the AP→OP cost update event with a source-of-truth invariant. **P2, Low effort.**

### S2 — Training completion → competence assignment is non-idempotent (A8, F4)

`resulting_competence_assignment_id` is set when the side-effect API call succeeds. No idempotency key, no retry safety, no outbox. Recommendation: idempotent on `(assignment_id, completed_at)` or write the resulting competence assignment through outbox with uniqueness constraint. **P2, Medium effort.**

### S3 — Complaint `visible_to_customer` enables transparency suppression (H2, A8)

The `responded` transition requires a `customer_communication` entry, but nothing requires `visible_to_customer = true`. A vertical can record hidden communications to formally close a complaint without the customer seeing evidence. Recommendation: add invariant — `responded` transition requires at least one `customer_communication` entry with `visible_to_customer = true` (or explicit policy override with reason + audit event). **P1, Low effort.**

### S4 — Pipeline stages are mutable enough to rewrite active-opportunity meaning (A8, F3)

Tenants can reorder or flip `is_terminal`/`is_win` on active stages without a migration step. Opens and closed opportunities reference mutable stage metadata. Recommendation: make stage changes versioned, or require migration path for open opportunities before stage metadata can be rewritten. Add nullable `stage_type` canonical field to unlock cross-tenant pipeline analytics. **P2, Low effort.**

### S5 — Sales-Orders allows ad-hoc lines that reservation cannot process (A1)

Only A1 caught this specifically: `item_id` nullable on lines, but reservation event requires `item_id`. Recommendation: add `line_kind` column (`stock | service | ad_hoc`) — only `stock` lines emit reservation requests. **P1, Low effort.**

### S6 — Blanket release label API scope mismatches schema (A1)

The label API endpoints accept `scope = release` but no `release_status_labels` table exists. Recommendation: either add the table or remove `release` scope from API. **P2, Low effort.**

### S7 — OP return events don't reference specific outbound shipments (A1, F3, F4)

Outbound side has `shipping_reference`; inbound side doesn't. Partial returns, mixed-condition returns, and multi-round cycles can't be reconciled to specific outbound shipments. Recommendation: add `outbound_shipment_ref` on `op_return_events`; include in event payload; add `round_number` to ship and return events for multi-round OP cycle auditability. **P1, Low effort.**

### S8 — Verification state machine has hidden intermediate state (F4, L2 adjacent)

The `pending → verified | rejected` state machine doesn't distinguish "operator hasn't confirmed yet" from "operator confirmed, awaiting verifier." Invariant 5 enforces the guard but the state machine hides it. Recommendation: revise to `awaiting_operator → awaiting_verifier → verified | rejected`. **P2, Low effort.**

### S9 — Daily sweep jobs have no infrastructure owner (F4, L2 supplemental on missing sections)

Five+ daily sweeps across three modules, no spec defines: who runs the sweep, what happens on partial-day failure, idempotency, batch size, alerting. Recommendation: add "Sweep Infrastructure" addendum covering all modules (pg_cron + processed_up_to_date watermark + batch size + alerting). **P2, Low effort.**

### S10 — Kit Readiness has no specified failure mode under Inventory unavailability (F3 unique + extension of F7 asymmetry finding)

Kit Readiness queries Inventory synchronously. If Inventory is down: fail-closed blocks production starts (24/7 manufacturing stoppage); fail-open silently passes without verifying availability. Neither is specified. Recommendation: fail-open with `check_unavailable` status; callers (Production) treat as soft warning. **P2, Low effort.**

---

## 5. Unique Insights by Mode (single-mode findings worth capturing)

### L2 (Debiasing) — Planning fallacy on "small extensions"

- **Manufacturing Costing without overhead ≠ manufacturing cost.** GAAP/IFRS manufacturing cost has three components (direct materials, direct labor, overhead). v0.1 deliberately excludes overhead. Calling it "manufacturing costing" creates false expectations. **Recommendation:** rename to "Direct Cost Accumulation" OR define a v0.1 flat-rate overhead allocation method.
- **MRP without lead-time-phasing ≠ MRP.** Current v0.1 is single-point explosion. The "when" (scheduling answer) is the point of MRP. **Recommendation:** rename to "BOM Net Requirements" until time-phased version lands.
- **AP Supplier Eligibility as a flag ≠ qualification process.** ISO 9001 supplier qualification is documented procedure. The platform delivers boolean + notes. Both Fireproof and HuberPower will build local qualification workflows, diverging. **Recommendation:** rename to "Vendor Approval Gate" OR scope a proper process with reviewer/criteria/evidence.

### L2 — CRM-Pipeline cross-vertical test not verified

Only Fireproof clearly needs CRM pipeline. HuberPower (in-house manufacturing for parent org) likely has no outbound sales funnel. TrashTech uses external CRM (Salesforce/HubSpot). RanchOrbit sells via auctions/commodity transactions. The cross-vertical case was asserted, not demonstrated. **Recommendation:** Confirm with HuberPower and RanchOrbit before implementation beads. May eliminate a full module from scope.

### H2 (Adversarial) — CRM owner_id mutable, enables opportunity takeover

`PUT /opportunities/:id` updates non-stage fields including `owner_id`. A rep can silently claim another rep's deal. **Recommendation:** `owner_id` change only through dedicated `reassign` endpoint with reason, actor, previous owner, optional approval policy.

### H2 — Sales-Orders booking has no payability gate

Booking triggers reservation and shipment work with no credit check. Orders for customers with known payment risk can be booked freely. **Recommendation:** add `book_precheck` step (AR credit status check or tenant policy default-deny + explicit override). Interacts with AR non-goal; may be out of scope intentionally — but warrants explicit declaration.

### I4 — Overlay bootstrapping protocol undefined

Three specs delegate AS9100 specifics to "Fireproof overlay service" without specifying: SDK usage (ModuleBuilder or standalone), NATS subject namespace for Fireproof-local events, join protocol for empty overlay rows, auth model, bead ownership. RoseElk can't write frontend beads that combine platform record + overlay data without this. **Recommendation:** produce a one-page "Overlay Service Pattern" doc covering these questions before RoseElk's beads unblock.

### I4 — Typed SDK client stubs are a blocking dependency for RoseElk's rewiring

Fireproof rewires to typed clients (`platform_client_sales_orders::*`). These clients are generated after the module is built. The standard decomposition puts SDK client last. RoseElk's rewiring beads depend on SDK stubs. **Recommendation:** mark typed client stubs explicitly as blocking dependencies; mail RoseElk the dependency graph.

### F7 — Overlay service proliferation

Three+ Fireproof overlay services implied (NCR overlay, AS9100 verification overlay, customer-concern-letter overlay). Without a documented pattern, three teams will design them inconsistently. **Recommendation:** one overlay-pattern doc prevents this.

### F4 — Multi-round OP cycles have no round demarcation

When OP goes through rejection-and-rework, second-round ship/return events append to the same `op_order_id` without a `round_number` column. Reconstructing audit trails requires timestamp inference. For AS9100, rejection cycle counts are a traceability data point. **Recommendation:** add `round_number` to ship/return events and payloads.

### A1 — `service_type` treated as both business discriminator and free text

Spec says `service_type` is open text; events publish `service_type` prominently. Free text ≠ reliable integration key. **Recommendation:** freeze to tenant-managed canonical codes OR split code vs display label.

### A8 — Complaint `due_date` goes stale after severity changes

Auto-calculated on triage; general PUT allows severity change; no rule recomputes `due_date`. Overdue sweep becomes wrong. **Recommendation:** explicit recompute rule on severity change, or freeze SLA-feeding fields once triage completes.

---

## 6. Risk Assessment

| Risk | Severity | Likelihood | Kernel Finding |
|------|----------|------------|----------------|
| Manufacturing Costing shows $0 labor for HuberPower; zero for Fireproof unless local bridging agreed | Critical | Certain if not fixed | K1 |
| Operations start despite active holds (safety invariant unenforced) | High | Likely under parallel decomposition | K2 |
| Sales-Orders invoice handoff fragments by vertical; AR doesn't create invoices | High | Likely | K3 |
| Customer identity chain diverges across verticals; orphaned Party, duplicate AR customers | High | Likely | K4 |
| Blanket-release over-commit race under concurrent load | High | Likely | K5 |
| TrashTech/RanchOrbit cannot close OP orders; stuck at `at_vendor` | High | Certain for those verticals | K6 |
| Cross-module event subscriptions silently miss events due to entity-type string divergence | Medium | Likely over time | K7 |
| Sales orders stall in `booked`; blanket releases stall in `pending` | High | Certain if not specified | K8 |
| HuberPower cannot adopt shop-floor-gates verification checklist without rework | High | High | K9 |
| Aerospace AS9100 lot genealogy broken after OP re-identification | High | Certain on first re-ID | K10 |
| Active OP orders continue for disqualified vendors; silent compliance gap | Medium | Possible | K11 |

---

## 7. Recommendations (prioritized)

### P0 — Resolve before any implementation bead is decomposed

| # | Action | Finding | Effort |
|---|--------|---------|--------|
| R1 | Replace `shop_floor_data.labor.approved.v1` → `production.time_entry.approved.v1` in Manufacturing Costing consumed events. Verify/add time-entry approval step in Production. | K1 | Low |
| R2 | Specify synchronous hold-check: add `GET /api/shop-floor-gates/work-orders/:wo_id/operations/:op_id/active-holds` to Gates; document Production consumer behavior. | K2 | Low |
| R3 | Add `sales_orders.invoice.requested.v1` consumer to AR with idempotent draft-invoice creation. Specify line-vs-order bundling. | K3 | Low (+ small AR spec edit) |
| R4 | Define the CRM→AR→SO customer identity chain: add AR subscription to `crm_pipeline.lead.converted.v1` (auto-draft AR customer); add `opportunity_id` on `sales_orders`; add `sales_orders.order.booked.v1` consumer on CRM for linkage update; add `party.party.deactivated.v1` consumer on Sales-Orders. | K4 | Low |
| R5 | Add `SELECT … FOR UPDATE` requirement to blanket-release creation spec; add idempotency key; wrap release + child SO creation in atomic transaction. | K5 | Low |
| R6 | Specify triggers for `booked → in_fulfillment` and blanket-release `pending → released`. Add `stock_confirmation_status` column to `sales_order_lines`. | K8 | Low |
| R7 | Add `outside_processing.re_identification.recorded.v1` consumer to Inventory extension with child-lot creation behavior. | K10 | Low |

### P1 — Resolve before bead decomposition; can proceed in parallel with P0 drafting

| # | Action | Finding | Effort |
|---|--------|---------|--------|
| R8 | Add `disposition_type` field on `op_orders` (`round_trip / destruction / transformation`); branch state machine for non-round-trip flows; scope quantity invariant to same-UoM cases. | K6 | Medium |
| R9 | Publish `contracts/entity-types.v1.json` canonical vocabulary; document validated-vs-opaque reference standard in CONTRACT-STANDARD.md. | K7 | Low |
| R10 | Aerospace vocabulary audit pass: checklist-ify `operation_start_verifications` columns; add `inspector` + `commissioning` signoff roles; reclassify `cert_ref` as opaque; rename `letter` complaint source; replace aerospace-specific CRM opp_types. | K9 | Medium |
| R11 | Add `ap.vendor.qualification_changed.v1` consumer to OP with flag/warning behavior. | K11 | Low |
| R12 | Add visible-customer-communication invariant for `responded` transition in complaints. | S3 | Low |
| R13 | Split ad-hoc Sales-Orders lines with `line_kind` column; only `stock` lines emit reservations. | S5 | Low |
| R14 | Add return-to-shipment reference on `op_return_events`; add `round_number` for multi-round cycles; include in event payloads. | S7 | Low |
| R15 | Restrict CRM `owner_id` mutation to dedicated `reassign` endpoint with reason + actor. | H2 unique | Low |
| R16 | Produce a one-page "Fireproof overlay service pattern" doc covering SDK usage, NATS subjects, join protocol, bead ownership. | I4 unique | Medium |

### P2 — Address before first customer demo; can be late-stage bead work

| # | Action | Finding | Effort |
|---|--------|---------|--------|
| R17 | Rename "Manufacturing Costing" to "Direct Cost Accumulation" OR define v0.1 flat-rate overhead. | L2 unique | Low |
| R18 | Rename MRP extension to "BOM Net Requirements" OR deliver time-phased v0.1. | L2 unique | Low |
| R19 | Rename AP Supplier Eligibility to "Vendor Approval Gate" OR scope qualification process. | L2 unique | Low |
| R20 | Confirm CRM-Pipeline cross-vertical need with HuberPower and RanchOrbit. If no, pull CRM out of platform scope. | L2 unique | Low (research) |
| R21 | Add "Sweep Infrastructure" addendum covering pg_cron + idempotency + batch + alerting across affected modules. | S9 | Low |
| R22 | Remove `actual_cost_cents` from OP OR define AP→OP cost update mechanism. | S1 | Low |
| R23 | Add `release_status_labels` table OR remove `release` from label API. | S6 | Low |
| R24 | Idempotent training-completion → competence-assignment write (outbox pattern). | S2 | Medium |
| R25 | Version pipeline stages OR require migration path for open opportunities on stage changes. Add nullable `stage_type` canonical for cross-tenant reporting. | S4 | Low |
| R26 | Revise verification state machine to expose `awaiting_operator → awaiting_verifier`. | S8 | Low |
| R27 | Specify Kit Readiness failure mode under Inventory unavailability (fail-open with `check_unavailable`). | S10 | Low |
| R28 | Recompute complaint `due_date` on severity change OR freeze SLA fields after triage. | A8 unique | Low |
| R29 | Add overlay-service scope decision to plan doc; document typed SDK client stubs as blocking dependencies for Fireproof rewiring beads. | I4 unique | Low |

### P3 — Track but defer; may emerge as real needs later

- Add `book_precheck` / credit-check gate (likely AR-side, interacts with existing AR scope).
- Make two-person verification default policy with tenant opt-out and audit.
- Add hold TTL / escalation / emergency-release protocol for hold-as-DoS mitigation.
- DLQ + replay contract for critical events (depends on SDK baseline).
- Extract signoff as platform-level attestation service once 3+ modules need it.
- Cost-ledger module vs. per-module cost extensions (defer until overhead allocation complexity is understood).

---

## 8. New Ideas and Extensions Worth Flagging

**Incremental (zero-cost now):**
- `opportunity_id` column on `sales_orders` (one nullable column, future-proofs linkage).
- `round_number` on `op_ship_events` / `op_return_events` (one int column, major audit value).
- `parent_complaint_id` on complaints (re-open after customer pushback).
- `disposition_type` on `op_orders` (unlocks TrashTech/RanchOrbit).
- `stage_type` canonical on `pipeline_stages` (cross-tenant analytics unlock).

**Significant:**
- **Verification Checklist Config Table** (replace hardcoded booleans). Fireproof seeds the AS9100 three; HuberPower seeds its own.
- **Overlay Service Pattern doc** (unblocks RoseElk, establishes template for HuberPower overlays).
- **Platform `production.time_entry.approved.v1`** as a canonical contract any vertical's labor capture can emit under.

**Radical (consider but defer):**
- **Cross-Module Transition Guard Library** (shared guard functions for state+permission+provenance).
- **Platform Entity Graph** (structured registry of first-class cross-module IDs).
- **Customer Lifecycle Orchestrator** (explicit module for lead→party→AR→SO progression).
- **Platform Integrity Graph** (background validator of causal chain completeness).

---

## 9. Assumptions and Open Questions

### Assumptions this analysis rests on

1. NATS event bus is shared across platform modules only; Fireproof-local services don't publish to platform NATS by default.
2. Postgres deployment uses default read-committed isolation.
3. Production module currently has time entries (catalog shows this) but may lack explicit approval step.
4. "Sample data only, no ETL" remains true through implementation.
5. HuberPower + TrashTech + RanchOrbit are at roughly equal planning maturity to Fireproof. If some are further along, their real workflow requirements should update the "cross-vertical" claims.
6. AR module's `POST /api/ar/customers` is callable service-to-service.
7. Typed SDK clients are either auto-generated or hand-written but treated as module artifacts requiring a bead.

### Questions for user decision

1. **R1 precondition:** Does Production have a `time_entry.approved.v1` event, or is the approval step an add? If add, acceptable to scope it into the manufacturing-costing extension bead?
2. **R2 precondition:** Synchronous hold-check confirmed, or is eventual-consistency cache acceptable for non-safety-critical verticals?
3. **R3 precondition:** Should AR invoice bundling be per-shipped-line or per-order? The answer changes AR's consumer behavior.
4. **R4 precondition:** Is AR customer creation intended to be platform-automatic (on lead conversion) or always a vertical-explicit step?
5. **R8 precondition:** TrashTech hazwaste is real use case? RanchOrbit livestock processing is real? (Confirms severity of K6.)
6. **R17/R18/R19 scope decisions:** Rename or rescope each?
7. **R20:** Confirm CRM-Pipeline cross-vertical need with HuberPower + RanchOrbit before implementation.

---

## 10. Confidence Matrix

| Finding | Modes | Confidence | Methodological diversity |
|---------|-------|------------|--------------------------|
| K1 Labor event broken | 6 | 0.97 | ★★★★★ (6 distinct lenses) |
| K3 AR invoice handoff | 3 | 0.95 | ★★★★☆ |
| K2 Hold enforcement | 6 | 0.94 | ★★★★★ |
| K5 Blanket race | 3 | 0.93 | ★★★★☆ |
| K4 CRM/AR/Party identity | 5 | 0.92 | ★★★★★ |
| K8 Missing state triggers | 4 | 0.90 | ★★★★☆ |
| K10 OP re-ID no consumer | 3 | 0.89 | ★★★★☆ |
| K9 Aerospace vocab leakage | 4 | 0.88 | ★★★★☆ |
| K6 OP round-trip assumption | 4 | 0.88 | ★★★★☆ |
| K7 entity_type polymorphism | 4 | 0.87 | ★★★★☆ |
| K11 Vendor disq propagation | 3 | 0.83 | ★★★☆☆ |

Methodological diversity check per ✂ Kill Thesis strengthened: each KERNEL finding was verified against a distinct evidence methodology from at least 3 modes (not just 3 modes citing the same evidence). K1 through K4 pass this strongly; K11 passes but with less diversity.

---

## 11. Contribution Scoreboard

| Mode | Code | Unique Findings | Total Findings | Evidence Quality | Contribution Score |
|------|------|-----------------|----------------|------------------|--------------------|
| Systems-Thinking | F7 | 2 (tri-modal identity, entity-type vocabulary) | 7 | High (multi-spec cross-reference) | 0.92 |
| Debiasing | L2 | 4 (planning fallacy, CRM cross-vertical test, over-applied AR template, authority bias) | 8 | High (field-level evidence) | 0.95 |
| Failure-Mode | F4 | 3 (sweep infrastructure, multi-round demarcation, verification state machine) | 8 + 4 supplemental | High (FMEA rigor) | 0.93 |
| Perspective-Taking | I4 | 3 (overlay bootstrapping, typed client blocking dep, TrashTech hazwaste one-way flow) | 8 | High (persona-specific) | 0.91 |
| Counterfactual | F3 | 2 (blanket race specific, tenant-defined stage reporting gap) | 8 | High (alternative-design tested) | 0.89 |
| Root-Cause | F5 | 1 (MRP + Kit Readiness duplicate computation) | 8 | Medium-High | 0.85 |
| Dependency-Mapping | F2 | 1 (blanket release dual-write transaction) | 8 | High (dep-graph concrete) | 0.86 |
| Adversarial | H2 | 3 (opportunity takeover, complaint transparency suppression, book precheck) | 8 | High (contract-level attacks) | 0.87 |
| Edge-Case | A8 | 2 (due_date recompute, pipeline mutability) | 6 | Medium (spec text only) | 0.78 |
| Deductive | A1 | 2 (ad-hoc line reservation mismatch, release label scope mismatch) | 6 | High (contract deductions) | 0.81 |

**Highest-value contributors by contribution score:** L2 (debiasing caught 4 systematic biases I wouldn't have seen), F4 (FMEA rigor surfaced sweep infrastructure + round demarcation that nothing else caught), F7 (cleanest systems view).

**Lowest-value:** A8 (much overlap with H2 and F4; filed shorter analysis).

**Mode echo check:** No mode echo. Most findings had 3+ independent evidence pathways. K1 and K2 had 6+ modes but each via a distinct methodology — genuine convergence, not methodological monoculture.

---

## 12. Mode Selection Retrospective

**Worked well:**
- L2 Debiasing was the single highest-value mode choice. Caught systematic author biases that 9 other modes missed. Without it, the analysis would be softer on aerospace vocabulary leakage, CRM cross-vertical claim, and "small extension" planning-fallacy findings.
- Mixing Claude (F7, F3, I4, L2, F5) and Codex (A8, F4, F2, A1, H2) was clearly right. Codex delivered thorough enumeration (F4 FMEA supplemental gaps; F2 dependency edges). Claude delivered nuanced synthesis (F7 tri-modal identity; L2 bias patterns).
- Perspective-Taking (I4) specifically covered HuberPower and TrashTech hazwaste — two stakeholder views I structurally could not see as the author.

**Would swap in hindsight:**
- A8 Edge-Case produced a shorter output with significant overlap with H2 and F4. In a next pass, swap for B3 Bayesian (probability-calibrated severity) or B10 Reference-Class (how similar migrations went elsewhere) to add genuinely different axis coverage.

**Axis coverage achieved:** Descriptive + Ampliative + Action + Multi-agent + Meta. Missed: Uncertainty-representation (B3 Bayesian), Belief-revision (E1), Adoption (I5 Sensemaking).

---

## 13. What Survives

Despite 11 KERNEL findings and 10 SUPPORTED findings, the overall migration architecture is sound:

- Module boundaries are mostly correct.
- Core substrate (SDK, tenant_id, contract-driven integration) is not challenged by any finding.
- The user's rulings (QMS stays in Fireproof, CNC machine-comm stays, SFDC kiosks stay) hold up under scrutiny.
- Option B pattern (canonical codes + tenant display labels) is architecturally sound; only selective over-application flagged.
- The strangler/data-migration-free approach is correct given sample-data reality.

**Nothing is shown to be fundamentally wrong.** All findings are additive refinements that should be applied before implementation beads are written.

---

## 14. Immediate Next Steps

1. **User reviews this report** — flag which P0 recommendations to action.
2. **Apply P0 spec edits** (R1–R7) — all 7 are small, additive, can be done in one pass.
3. **Mail RoseElk** the finalized module list + overlay-pattern doc (R16) + typed-client dependency graph (I4 unique).
4. **Apply P1 spec edits** (R8–R16) — medium effort; some have open user questions.
5. **Hold P2/P3 until implementation** — file as `docs/plans/` followups.
6. **Circulate revised specs to agent swarm** (CopperRiver, PurpleCliff, MaroonHarbor, SageDesert, DarkCrane) for sign-off before bead decomposition.

---

## Appendix — Mode Output Files

Full mode analyses preserved at:
- `MODE_OUTPUT_F7.md` (Systems-Thinking, 27KB)
- `MODE_OUTPUT_F3.md` (Counterfactual, 33KB)
- `MODE_OUTPUT_I4.md` (Perspective-Taking, 28KB)
- `MODE_OUTPUT_L2.md` (Debiasing, 33KB)
- `MODE_OUTPUT_F5.md` (Root-Cause, 27KB)
- `MODE_OUTPUT_H2.md` (Adversarial, 17KB)
- `MODE_OUTPUT_A8.md` (Edge-Case, 11KB)
- `MODE_OUTPUT_F4.md` (Failure-Mode, 17KB)
- `MODE_OUTPUT_F2.md` (Dependency-Mapping, 22KB)
- `MODE_OUTPUT_A1.md` (Deductive, 14KB)

*End of report.*
