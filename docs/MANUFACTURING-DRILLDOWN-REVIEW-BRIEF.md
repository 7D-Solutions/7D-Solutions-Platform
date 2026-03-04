# Manufacturing Lifecycle — Drill-Down Tabs Review Brief

**Bead:** bd-87f5a
**Date:** 2026-03-04
**From:** BrightHill (Orchestrator)
**Review requested from:** All agents + ChatGPT

---

## What We're Building

Adding 10 drill-down tabs to `docs/plans/manufacturing-lifecycle.drawio` — one per swim lane in the overview. Each tab shows internal process flows, data entities, events, and integration points for that module area.

**All tabs go in the same `.drawio` file** as additional `<diagram>` elements.

---

## Layout Convention (Same for Every Tab)

- Dark background (#1a1a2e) matching the overview
- 2-3 swim lanes showing internal process steps (left → right)
- Right-side info panels (x=1200+) for: Data Entities, Events Published, Events Consumed
- Same color coding: GREEN = exists, AMBER = proposed, RED = gap, BLUE = needs retrofit
- Plain English labels only — no file paths or code

---

## 10 Proposed Tabs

### Tab 1: Procurement & Materials (GREEN)

**Lane border:** Green (modules exist)
**Crates:** ap (AP module), inventory, shipping-receiving

**Process Flow (Swim Lane 1 — Purchase-to-Stock):**
Draft PO → Approve PO → Vendor Ships → Receive Materials → Inspection Routing Decision → (Pass) Store in Inventory → Goods Receipt triggers 3-Way Match → Approve Bill → Payment Run

**Process Flow (Swim Lane 2 — Exception Path):**
Inspection Routing → (Fail) → NCR (links to Quality tab)

**Data Entities Panel:**
- Vendor (AP) — payment terms, method, remittance
- PurchaseOrder + PoLine (AP) — PO header/lines, status machine: draft→approved→closed
- VendorBill + BillLine (AP) — invoice matching, status: open→matched→approved→paid
- three_way_match (AP) — links bill lines ↔ PO lines ↔ receipt records
- PaymentRun (AP) — batch payments by vendor/currency
- Item (Inventory) — SKU, tracking mode (none/lot/serial), GL accounts
- InventoryLot / InventorySerialInstance (Inventory) — batch/unit tracking
- AvailableLayer (Inventory) — FIFO cost layers

**Events Published:**
- ap.po_created, ap.po_approved, ap.po_closed
- ap.po_line_received_linked (receipt ↔ PO 3-way anchor)
- ap.vendor_bill_created, ap.vendor_bill_matched, ap.vendor_bill_approved, ap.vendor_bill_voided
- ap.payment_run_created, ap.payment_executed
- inventory.item_received, inventory.item_issued
- sr.receipt_routed_to_inspection.v1, sr.receipt_routed_to_stock.v1
- shipping_receiving.inbound_closed

**Events Consumed:**
- ap.vendor_bill_approved → GL (expense posting)
- inventory.item_received → AP (GRN linking)

**Cross-references:** Engineering tab (BOM drives procurement), Quality tab (inspection failures → NCR), Fulfillment tab (AP → GL)

---

### Tab 2: Engineering (AMBER + GREEN)

**Lane border:** Amber (BOM and ECO are proposed; Document Control exists)
**Crates:** (proposed: bom-rs), platform/doc-mgmt (existing)

**Process Flow (Swim Lane 1 — BOM Lifecycle):**
Create BOM → Add Components (multi-level) → Set Effectivity Dates → Release BOM → BOM Revision → Where-Used Query

**Process Flow (Swim Lane 2 — Change Control):**
ECO Request → Workflow Approval → BOM Revision Created → Document Update → Release Revised BOM

**Data Entities Panel (Proposed):**
- BomHeader — product structure, revision, effectivity_from/to
- BomLine — component items, quantity_per, find_number, reference designators
- EngineeringChangeOrder — change request with workflow lifecycle
- BomRevision — immutable snapshot per release

**Data Entities Panel (Existing):**
- Document (doc-mgmt) — drawings, specs, work instructions
- WorkflowInstance (workflow) — ECO approval routing

**Events (Proposed):**
- bom.created, bom.released, bom.revision_created
- bom.eco_submitted, bom.eco_approved, bom.eco_rejected
- bom.component_added, bom.component_removed

**Events Consumed:**
- workflow.approved / workflow.rejected (ECO decisions)

**Cross-references:** Procurement tab (BOM drives PO creation), Production tab (BOM explosion for work orders), Quality tab (BOM revision triggers re-inspection)

**Build Phase:** A (foundation — build first, fewest dependencies)

---

### Tab 3: Production (AMBER)

**Lane border:** Amber (proposed module)
**Crates:** (proposed: production-rs)

**Process Flow (Swim Lane 1 — Work Order Lifecycle):**
Create WO (from BOM explosion) → Release WO → Issue Materials (from Inventory) → Operations (Machine→Check→Weld→Check→Heat Treat→Assembly) → Final Test → Receive Finished Goods (to Inventory)

**Process Flow (Swim Lane 2 — Quality Touchpoints):**
Operations ↔ In-Process Checks (from Quality) → Final Inspection (from Quality) → FG Receipt

**Data Entities Panel (Proposed):**
- WorkOrder — lifecycle (draft→released→in_progress→completed→closed), BOM reference, quantity, due_date
- WoOperation — routing steps, sequence, workcenter, setup/run times
- MaterialIssue — component issuance from inventory (lot/serial traceability)
- ProductionReceipt — FG receipt into inventory (distinct from purchase receipt — different costing)
- WipCostAccumulator — material + labor + overhead tracking per WO

**Events (Proposed):**
- production.wo_created, production.wo_released, production.wo_completed, production.wo_closed
- production.material_issued (→ inventory.item_issued)
- production.operation_started, production.operation_completed
- production.fg_received (→ inventory.item_received with production receipt type)

**Events Consumed:**
- bom.released (BOM explosion source)
- inventory.item_issued (material consumption confirmation)
- workflow.approved (WO release approval)
- workforce_competence.competence_assigned (operator qualification check)

**Cross-references:** Engineering tab (BOM explosion), Procurement tab (material availability), Quality tab (in-process + final inspection), Maintenance tab (workcenter availability, equipment breakdown → downtime), People tab (operator qualifications, labor collection), Fulfillment tab (FG receipt → ship, WIP→FG cost transfer → GL)

**Build Phase:** B (core manufacturing — after BOM)

---

### Tab 4: Quality Management (AMBER)

**Lane border:** Amber (proposed module, split into Inspection now + NCR/CAPA later)
**Crates:** (proposed: quality-inspection-rs, quality-ncr-rs deferred)

**Process Flow (Swim Lane 1 — Inspection):**
Define Inspection Plan → Receiving Inspection (from S-R routing) → In-Process Checks (from Production ops) → Final Inspection → Pass: Release to FG / Fail: Create NCR

**Process Flow (Swim Lane 2 — NCR/CAPA, deferred):**
NCR Created → Investigation → MRB Disposition (Repair / Use-As-Is / Scrap / Return to Vendor / Sort) → CAPA (root cause → corrective action → update inspection plans)

**Data Entities Panel (Proposed — Inspection, build now):**
- InspectionPlan — characteristics, tolerances, sampling rules (ISO 2859)
- InspectionRecord — results per characteristic, pass/fail, inspector_id
- FirstArticleInspection — FAI report (concept is platform; AS9102 format is app-specific)
- InspectionHold — integration with Inventory status buckets (quarantine)

**Data Entities Panel (Proposed — NCR/CAPA, deferred):**
- NonconformanceReport — defect description, severity, disposition pending
- MrbDisposition — decision record (repair/use-as-is/scrap/RTV/sort)
- CorrectiveAction — root cause, action items, verification, effectiveness check

**Events (Proposed — Inspection):**
- quality.inspection_plan_created, quality.inspection_plan_revised
- quality.inspection_completed (pass/fail, with results)
- quality.hold_placed, quality.hold_released

**Events (Proposed — NCR/CAPA, deferred):**
- quality.ncr_created, quality.ncr_dispositioned
- quality.capa_opened, quality.capa_closed

**Events Consumed:**
- sr.receipt_routed_to_inspection.v1 (receiving inspection trigger)
- production.operation_completed (in-process check trigger)
- workforce_competence.competence_assigned (inspector authorization)
- calibration.status_changed (gage validity for inspection equipment)

**Cross-references:** Procurement tab (receiving inspection), Production tab (in-process + final), Calibration tab (instrument validity), People tab (inspector authorization), Post-Sale tab (RMA → NCR)

**Build Phase:** C (quality gates — after Production)

---

### Tab 5: Calibration (AMBER)

**Lane border:** Amber (proposed standalone optional module)
**Crates:** (proposed: calibration-rs)

**Process Flow (Swim Lane 1 — Calibration Lifecycle):**
Register Instrument → Set Calibration Schedule → Perform Calibration → Record Results & Certificate → (Pass) Update Status to In-Cal → (Fail) OOT Disposition

**Process Flow (Swim Lane 2 — OOT Impact):**
Out-of-Tolerance Found → Suspect Product Trace (lookup inspection records using this instrument) → NCR for Affected Product → Quarantine Suspect Lots

**Data Entities Panel (Proposed):**
- Instrument — gage/instrument/standard/fixture, serial number, location, cal_interval
- CalibrationSchedule — due dates, intervals (calendar-based), in-cal/due/overdue status
- CalibrationRecord — results, certificate reference, NIST traceability chain, performed_by
- OotDisposition — out-of-tolerance impact assessment, affected product list

**Events (Proposed):**
- calibration.instrument_registered
- calibration.scheduled, calibration.completed (pass/fail)
- calibration.oot_detected (triggers suspect product trace)
- calibration.status_changed (in-cal / due / overdue)

**Events Consumed:**
- quality.inspection_completed (identifies which instruments were used)

**Cross-references:** Quality tab (instrument validity for inspections, OOT → NCR), Maintenance tab (calibration is a form of instrument maintenance)

**Build Phase:** C (standalone, can parallelize with Inspection)

---

### Tab 6: Equipment Maintenance (GREEN)

**Lane border:** Green (module exists)
**Crates:** maintenance

**Process Flow (Swim Lane 1 — PM Lifecycle):**
Create Maintenance Plan → Assign to Assets → Scheduler Detects Due (calendar or meter) → Generate PM Work Order → Draft → Approval → Scheduled → In Progress ⇄ On Hold → Completed → Closed

**Process Flow (Swim Lane 2 — Corrective & Downtime):**
Equipment Breakdown (from Production) → Record Downtime Event → Create Corrective WO → Repair → Complete → Track Impact Classification

**Data Entities Panel (Existing):**
- MaintenancePlan — schedule types (calendar/meter/both), task checklist, priority
- PlanAssignment — links plans to assets, tracks next_due_date, last_meter_reading
- WorkOrder (Maintenance) — 8-state lifecycle, types: Preventive/Corrective/Inspection
- Asset — types: Vehicle/Machinery/Equipment/Facility, status: Active/Inactive/Retired
- DowntimeEvent — immutable, impact_classification (none/minor/major/critical), workcenter_id
- MeterType + MeterReading — meter-based scheduling
- WO Parts + Labor — resource tracking per work order

**RETROFIT NEEDED (BLUE):**
- Workcenter Master table — currently just a UUID on DowntimeEvent, needs real entity
- Capacity units, calendars, machine-to-workcenter association
- Production module will own Workcenter; Maintenance consumes via events/API

**Events Published:**
- maintenance.work_order.created/status_changed/completed/closed/cancelled/overdue
- maintenance.plan.due, maintenance.plan.assigned
- maintenance.asset.created/updated/out_of_service_changed
- maintenance.downtime.recorded
- maintenance.meter_reading.recorded
- maintenance.calibration.created/completed/event_recorded/status_changed

**Events Consumed:**
- (future) production.equipment_breakdown → triggers corrective WO

**Cross-references:** Production tab (workcenter availability, breakdown → downtime), Calibration tab (instrument maintenance overlap), People tab (technician qualifications)

---

### Tab 7: People & Training (GREEN)

**Lane border:** Green (modules exist)
**Crates:** workforce-competence, timekeeping

**Process Flow (Swim Lane 1 — Competence Management):**
Register Competence Artifact (cert/training/qualification) → Assign to Operator → Track Expiry → Renewal Workflow → Authorization Check at Point-of-Use

**Process Flow (Swim Lane 2 — Time & Labor):**
Clock In → Record Time Entry (project/task allocation) → Approval → Correct/Void if needed → Labor Cost → GL (cost allocation) + AR (billable time)

**Data Entities Panel (Existing — Workforce-Competence):**
- CompetenceArtifact — types: certification, training, qualification, with validity_duration
- OperatorCompetence — artifact-to-operator assignment, award_date, expiry tracking, revocable
- AcceptanceAuthority — authorization scope for operator approvals

**Data Entities Panel (Existing — Timekeeping):**
- TimesheetEntry — append-only (original/correction/void), project/task allocation
- Employee — employee master
- Project / Task — work allocation context
- Approval — entry approval workflow

**Events Published (Workforce-Competence):**
- workforce_competence.artifact_registered
- workforce_competence.competence_assigned
- workforce_competence.acceptance_authority_granted/revoked

**Events Published (Timekeeping):**
- Outbox-based labor cost events → GL, AR

**Cross-references:** Production tab (operator qualifications for operations), Quality tab (inspector authorization for inspections), Maintenance tab (technician qualifications), Fulfillment tab (labor cost → GL)

---

### Tab 8: Fulfillment & Finance (GREEN)

**Lane border:** Green (modules exist)
**Crates:** shipping-receiving (outbound), ar, payments, gl

**Process Flow (Swim Lane 1 — Ship-to-Cash):**
FG in Inventory → Create Outbound Shipment → Picking → Packed → Ship to Customer → Deliver → Invoice Customer (AR) → Collect Payment → GL Journal Entry

**Process Flow (Swim Lane 2 — Cost Accounting):**
Material Issue (Inventory FIFO) → WIP Cost Accumulation → FG Receipt (rolled-up cost) → COGS on Sale → Revenue Recognition → GL Posting

**Data Entities Panel (Existing):**
- Shipment (S-R) — outbound: Draft→Confirmed→Picking→Packed→Shipped→Delivered→Closed
- ShippingDocument (S-R) — packing slip, bill of lading
- Invoice (AR) — draft/open/paid/void, with hosting support
- Customer (AR) — with Tilled payment processor link
- Charge / Refund (AR) — payment processing
- JournalEntry + JournalLine (GL) — posted entries, debit/credit pairs
- Account (GL) — chart of accounts (asset/liability/equity/revenue/expense)
- Period (GL) — accounting period close workflow
- FxRate (GL) — currency exchange and revaluation

**Events Published:**
- shipping_receiving.outbound_shipped, shipping_receiving.outbound_delivered
- ar.invoice_opened, ar.invoice_paid, ar.credit_memo_created
- ar.usage_captured, ar.usage_invoiced
- gl.accrual_created, gl.accrual_reversed
- fx.rate_updated, gl.fx_revaluation_posted

**Events Consumed:**
- inventory.item_issued (COGS cost layers)
- production.fg_received (FG costing)
- ap.vendor_bill_approved (expense posting to GL)
- timekeeping.* (labor cost allocation to GL)

**Cross-references:** Production tab (FG receipt → ship, WIP→FG cost), Procurement tab (AP → GL), People tab (labor cost → GL)

---

### Tab 9: Sales Cycle (RED — Gap)

**Lane border:** Red (no modules exist)
**Crates:** none

**Process Flow (Envisioned):**
RFQ Received → Evaluate (BOM costing, capacity check) → Quote / Estimate → Customer Accepts → Sales Order → Order Review (material + capacity check) → Triggers: BOM review + PO creation + WO creation

**What's Needed:**
- RFQ management (capture, track, respond)
- Quoting engine (BOM cost rollup + margin + lead time)
- Sales order management (customer PO, lines, delivery dates)
- Order review workflow (material availability + shop capacity)

**Data Entities (Conceptual):**
- RequestForQuote — customer inquiry, line items, due date
- Quote — estimated cost, margin, lead time, validity period
- SalesOrder — customer PO reference, lines, delivery schedule
- OrderReview — material check + capacity check results

**Scope Decision Needed:**
- Platform or app-specific? The overview chart flags this as "WhiteValley needs to weigh in"
- RFQ/Quoting might be app-specific (aerospace quoting ≠ food manufacturing quoting)
- Sales Order might be platform (generic order-to-production trigger)

**Cross-references:** Engineering tab (BOM for costing), Production tab (order → WO), Procurement tab (order → PO)

---

### Tab 10: Post-Sale (RED + GREEN + AMBER mix)

**Lane border:** Red/Green mix
**Crates:** shipping-receiving (RMA exists), (proposed: quality-ncr-rs for NCR from RMA)

**Process Flow (Swim Lane 1 — Customer Returns):**
Customer Complaint → (Gap: no Customer Support module) → RMA Created (S-R) → Receive Return → Inspect → Disposition (Return to Stock / Scrap / Repair)

**Process Flow (Swim Lane 2 — Quality Feedback):**
RMA Inspection → NCR from RMA (links to Quality NCR/CAPA) → Root Cause → CAPA → Update Inspection Plans

**Data Entities Panel (Existing — S-R):**
- RMA — return authorization with receipt items
- RmaReceipt / RmaReceiptItem — returned goods, SKU, quantity, condition_notes
- Disposition: Received → Inspect → Quarantine → Return-to-Stock or Scrap

**Gap: Customer Support**
- Case management, warranty tracking — no module exists
- Platform or app-specific? Needs decision.

**Events (Existing):**
- shipping_receiving.shipment_created (RMA receipt)
- shipping_receiving.inbound_closed (RMA fully received)

**Events (Proposed link to NCR):**
- quality.ncr_created (triggered by RMA inspection failure)

**Cross-references:** Quality tab (NCR/CAPA from RMA), Procurement tab (Return to Vendor disposition), Fulfillment tab (credit memo from return)

---

## What I Need From Reviewers

1. **Module boundaries** — Are the entities assigned to the right modules?
2. **Missing integration points** — Any cross-module flows I've missed?
3. **Green module accuracy** — Do the entities/events match what's actually in the codebase?
4. **Amber module feasibility** — Are the proposed data entities reasonable for the stated scope?
5. **Red module scope** — Where should the platform/app boundary be for Sales and Customer Support?
6. **Tab layout** — Any content that should be added or removed from the drill-down views?

**Please reply with: APPROVED (with any notes) or CHANGES REQUESTED (with specifics).**
