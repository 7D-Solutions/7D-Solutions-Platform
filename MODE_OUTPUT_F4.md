# Mode F4 — Failure-Mode Analysis (FMEA)
## 7D Solutions Platform — Fireproof Migration Spec Review

**Analyst:** CopperBarn (F4 Failure-Mode / FMEA mode) — merged with Codex partial pass
**Date:** 2026-04-17
**Specs reviewed:** bd-ixnbs migration plan, SALES-ORDERS, OUTSIDE-PROCESSING, CUSTOMER-COMPLAINTS, CRM-PIPELINE, SHOP-FLOOR-GATES, PLATFORM-EXTENSIONS, AR-MODULE-SPEC (template), LAYERING-RULES, BOUNDARY-ENFORCEMENT

---

## 1. Thesis

The FMEA lens asks: *for each architectural decision, what is the failure mode, how bad is it, how likely, and can we detect it before it causes damage?* Applying this rigorously surfaces **one critical defect that makes a core feature non-functional at launch** (labor cost trigger references a module that was explicitly retired), **three high-severity implicit contracts with no specified mechanism** (hold enforcement, lot genealogy after OP re-identification, cross-module customer identity chain), and **five medium-severity failures** rooted in under-specified infrastructure: state machine gaps, missing sweep job ownership, ambiguous OP round demarcation, a missing AR subscription in complaints, and training completion split-brain risk. The overall spec quality is high; failures cluster at the seams between modules and in operational infrastructure that the specs assume but don't design.

---

## 2. Top Findings

### §F1 — Manufacturing Costing Labor Trigger References a Retired Module
**Severity:** Critical | **Confidence:** 0.95 | **Occurrence:** Certain | **Detection:** Silent at launch

**Evidence:** `PLATFORM-EXTENSIONS-SPEC.md`, §4 Production Extension — Manufacturing Costing, "Consumed events":
> `shop_floor_data.labor.approved.v1` → Production computes labor cost (duration × operator rate × workcenter cost rate) and posts

**Evidence for retirement:** `bd-ixnbs-fireproof-platform-migration.md`, §Retired drafts:
> `SHOP-FLOOR-DATA-MODULE-SPEC.md — banner-flagged. Only the barcode resolution portion moved to the Inventory extension; kiosks + operator sessions + kiosk-driven labor capture stay in Fireproof…`

**Reasoning chain:** `shop_floor_data.labor.approved.v1` is not a platform event — the module that would emit it was retired from platform scope. Labor capture stays in Fireproof as bespoke hardware workflow. Platform has no mechanism for this event to ever fire. When HuberPower (the second manufacturing vertical) uses manufacturing costing, it has no kiosk/SFDC at all — the trigger simply never exists. The Platform Production module already owns Time Entries for cross-vertical labor tracking. The correct trigger is `production.time_entry.approved.v1` (or `.recorded.v1`), not a shop-floor-data event that cannot exist on platform.

**So What?** Labor is typically 30–60% of WO cost. Without this trigger, `work_order_cost_summaries` will permanently show $0 for labor. Replace the consumed event with `production.time_entry.approved.v1` before the costing bead is written. Requires either adding an approval step to Production's time entry workflow or using `time_entry.recorded.v1` as the trigger. Decision needed at spec level, not at implementation bead level.

---

### §F2 — Hold Enforcement Is an Implied Production Contract With No Mechanism
**Severity:** High | **Confidence:** 0.91 | **Occurrence:** Likely | **Detection:** Only discovered during integration testing

**Evidence:** `SHOP-FLOOR-GATES-MODULE-SPEC.md`, §8 Invariant 8:
> Hold prevents operation start when active on that operation. Downstream — Production should check for active operation-scoped holds before allowing an operation to start. Platform Gates emits `hold.placed.v1`; Production is the enforcer, not Gates. (Alternative: Gates returns active holds via a GET endpoint; Production calls it. Either works; design detail for implementation bead.)

**Reasoning chain:** The entire operational value of shop-floor-gates holds depends on Production refusing to start an operation when a hold is active. The mechanism is deferred to "design detail for implementation bead" — meaning neither module's spec defines it. The two integration paths have radically different failure modes: event subscription can fall behind, leaving a gap between hold placement and Production's local cache update; synchronous GET adds an availability dependency on Gates at every operation-start. Gates spec does not list Production as a consumer of `hold.placed.v1`. If this is left to the implementation bead, the first implementor makes a cross-module architectural decision with no specification backing. The chance of misalignment between the Gates bead and the Production bead is high because they will likely be executed by different agents.

