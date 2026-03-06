# Manufacturing Lifecycle Drill-Down Tabs — Review by Claude Desktop

**Date:** 2026-03-04
**Reviewer:** Claude Desktop (Cowork)
**Artifact reviewed:** `docs/plans/manufacturing-lifecycle.drawio` (11 tabs)
**Context:** `docs/manufacturing-review-synthesis.md`

---

## Verdict: APPROVED with 14 issues (0 blocking, 6 medium, 8 minor)

The drill-down tabs are well-structured and represent a significant step up from the overview-only Round 1 diagram. Process flows are logical, data entities are reasonable, and the event names are mostly consistent with actual codebase conventions. The main problems are cross-reference direction inconsistencies (6 instances) and a handful of missing integration points in specific tabs. Nothing is architecturally wrong.

---

## Tab-by-Tab Review

### Tab 0: Overview (RFQ to RMA)

**Process flows:** Correct. The Round 2 changes are applied — PO is green, Receiving Inspection routing is referenced on the Receive Materials box, MRB has disposition paths. The overview is an accurate summary of the drill-downs.

**Issues:** None.

---

### Tab 1: Procurement & Materials

**Process flows:** Excellent. The three-lane structure (PO lifecycle → Receiving & Inspection Routing → AP Settlement) correctly represents procure-to-pay. The inspection routing diamond with `direct_to_stock` / `send_to_inspection` branches matches the existing Shipping-Receiving implementation.

**Data entities:** Reasonable. `PurchaseOrder`, `VendorInvoice`, `ThreeWayMatchResult` all exist in AP. `StatusBucket` in Inventory is correct.

**Events:**

**Issue #1 (MINOR — event naming):** Tab 1 lists events as `ap.po.created`, `ap.po.approved`. The actual codebase uses underscores: `ap.po_created`, `ap.po_approved` (verified in `modules/ap/src/events/po.rs`). Similarly, `sr.receipt.created` and `sr.inspection_routing.decided` should be verified against Shipping-Receiving's actual subject constants. Consistent dot-vs-underscore convention matters for downstream consumers wiring up subscriptions.

**Cross-references:** Correct and complete. All four outbound references (→ Tab 4, → Tab 8, → Tab 3) have matching reciprocals on those tabs.

---

### Tab 2: Engineering

**Process flows:** Good. The three-lane structure (BOM Structure → ECO via Workflow → Part Numbering) is clean. The ECO lane correctly shows Workflow module handling the review/approve steps (green boxes) while BOM handles the domain-specific draft and apply steps (amber boxes).

**Data entities:** Reasonable. `BomHeader`, `BomRevision`, `BomLine`, `EngineeringChangeOrder`, `EcoAffectedItem` are the right entities for a discrete manufacturing BOM module.

**Events:** `bom.revision.released` is the key event that Production consumes — correct.

**Issue #2 (MINOR — missing entity):** The data entities panel doesn't list `BomAlternate` or any alternate/substitute component concept. In manufacturing, BOMs commonly have alternate materials (if component A is unavailable, use component B). This isn't blocking for v1 but is worth noting as a future entity.

**Cross-references:**

**Issue #3 (MEDIUM — missing reciprocal):** Tab 2 says "← Tab 7 (People): Workforce-Competence for ECO reviewer qualifications." But Tab 7 does not list Tab 2 in its cross-references. Tab 7's outbound refs go to Tabs 3, 4, 5, 6, 8 — but not Tab 2. Add "→ Tab 2 (Engineering): ECO reviewer qualifications" to Tab 7.

---

### Tab 3: Production

**Process flows:** Good. The two-lane structure (WO Lifecycle → Shop Floor Execution) correctly separates planning from execution. The authorization check between "Start Operation" and "Complete Operation" is correctly placed and colored green (Workforce-Competence exists).