**So What?** Pick the enforcement model before beads are decomposed and document it in both specs. Recommendation: Production calls `GET /api/shop-floor-gates/operations/:op_id/holds?status=active` at operation-start time (synchronous, guarantees consistency). Add this endpoint explicitly to Gates' spec and add the "check for active holds before allowing operation start" behavior to Production's spec.

---

### §F3 — OP Re-Identification Has No Inventory Consumer
**Severity:** High | **Confidence:** 0.90 | **Occurrence:** Certain on first OP cycle with re-ID | **Detection:** Only discovered during end-to-end testing

**Evidence:** `OUTSIDE-PROCESSING-MODULE-SPEC.md`, §11 Open questions:
> Recommend: Inventory creates a child lot, with lot_genealogy recording parent→child. OP's re-identification record is the trigger; Inventory owns the lot mechanics.

`OUTSIDE-PROCESSING-MODULE-SPEC.md`, §5.1 Events Produced: `outside_processing.re_identification.recorded.v1` is defined.

**Evidence of missing consumer:** `PLATFORM-EXTENSIONS-SPEC.md`, §2 and §3 (Inventory extensions). Neither barcode-resolution nor remnant-tracking sections list `outside_processing.re_identification.recorded.v1` in consumed events.

**Reasoning chain:** The open-question resolution ("Inventory creates a child lot") is described as a recommendation, not a specified behavior. The event is emitted by OP but no spec says Inventory consumes it. The lot genealogy update simply does not happen in the current spec set. For aerospace (Fireproof), this is traceability-critical: when a raw bar-stock lot goes to heat treatment and returns as heat-treated material with a new part number, the parent→child lot link in Inventory is the audit chain. Without it, the new material is an orphan lot with no genealogy.

**So What?** Add to Inventory extensions: "Consumed: `outside_processing.re_identification.recorded.v1` — creates child lot via lot-split API, records genealogy edge (parent=old_lot_id, child=new_lot_id, source=outside_processing)." One event, one handler, but must be specified now so OP and Inventory extension beads write compatible contracts.

---

### §F4 — CRM/AR/Party Customer Identity Chain Has No Failure Contract
**Severity:** High | **Confidence:** 0.88 | **Occurrence:** Any Party outage or retry scenario | **Detection:** Orphan records; hard to diagnose from logs

**Evidence:** `CRM-PIPELINE-MODULE-SPEC.md`, §4.1: `POST .../leads/:id/convert` — "creates Party company (via Party API) + opportunity." `CRM-PIPELINE-MODULE-SPEC.md`, §8 Invariant 1: "A lead can only transition to converted if party_id is set (either pre-existing or created during conversion via Party API call from the handler)."

`SALES-ORDERS-MODULE-SPEC.md`, §3: `sales_orders` table has both `customer_id` (ref → AR customer) and `party_id` (ref → Party). `CRM-PIPELINE-MODULE-SPEC.md`, §9: "no automatic AR creation — vertical orchestrates this via their own event handler."

**Reasoning chain:** The identity progression (CRM lead → Party company → AR customer → Sales-Orders eligibility) spans three modules, but no module owns the whole path. Lead conversion calls Party API synchronously within the same HTTP request that also writes the CRM converted status. This is a distributed transaction across two modules with no compensation:
- If the Party API call succeeds but the CRM write fails → Party has an orphaned company record, CRM lead is still in `qualified`. Retry creates a second Party company.
- If Party is down → lead conversion returns 5xx. No idempotency key prevents duplicate Party company creation on retry.
- AR customer creation is explicitly deferred to "vertical orchestration" — meaning it can fail independently. Sales-Orders then has `customer_id=null` on a record that should be billable.

**So What?** (a) Split lead conversion into two explicit steps: call Party API first (returns party_id), then call CRM convert with party_id already set — removes the within-handler Party call; (b) define the idempotency key for Party company creation on conversion; (c) specify whether Sales-Orders may have `customer_id=null` as a valid draft state or whether AR customer creation is a pre-requisite to SO booking.

---

### §F5 — Verification State Machine Has Undocumented Intermediate State
**Severity:** Medium | **Confidence:** 0.87 | **Occurrence:** Every verification | **Detection:** Implementation-time confusion; contract test failures

**Evidence:** `SHOP-FLOOR-GATES-MODULE-SPEC.md`, §3: `operation_start_verifications.status` = `pending/verified/rejected`.
`SHOP-FLOOR-GATES-MODULE-SPEC.md`, §4.3: `POST .../confirm` — "Operator confirms all fields → pending; verifier then signs off to move to verified."
`SHOP-FLOOR-GATES-MODULE-SPEC.md`, §8 Invariant 5: "`verified` transition requires `operator_confirmed_at` to be non-null AND all three boolean flags true."

**Reasoning chain:** The state machine shows three states: `pending → verified | rejected`. But the workflow has two distinct pending sub-states: "pending and operator has not confirmed" vs. "pending and operator has confirmed." A verifier who calls `POST .../verify` on an unconfirmed verification must get a 422 (by invariant 5), but the state machine doesn't represent this guard. Implementation agents will model this as one pending state, discover the invariant enforces a conditional transition, and produce a documented state machine that doesn't match the implemented behavior, failing contract tests.

**So What?** Revise the state machine to expose the two sub-states: `awaiting_operator` → (confirm) → `awaiting_verifier` → (verify | reject) → `verified | rejected`. This makes the invariant structure self-documenting and eliminates the ambiguity.

---

### §F6 — Daily Sweep Jobs Have No Infrastructure Owner or Failure Model
**Severity:** Medium | **Confidence:** 0.93 | **Occurrence:** Every deployment | **Detection:** Silent data divergence until downstream consumer notices missing events

**Evidence:**
- `SALES-ORDERS-MODULE-SPEC.md`, §5.1: `sales_orders.blanket.expired.v1` — "Daily sweep: valid_until < now() and status still active"
- `CUSTOMER-COMPLAINTS-MODULE-SPEC.md`, §5.1: `customer_complaints.complaint.overdue.v1` — "Daily sweep: due_date < now() and status not in..."
- `CRM-PIPELINE-MODULE-SPEC.md`, §5.1: `crm_pipeline.activity.overdue.v1` — "Daily sweep"

**Reasoning chain:** Five-plus daily sweep events are specified across three modules. None of the specs defines: who runs the sweep, what happens if the sweep fails mid-run (partial day processed, some tenants missed), whether sweeps are idempotent, how large datasets are handled, or what the lag tolerance is. The failure modes are quiet — blanket orders that expired yesterday are still showing as `active`. No one notices until a release is created against an expired blanket. The Platform SDK (frozen, additive-only) does not document a cron facility.

**So What?** Add a "Sweep Infrastructure" section to each affected spec specifying: scheduler mechanism (recommendation: pg_cron registered in module migrations), idempotency guarantee (processed_up_to_date watermark), batch size per run, and alerting on sweep failure. One spec addendum can cover all modules rather than repeated boilerplate.

---

### §F7 — Sales-Orders Has State Transitions Without Executable Triggers
**Severity:** Medium | **Confidence:** 0.88 | **Occurrence:** Certain | **Detection:** Implementation surprise; orders stall

**Evidence:** `SALES-ORDERS-MODULE-SPEC.md`, §6.1: State machine includes `booked → in_fulfillment` transition. No endpoint or event triggers this transition. The blanket release lifecycle includes `pending → released` but no endpoint is specified for that transition either; `POST .../releases` creates a release and `POST .../releases/:id/ship` marks it shipped, with no path through `released` state.

**Reasoning chain:** This is a classic failure mode from porting a working Fireproof state machine into a new contract: the states are preserved, but not the mechanisms that advance them. In implementation, this becomes one of two failure modes: either the state is unreachable (orders stall in `booked`, releases stall in `pending`), or teams invent their own triggers and behavior diverges across verticals. The spec says "in_fulfillment" is a valid status that downstream systems can filter on, so it must be reachable.

**So What?** Before decomposing the Sales-Orders module bead: (a) specify the trigger for `booked → in_fulfillment` (recommendation: auto-advance when the first `inventory.reservation.confirmed.v1` arrives, or explicit operator action); (b) decide whether blanket releases are created as `pending` (requiring a separate release action) or directly as `released` (combining create and release). If `pending` is real, add the missing transition endpoint.

---

### §F8 — Multi-Round OP Cycles Have No Round Demarcation in Append-Only Records
**Severity:** Medium | **Confidence:** 0.82 | **Occurrence:** Any OP rejection that returns to vendor | **Detection:** Audit review only; manual reconstruction required

**Evidence:** `OUTSIDE-PROCESSING-MODULE-SPEC.md`, §6.1: `review_in_progress → at_vendor` on rejection = "logged as a second round." `OUTSIDE-PROCESSING-MODULE-SPEC.md`, §3: `op_ship_events` and `op_return_events` have `op_order_id` as FK — no `round_number` column.