**Issue #4 (MEDIUM — missing reject/scrap branch):** "Complete Operation" mentions "Good/reject qty" in its label, but there's no branching path for rejected material. In manufacturing, rejects at operation completion either go to rework (loop back to an earlier operation or create a rework WO) or to scrap (inventory adjustment + GL write-off). The flow currently goes linearly from Complete Operation → FG Receipt, implying 100% yield. At minimum, add a dashed branch from Complete Operation to Quality-Inspection (in-process rejection triggers inspection disposition) or a note acknowledging that reject handling flows through Tab 4.

**Data entities:** Correct. `WorkOrder`, `Routing`, `Operation`, `MaterialIssue`, `OperationCompletion`, `ProductionReceipt` cover the core Production domain.

**Events:** `production.wo.created`, `production.operation.started`, `production.operation.completed`, `production.fg.received` — clean, consistent naming. These match what Tab 4, 6, 7, and 8 consume.

**Cross-references:** Complete and consistent. All six references have matching reciprocals on the target tabs. The "v1: Explicit issue only (no backflush)" annotation is important and correctly placed.

---

### Tab 4: Quality Management

**Process flows:** Strong. The three-lane structure (Inspection → Disposition & Hold/Release → NCR/CAPA deferred) is well-organized. The MRB disposition diamond correctly shows all five options from Round 2 consensus: Rework, Scrap, Use-As-Is, Return to Vendor, Sort/100% Screen.

**Data entities:** Reasonable. `InspectionPlan`, `Characteristic`, `SamplingRule`, `InspectionRecord`, `InspectionMeasurement` are the right entities.

**Issue #5 (MINOR — ambiguous revision reference):** The InspectionPlan entity shows "(item, revision)" but doesn't specify whether "revision" means BOM revision or Inventory item revision. In manufacturing, inspection plans are typically tied to the item + item revision (because inspection criteria change when the item definition changes, not when the BOM structure changes). Clarify as "(item_id, item_revision_id)" to avoid confusion during implementation.

**Events:** The consume list (`sr.inspection_routing.decided`, `production.operation.completed`, `production.fg.received`) correctly identifies the three trigger points for receiving, in-process, and final inspection.

**Cross-references:**

**Issue #6 (MEDIUM — missing reciprocal):** Tab 4 has no "← Tab 5 (Calibration)" cross-reference. Tab 5 says "→ Tab 4 (Quality): OOT triggers suspect product trace via inspection records." Tab 4 should have a corresponding "← Tab 5 (Calibration): OOT suspect product trace queries inspection records" entry.

**Issue #7 (MEDIUM — direction inconsistency):** Tab 4 says "→ Tab 10 (Post-Sale): NCR from RMA (deferred)." The arrow direction is wrong — Post-Sale sends RMA-triggered NCRs to Quality, not the other way around. Tab 10 correctly says "→ Tab 4 (Quality): NCR from RMA (deferred)." Tab 4 should say "← Tab 10 (Post-Sale): NCR from RMA (deferred)."