**Reasoning chain:** When an OP order goes through a rejection-and-rework cycle, the second round's ship events append to the same `op_order_id` as the first round. The data model has no field to distinguish them. Reconstructing the audit trail requires knowing that a specific event sequence represents two rounds, inferrable only by timestamp ordering. A compliance auditor cannot look at `op_ship_events` and immediately know which events belong to which round. For AS9100, the number of vendor rejection/rework cycles is a traceability data point.

**So What?** Add `round_number` (integer, starting at 1) to both `op_ship_events` and `op_return_events`. Increment when `review_in_progress → at_vendor` transition occurs. Include `round_number` in `outside_processing.shipped.v1` and `outside_processing.returned.v1` event payloads. Low implementation cost, high audit-trail clarity. Must be in the initial schema or it becomes a migration later.

---

## 3. Risks Identified

| Risk | Severity | Likelihood | Failure path |
|------|----------|------------|--------------|
| Labor costs never post on platform (§F1) | Critical | Certain | Broken trigger chain from day one |
| Active holds don't prevent work starts (§F2) | High | Likely | Unspecified Production contract |
| Lot genealogy not updated after OP re-identification (§F3) | High | Certain | Missing event consumer |
| CRM/Party/AR orphan records or duplicate customers (§F4) | High | Moderate | No saga or idempotency key |
| Verifier approves unconfirmed verification (§F5) | Medium | Low-Moderate | Invariant is guard but state machine doesn't show it |
| Blanket orders expired but still `active` (§F6) | Medium | Moderate | Silent drift on sweep failure |
| Orders stall in `booked`; releases stall in `pending` (§F7) | Medium | High | Missing transition triggers |
| Multi-round OP cycles unauditable (§F8) | Medium | Moderate | No round demarcation |
| Invoice-linked complaints lack AR context | Medium | High | No AR event subscription in complaints |
| OP continues for disqualified vendors | Medium-High | Medium | No AP vendor-qualification consumer |
| Training completion diverges from competence assignment | Medium | Medium | No outbox/compensation on cross-module write |
| AP qualification override role undefined | Medium | Moderate | Override log destination and granting authority unspecified |
| Soft FK dangling references (source_entity_id) | Low-Medium | Low | No FK enforcement, no deletion triggers |
| Kit readiness fails if Inventory is unavailable | Low | Low | Operational inconvenience, not data corruption |

### Supplemental: Customer-Complaints Missing AR Subscription
`CUSTOMER-COMPLAINTS-MODULE-SPEC.md` lists `invoice` as a valid `source_entity_type` but does not consume any AR events. Billing complaints will be stored with the source reference but receive no lifecycle updates from AR (e.g., if the invoice is voided, the complaint still shows it as live). This creates operational blindness for billing complaint triage.

### Supplemental: Outside-Processing Vendor Qualification Gap
`PLATFORM-EXTENSIONS-SPEC.md` §7 defines `ap.vendor.disqualified.v1` but `OUTSIDE-PROCESSING-MODULE-SPEC.md` §5.2 consumed events does not include it. An OP order active at the time a vendor is disqualified will continue to ship, receive, and close without any warning. For verticals with supplier qualification requirements, this is a compliance gap.

### Supplemental: Training Completion Split-Brain
`PLATFORM-EXTENSIONS-SPEC.md` §6: `training_completions.resulting_competence_assignment_id` is "set when passing creates a competence_assignment via API call." No failure semantics or outbox defined. The module can record a passed training while the competence assignment write fails, or the operator can appear certified in one table and uncertified in the other.

### Supplemental: AP Override Role Governance
`PLATFORM-EXTENSIONS-SPEC.md` §7: "role-gated override (`ap:po:create_without_qualification`) for exceptional cases, logged." The override permission name is defined but not who can grant it, whether there is a per-vendor limit, or where override audit events are logged (`vendor_qualification_events` tracks qualification status, not PO creation overrides).

---

## 4. Recommendations