**Deferred items:** NCR, CAPA, and FAI are correctly shown in gray (#555555) rather than amber, making it visually clear they're not in scope for the current build. FAI is annotated as "App-specific, not platform" which aligns with the synthesis.

---

### Tab 5: Calibration

**Process flows:** Clean and correct. The two-lane structure (Calibration Lifecycle → OOT Disposition & Suspect Product Trace) is well-organized. The pass/fail diamond after "Perform Calibration" is the right decision point.

**Color coding:**

**Issue #8 (MINOR — color accuracy):** Tab 5 header says "Status: GREEN — Exists inside Maintenance module today." The calibration lifecycle boxes (Instrument Record, Cal Schedule, Perform Calibration, Certify & Return) are green, which is accurate — `maintenance.calibration.*` events exist in the codebase (verified: `CALIBRATION_CREATED`, `CALIBRATION_COMPLETED`, `CALIBRATION_EVENT_RECORDED`, `CALIBRATION_STATUS_CHANGED` in `modules/maintenance/src/events/subjects.rs`). However, the OOT Disposition boxes are amber. This is correct — OOT disposition and suspect product trace are proposed extensions, not currently implemented. The status header should say "Status: GREEN with AMBER extensions" to avoid implying everything in the tab exists today.

**Events:**

**Issue #9 (MINOR — event naming mismatch):** Tab 5 lists `maintenance.calibration.completed` and `maintenance.calibration.oot_found`. The actual codebase has `maintenance.calibration.completed` (correct) but does NOT have `maintenance.calibration.oot_found` — it has `maintenance.calibration.status_changed` which would cover OOT as a status transition. Either the proposed event name should match the existing convention (`status_changed` with an OOT payload), or the new event should be explicitly called out as a proposed addition.

**Cross-references:**

**Issue #10 (MEDIUM — direction inconsistency):** Tab 5 says "→ Tab 7 (People): Calibration tech competence requirements." This implies Calibration sends something to People. But the relationship is: People/Workforce-Competence provides authorization data that Calibration consumes (checking if the cal tech is qualified). Tab 5 should say "← Tab 7 (People): Calibration tech competence requirements." Tab 7 correspondingly says "→ Tab 5 (Calibration): Cal tech quals" which is the correct direction from Tab 7's perspective.

---

### Tab 6: Equipment Maintenance

**Process flows:** Correct and complete. The two-lane structure (PM Scheduling → Downtime & Workcenter) accurately represents the existing Maintenance module with the workcenter retrofit clearly marked in blue.

**Data entities:** Accurate. The retrofit note correctly states "Production owns workcenter going forward; Maintenance consumes via API."

**Events:** The published events (`maintenance.wo.created`, `maintenance.wo.completed`, `maintenance.downtime.started`, `maintenance.downtime.ended`, `maintenance.pm.due`, `maintenance.pm.overdue`) are reasonable. Checking against the actual codebase: the real subjects use slightly different naming (`maintenance.work_order.created`, `maintenance.work_order.completed`, `maintenance.downtime.recorded`, `maintenance.plan.due`). Tab 6's event names are simplified/summarized rather than exact.

**Issue #11 (MINOR — event name accuracy):** Tab 6 event names don't match the actual NATS subjects exactly. `maintenance.wo.created` should be `maintenance.work_order.created`. `maintenance.downtime.started`/`ended` should be `maintenance.downtime.recorded` (single event, not start/end pair). `maintenance.pm.due` should be `maintenance.plan.due`. These are minor but could cause confusion during implementation wiring.

**Cross-references:** Correct. All three references have matching reciprocals.

---

### Tab 7: People & Training

**Process flows:** Good. The two-lane structure (Competence & Authorization → Timekeeping & Labor) correctly separates the two modules. The "Authorization Check" box with "Point-of-use API query" annotation correctly describes how Production and Quality will consume this capability.

**Data entities:** Accurate. `CompetenceArtifact`, `AcceptanceAuthority`, `QualificationRequirement` match the existing Workforce-Competence domain. `TimeEntry`, `LaborAllocation`, `TimesheetPeriod` match Timekeeping.

**Events consumed:** `production.operation.started` and `production.operation.completed` for auto-creating time entries is a smart integration design.

**Issue #3 (repeated):** Missing "→ Tab 2 (Engineering): ECO reviewer qualifications" in cross-references.

---

### Tab 8: Fulfillment & Finance

**Process flows:** Clean. The two-lane structure (Physical flow: Pick → Ship → Invoice → Payment, and Financial flow: COGS → Revenue → Cash → FG Cost → Labor Cost) correctly separates the physical and financial streams. The "FG Cost Rollup" box with "Event-driven from Production" annotation captures the critical WIP → FG cost transfer.

**Data entities:** Correct for all four modules (Shipping-Receiving, AR, Payments, GL).

**Events consumed:** `production.fg.received`, `timekeeping.period.closed`, `inventory.movement.created` — all correct trigger events for the financial postings.

**Cross-references:** Complete and consistent.

---

### Tab 9: Sales Cycle

**Process flows:** Appropriately minimal for a gap analysis tab. The linear flow (RFQ → Quote → SO → Review → Release to Production) is correct. The Sales Order box having a dashed red border (distinguishing it as "likely platform" vs the solid red RFQ/Quote boxes) is a good visual cue.

**Issue #12 (MEDIUM — direction inconsistency):** Tab 9 says "→ Tab 2 (Engineering): BOM costing." The arrow direction is wrong — Sales Order requests BOM cost data from Engineering, so Sales is the consumer. Tab 9 should say "← Tab 2 (Engineering): BOM costing." Tab 2 correspondingly says "→ Tab 9 (Sales): BOM costing for quotes" which is the correct direction from Tab 2's perspective.

**Gap analysis notes:** The "Platform vs App-Specific" panel is a valuable addition. The distinction (Sales Order = likely platform, RFQ/Quoting = likely app-specific) aligns with the synthesis consensus.

---

### Tab 10: Post-Sale — RMA & Customer Support

**Process flows:** Correct. The Customer Complaint → Triage → Action decision point (red/gap) feeding into the existing RMA lifecycle (green) is accurate. The RMA disposition paths (Replace, Repair, Credit/Refund) match standard post-sale dispositions.

**Issue #13 (MINOR — RMA disposition model mismatch):** The existing RMA code has a 5-state physical disposition model: `received → inspect → quarantine → return_to_stock | scrap` (verified in `modules/shipping-receiving/src/domain/rma/types.rs`). The diagram shows business-level dispositions (Replace, Repair, Credit/Refund) which are complementary but at a different abstraction level. Both are correct — the physical disposition (what happens to the returned item) and the business disposition (what the customer gets) are different axes. Consider adding a note: "Physical disposition handled by S-R RMA state machine; business disposition shown here."

**Deferred NCR box:** Correctly shown in gray with "(Deferred — Tab 4 scope)" annotation. The cross-reference to Tab 4 is present.

**Cross-references:** Correct. Tab 10's "→ Tab 4 (Quality): NCR from RMA (deferred)" matches (though Tab 4 has the direction wrong — see Issue #7).

---

## Cross-Reference Consistency Matrix

I audited every cross-reference across all 11 tabs. Here's the full consistency check:

| Source Tab | Reference | Target Tab Reciprocal | Status |
|------------|-----------|----------------------|--------|
| Tab 1 → Tab 3 | Material issue from inventory | Tab 3 ↔ Tab 1 | ✅ |
| Tab 1 → Tab 4 | Inspection routing triggers | Tab 4 ← Tab 1 | ✅ |
| Tab 1 → Tab 8 | GL cost postings | Tab 8 ← Tab 1 | ✅ |
| Tab 2 → Tab 1 | Item master lookups | (implicit) | ✅ |
| Tab 2 → Tab 3 | BOM explosion | Tab 3 ← Tab 2 | ✅ |
| Tab 2 → Tab 9 | BOM costing | Tab 9 → Tab 2 | ❌ Direction mismatch (Issue #12) |
| Tab 2 ← Tab 7 | ECO reviewer quals | Tab 7 missing | ❌ Missing reciprocal (Issue #3) |
| Tab 3 → Tab 4 | IPC + final inspection | Tab 4 ← Tab 3 | ✅ |
| Tab 3 ← Tab 6 | Workcenter availability | Tab 6 → Tab 3 | ✅ |
| Tab 3 ← Tab 7 | Operator authorization | Tab 7 → Tab 3 | ✅ |
| Tab 3 → Tab 8 | FG receipt → GL | Tab 8 ← Tab 3 | ✅ |
| Tab 4 ← Tab 7 | Inspector authorization | Tab 7 → Tab 4 | ✅ |
| Tab 4 → Tab 1 | Hold/release inventory | (implicit) | ✅ |
| Tab 4 → Tab 10 | NCR from RMA | Tab 10 → Tab 4 | ❌ Direction wrong on Tab 4 (Issue #7) |
| Tab 4 ← Tab 5 | OOT suspect product trace | Tab 4 missing | ❌ Missing reciprocal (Issue #6) |
| Tab 5 → Tab 4 | OOT → suspect trace | (see above) | ❌ |
| Tab 5 → Tab 7 | Cal tech quals | Tab 7 → Tab 5 | ❌ Direction wrong on Tab 5 (Issue #10) |
| Tab 5 ← Tab 6 | Repair WOs | Tab 6 → Tab 5 | ✅ |
| Tab 6 → Tab 3 | Workcenter availability | Tab 3 ← Tab 6 | ✅ |
| Tab 6 → Tab 5 | Calibration sub-module | Tab 5 ← Tab 6 | ✅ |
| Tab 6 ← Tab 7 | Technician quals | Tab 7 → Tab 6 | ✅ |
| Tab 7 → Tab 3 | Operator auth | Tab 3 ← Tab 7 | ✅ |
| Tab 7 → Tab 4 | Inspector auth | Tab 4 ← Tab 7 | ✅ |
| Tab 7 → Tab 8 | Labor cost → GL | Tab 8 ← Tab 7 | ✅ |
| Tab 8 → Tab 9 | SO triggers shipment | Tab 9 implicit | ✅ |
| Tab 8 → Tab 10 | Credit memos | Tab 10 implicit | ✅ |

**Summary: 20 correct, 5 inconsistent (3 direction mismatches, 2 missing reciprocals)**

---

## Issue Summary

| # | Tab | Severity | Issue |
|---|-----|----------|-------|
| 1 | Tab 1 | MINOR | Event names use dots (`ap.po.created`) but codebase uses underscores (`ap.po_created`) |
| 2 | Tab 2 | MINOR | Missing `BomAlternate` entity for substitute components |
| 3 | Tab 2/7 | MEDIUM | Tab 7 missing reciprocal "→ Tab 2: ECO reviewer qualifications" |
| 4 | Tab 3 | MEDIUM | No reject/scrap branch from "Complete Operation" |
| 5 | Tab 4 | MINOR | InspectionPlan "(item, revision)" should specify item_revision_id, not BOM revision |
| 6 | Tab 4 | MEDIUM | Missing "← Tab 5 (Calibration): OOT suspect product trace" |
| 7 | Tab 4 | MEDIUM | Direction wrong: "→ Tab 10" should be "← Tab 10" (NCR from RMA) |
| 8 | Tab 5 | MINOR | Header says "GREEN" but OOT disposition is AMBER; clarify as "GREEN with AMBER extensions" |
| 9 | Tab 5 | MINOR | `maintenance.calibration.oot_found` doesn't exist; actual event is `maintenance.calibration.status_changed` |
| 10 | Tab 5 | MEDIUM | Direction wrong: "→ Tab 7" should be "← Tab 7" (receives competence data) |
| 11 | Tab 6 | MINOR | Event names simplified; actual subjects differ (e.g., `maintenance.work_order.created` not `maintenance.wo.created`) |
| 12 | Tab 9 | MEDIUM | Direction wrong: "→ Tab 2" should be "← Tab 2" (Sales receives BOM costing) |
| 13 | Tab 10 | MINOR | RMA disposition abstraction level differs from existing code model; add clarifying note |
| 14 | — | — | **No issues with Tabs 0, 8** — these are clean |

---

## What's Done Well

- The drill-down structure (overview → per-module tabs) is the right level of detail for implementation planning.
- Color coding is accurate for all boxes verified against the codebase.
- Event publish/consume relationships are consistent across tabs (the events Tab 3 publishes are the events Tabs 4, 6, 7, and 8 consume).
- Deferred items (NCR/CAPA, FAI) are visually distinct in gray rather than amber.
- The "Platform vs App-Specific" panel on Tab 9 captures the key scoping question for Sales.
- Retrofit items (blue boxes) are correctly identified and annotated with what needs to change.
- The synthesis recommendations (discrete manufacturing only, no backflush in v1, explicit issue) are reflected in the annotations.

---

*Review complete. APPROVED with 14 issues to address — none blocking.*