| ID | Priority | Effort | Recommendation | Expected Benefit |
|----|----------|--------|----------------|------------------|
| R1 | P0 | Low | Replace `shop_floor_data.labor.approved.v1` with `production.time_entry.approved.v1` (or `.recorded.v1`) in manufacturing costing consumed events. Verify/add approval step to Production time entries. | Labor cost posts correctly from day one for all verticals |
| R2 | P0 | Med | Specify hold enforcement mechanism before Gates and Production beads are written. Recommendation: synchronous GET check + explicit Gates endpoint. Document in both specs. | Hold mechanism has contractual teeth |
| R3 | P0 | Low | Add `outside_processing.re_identification.recorded.v1` to Inventory extension consumed events with child lot creation behavior. | OP re-identification is traceable in Inventory |
| R4 | P0 | Low | Define the trigger for `booked → in_fulfillment` and specify initial status of blanket releases. | Orders don't stall; implementation has clear targets |
| R5 | P1 | Low | Split CRM lead conversion: Party API call explicit and separate from CRM convert endpoint. Add idempotency guidance. | No duplicate Party companies on retry |
| R6 | P1 | Low | Add `round_number` to `op_ship_events` and `op_return_events`. Include in event payloads. | Multi-round OP cycles are auditable |
| R7 | P1 | Med | Revise verification state machine to expose `awaiting_operator` and `awaiting_verifier` as distinct states. | Contract tests pass first time; state machine matches behavior |
| R8 | P1 | Low | Add "Sweep Infrastructure" addendum (pg_cron, idempotency, batch size, alerting) covering all modules with daily sweeps. | Prevents silent data divergence on sweep failure |
| R9 | P2 | Low | Add `ap.vendor.qualification_changed.v1` consumer to Outside-Processing with visible flag/log on active OP orders. | Compliance drift prevented |
| R10 | P2 | Low | Add AR invoice events consumer (or explicit lookup path) to Customer-Complaints for invoice-linked complaints. | Billing complaints not context-blind |
| R11 | P2 | Med | Add outbox or retry semantics for training completion → competence assignment write. | No split-brain training records |
| R12 | P3 | Low | Add `vendor_po_override_events` audit table to AP extension spec. Define override role granting authority. | Supplier control has complete audit trail |

---

## 5. New Ideas and Extensions

### Incremental
- **Active-hold check endpoint in Gates:** Add `GET /api/shop-floor-gates/operations/:id/has-active-hold` returning `{blocked: bool, hold_ids: []}` — purpose-built for Production's pre-start check, cheaper than listing all holds and filtering client-side.
- **round_number on OP event payloads:** Include `round_number` in `outside_processing.shipped.v1` and `outside_processing.returned.v1` so downstream consumers (manufacturing costing, Quality-Inspection overlay) know which round they're processing.
- **Vendor-disqualification flag on OP list view:** OP active orders for a disqualified vendor should surface a warning badge to prevent silent compliance drift.

### Significant
- **Sweep-job platform primitive:** Given that 5+ modules need daily sweeps, and the Platform SDK is frozen for new extension points, consider adding a `.sweeper()` registration to the module manifest DSL (additive, not breaking). This makes sweep infrastructure first-class and eliminates per-module implementation fragmentation.
- **Platform-neutral labor approval event contract:** Define `production.time_entry.approved.v1` with a canonical payload (operator, duration, workcenter, work_order_id, operation_id) that any manufacturing vertical can produce without depending on Fireproof's kiosk stack.

### Radical (flag for future discussion, not actionable now)
- **Platform signoff primitive:** Shop-floor-gates has a whitelisted signoff pattern. Quality-Inspection will likely need signoffs on inspections. Rather than each module embedding its own, a platform-level `attestation` service could provide cryptographically signed attestations with a unified audit trail. This would be a new module (not an SDK extension) and is out of scope for this wave — but worth flagging before N modules build incompatible patterns.
- **Customer identity bridge spec:** Define a small, explicit lifecycle bridge spec for CRM lead → Party company → AR customer → Sales-Orders eligibility. Documentation and contract work, not a new runtime abstraction, but it makes the current hidden orchestration chain visible and testable.

---

## 6. Assumptions Ledger

1. **Production module has time entries with an approval workflow (or can add one).** The manufacturing costing fix depends on this. If Production's time entries are record-only with no approval step, the fix requires adding approval to Production's spec first.
2. **Party module enforces company dedup by some combination of name/email/external-id.** CRM lead conversion relies on this for retry safety.
3. **pg_cron or equivalent is available in the deployment environment.** Sweep-job recommendation assumes this.
4. **shop-floor-gates is consumed exclusively by manufacturing verticals (Fireproof, HuberPower).** If TrashTech or RanchOrbit need operational holds, scope assumptions change.
5. **OP re-identification is always 1:1 (old lot → new lot).** If re-identification can split one lot into multiple new lots, the Inventory event consumer needs to handle multiple child lots per event.
6. **Training completion and competence assignment are intended to remain separate module-owned records.** If they are meant to be atomic writes within one service, the split-brain risk disappears.
7. **AR customer is a separate record from Party company, not a renamed view.** Sales-Orders' dual reference (customer_id + party_id) is intentional, not redundant.

---

## 7. Questions for Project Owner

1. **Labor trigger (§F1):** Does Production's existing time entry model have an "approved" status, or is it record-only? If record-only, do you want to add approval, or use `time_entry.recorded.v1` as the trigger (immediate posting, no approval gate)?
2. **Hold enforcement mechanism (§F2):** Event subscription or synchronous GET? This is an architectural decision with test-contract implications. Which do you prefer, and should it be specified before Gates and Production beads are written?
3. **`booked → in_fulfillment` trigger (§F7):** Is `in_fulfillment` auto-advanced on first inventory reservation confirmation, or does it require an explicit operator action?
4. **Daily sweep infrastructure (§F6):** Does the Platform SDK's `.run()` already include a cron/sweep facility? Or does each module need to implement its own?
5. **OP multi-round auditing (§F8):** Is `round_number` required for the Fireproof aerospace customer's traceability records, or is timestamp-ordering reconstruction acceptable for v0.1?
6. **Signoff fragmentation:** Is "each module embeds its own signoff" the long-term plan, or is a future platform-level attestation service on the roadmap? If eventual, should the whitelist be extensible now?
7. **Training completion atomicity:** Is passing training intended to be atomic with competence assignment creation, or is eventual consistency (with retry) acceptable?

---

## 8. Points of Uncertainty

- **Production module's current spec:** Not in the review set. §F2 assumes Production doesn't already consume Gates events. If it does, §F2 is a non-issue.
- **Time entry approval in Production:** §F1 resolution depends on whether Production has this. Not knowable from the specs reviewed.
- **Platform SDK cron facility:** Whether `.run()` supports periodic tasks is not documented in the context pack. The sweep-job gap (§F6) may already be solved at SDK level.
- **Kit readiness Inventory call mechanics:** The spec says "pulls on-hand from Inventory" but doesn't name the specific Inventory endpoint. If Inventory's availability query API doesn't exist yet, kit readiness needs to specify it before the kit-readiness bead is written.
- **Whether Fireproof has internal retry/saga behavior** for CRM conversion and training completion that is not documented in the platform specs. If so, some of §F4 and training risks are mitigated for Fireproof but not for other verticals.

---

## 9. Agreements and Tensions with Other Perspectives

**Expected agreements with F7 (Systems-Thinking):** Both modes likely flag the hold-enforcement gap (§F2) and the missing lot-genealogy consumer (§F3) as cross-module seam problems. Systems-Thinking frames them as emergent behaviors owned by no one; FMEA frames them as failure modes with no detection path.

**Expected agreements with F5 (Root-Cause):** §F1 (broken labor trigger) is likely flagged from both directions — root-cause asks "why was shop-floor-data retired but its event name left in the costing consumed events list?" FMEA asks "what breaks when it fires (or doesn't)?"

**Expected tension with A8 (Edge-Case):** Edge-case mode would likely expand §F5 (verification state machine) into detailed permutation analysis of all boolean flag combinations. FMEA is satisfied identifying the structural gap; edge-case would enumerate all 2^n flag states.

**Expected tension with I4 (Perspective-Taking):** I4 might frame §F6 (sweep jobs) as an operator experience problem (operators not getting timely overdue alerts). FMEA frames it as infrastructure gap. I4 may advocate for webhook push; FMEA is indifferent to mechanism as long as the failure model is specified.

**Unique coverage (not expected from other modes):** §F1 (retired-module trigger) and §F3 (missing event consumer) are pure failure modes that require cross-document diff reasoning — checking what the spec says relative to decisions made in other documents in the set. This is FMEA's strongest suit and is unlikely to be surfaced by modes that focus on what the spec *intends* rather than what it *says*.

---

## 10. Confidence: 0.85

**Calibration note:** High confidence on §F1 (textual evidence is unambiguous — shop-floor-data retired, event reference left in), §F3 (event emitted, no consumer specified — absence of evidence is the evidence), §F6 (five specs define sweeps, none defines the runner), and §F7 (state machine has an arc with no triggering endpoint). Medium confidence on §F2 (Production module spec not in review set — it may already handle this) and §F4 (the failure mode is real but the severity depends on how verticals actually implement the bridge). Overall score reflects the known gap in Production module review.
