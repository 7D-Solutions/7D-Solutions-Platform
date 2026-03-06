# Manufacturing Lifecycle Flowchart — Review by Claude Desktop

**Date:** 2026-03-04
**Reviewer:** Claude Desktop (Cowork)
**Artifact reviewed:** `docs/plans/manufacturing-lifecycle.drawio`
**Brief:** `docs/plans/manufacturing-lifecycle-review-brief.md`

---

## Verdict: SUGGESTIONS

The diagram is a strong first draft that captures the end-to-end manufacturing lifecycle well. The swim lane structure is clear, the color coding is effective, and the cross-lane relationships surface real integration points. However, there are two factual errors in module assignments, one significant missing process step, and several flow logic issues that should be corrected before this goes to the implementation team.

---

## 1. Completeness — Missing Process Steps

### CRITICAL: Receiving Inspection is missing

Between "Receive Materials" and "Store in Inventory" there must be a **Receiving Inspection** step. In manufacturing — especially regulated manufacturing — materials do not go directly from the dock to stock. They are inspected against the PO specification and either accepted (direct to stock) or held (quarantine for further inspection).

This is not hypothetical. The platform already has this hook built: Shipping-Receiving has `inspection_routing` with types `direct_to_stock` and `send_to_inspection`, a full inspection routing service (`src/domain/inspection_routing/service.rs`), and a dedicated HTTP endpoint (`src/http/inspection_routing.rs`). The diagram should show this existing capability.

**Fix:** Add a green-bordered box "Receiving Inspection" between "Receive Materials" and "Store in Inventory", with a dashed arrow to Quality-Inspection (Inspection Plans & Records) for the inspection criteria lookup. Add a branch: pass → store, fail → quarantine → NCR.

### MISSING: Rework loop from MRB Disposition

The MRB Disposition box shows three options (Repair/Rework, Use-As-Is, Scrap/Return to Vendor) but none of them have outbound arrows showing where they go. This is the most important branching point in the quality flow:

- **Repair/Rework** → arrow back up to Production (create a rework work order)
- **Use-As-Is (concession)** → arrow to Finished Goods Receipt (accept with deviation)
- **Scrap** → arrow to Inventory (adjustment/write-off, GL cost posting)
- **Return to Vendor** → arrow to AP/Procurement (debit memo, replacement PO)

Without these arrows the diagram shows problems being identified but not resolved.

### MISSING: Production cost flow to GL

No arrow exists from Production to General Ledger. Manufacturing cost accounting is a core financial flow: material cost (from Inventory FIFO layers) + labor cost (from Production operation tracking) + overhead = finished goods unit cost. This rolled-up cost posts to GL as WIP → Finished Goods → COGS. There should be a dashed arrow from the Production lane to GL, and ideally from "Finished Goods Receipt" to GL as well (the cost transfer entry).

### MISSING: Production labor → Timekeeping

The People & Training lane shows Timekeeping but there is no arrow from Production operations to Timekeeping. Production labor collection (who worked on which operation, for how long) needs to flow to Timekeeping for payroll and to GL for cost accumulation. Add a dashed cross-lane arrow from Operations → Timekeeping.

---

## 2. Accuracy — Module Assignment Errors

### ERROR: Purchase Orders should be GREEN, not RED

The diagram shows "Purchase Orders" as a red (gap) box. This is incorrect. The AP module already has full purchase order support:

- `src/domain/po/` — PO domain with create, approve, close lifecycle
- `src/http/purchase_orders.rs` — HTTP endpoints
- `src/events/po.rs` — Events: `ap.po_created`, `ap.po_approved`, `ap.po_closed`
- `src/domain/receipts_link/` — PO-to-receipt linking
- `src/domain/match/engine.rs` — Three-way match (PO + receipt + invoice)
- `src/consumers/inventory_item_received.rs` — Consumes inventory receipt events for PO linking

Purchase Orders should be a **green box** labeled "Purchase Orders (AP module)". The only thing AP doesn't do is purchase requisition / request-for-quote, which could be called out as a separate red gap if needed.

### ERROR: "Inventory Retrofit" is not a process step

"Inventory Retrofit" appears as a blue box in the procurement flow between "Store in Inventory" and "Accounts Payable." But a retrofit is a development task, not a business process step. Users don't perform an "inventory retrofit" — they classify items, receive production output, issue components, etc.

**Fix:** Remove the "Inventory Retrofit" box from the process flow. Instead, add a blue annotation or note on the "Store in Inventory" box indicating that retrofit work is needed (make/buy classification, production entry types). The actual process flow should be: Receive → Inspect → Store → (AP three-way match). The retrofit details belong in the review document, not the lifecycle diagram.

### MINOR: RMA box should be green with existing disposition model

The RMA box is shown as green, which is correct — Shipping-Receiving has a full RMA domain with a 5-state disposition model (received → inspect → quarantine → return_to_stock / scrap). However, the brief describes RMA disposition as "mirroring NCR flow." Worth noting that S-R's RMA disposition is already more specific than the Quality NCR disposition and could serve as a reference implementation.

---

## 3. Flow Logic — Arrow Issues

### The procurement-to-engineering flow is backwards

Currently the diagram shows: `Inventory Retrofit → BOM` (arrow `a-retro-bom`). This implies that storing materials leads to creating a BOM, which is backwards. In manufacturing, the flow is:

1. **Sales Order → Order Review** triggers both engineering review and procurement
2. **Engineering** designs or reviews the BOM
3. **BOM explosion** generates the material requirements
4. **Procurement** buys what's needed (PO created from BOM demand)
5. **Materials arrive** and are received into inventory

The correct flow is: Order Review → BOM (parallel with or before PO), and BOM → PO (BOM drives what to buy). The current `retro-bom` arrow should be replaced with `review → bom` and `bom → po`.

**Fix:**
- Add arrow: Order Review → BOM (engineering review triggered by order)
- Add arrow: BOM → Purchase Orders (BOM drives procurement)
- Remove arrow: Inventory Retrofit → BOM
- Keep arrow: BOM → Work Orders (BOM drives production)

This gives the correct make-to-order flow: Order → Design → Buy + Make → Ship.

### The BOM → Work Orders arrow skips a step

`step-bom → step-wo` is correct in concept but oversimplified. In practice, BOM defines what to make; a planning step (manual or MRP) decides when and how many to make; then work orders are created. Since MRP is deferred, this is acceptable for now, but a note or annotation acknowledging the planning gap would be helpful.

---

## 4. Cross-Lane Relationships

### Present and correct
- Operations → Downtime Tracking (production breakdowns trigger maintenance)
- Operator Qualifications → Operations (must be qualified to work)
- Inspector Authorization → Inspection Plans (must be authorized to inspect)
- Calibration Schedule → Inspection Plans (calibrated equipment validates inspection)
- OOT Disposition → NCR/CAPA (out-of-tolerance triggers nonconformance)
- In-Process Checks → Inspection Plans (quality governance over production checks)
- Final Inspection → Inspection Plans (quality governance over final inspection)

### Missing cross-lane arrows

| From | To | Why |
|------|----|-----|
| Receiving Inspection | Inspection Plans & Records | Incoming inspection governed by inspection plans |
| Operations | Timekeeping | Labor hours from production flow to payroll |
| MRB Disposition (Repair) | Work Orders | Rework disposition creates a production work order |
| MRB Disposition (Scrap) | Inventory (adjustment) | Scrap disposition triggers inventory write-off |
| MRB Disposition (Return) | AP / Purchase Orders | Return-to-vendor triggers debit memo |
| Finished Goods Receipt | General Ledger | WIP → FG cost transfer posting |
| NCR from RMA | NCR/CAPA | RMA-sourced NCRs feed into quality management (arrow shown in brief but not verified in draw.io — confirm it exists) |
| BOM | Inventory | BOM references inventory items (dashed, data reference) |

---

## 5. MRB Disposition

### The three options are correct but incomplete

The standard MRB (Material Review Board) disposition options in manufacturing are:

1. **Repair / Rework** — Fix the nonconformance and re-inspect. ✅ Shown.
2. **Use-As-Is (concession)** — Accept with documented deviation. ✅ Shown.
3. **Scrap** — Destroy the material, write off the cost. ✅ Shown.
4. **Return to Vendor** — Send back to supplier for credit/replacement. ✅ Shown (grouped with Scrap).

**Missing disposition:**
5. **Reclassify / Downgrade** — Accept at a lower specification or for a different end-use. This is common in manufacturing where a part doesn't meet spec A but is acceptable for spec B. It's distinct from "Use-As-Is" because the item's classification changes.

**Suggestion:** Either add "Reclassify" as a fourth MRB option, or explicitly note that it falls under "Use-As-Is" for the platform's purposes. Both are defensible — just be deliberate about it.

**Also:** Return to Vendor should be separated from Scrap visually. They have different downstream flows: Scrap triggers an inventory adjustment and GL write-off; Return to Vendor triggers a shipment back to the supplier, a debit memo in AP, and potentially a replacement PO. Grouping them understates the work.

---

## 6. Calibration → NCR Feedback

### Correctly modeled

The OOT Disposition → NCR/CAPA arrow (dashed amber) correctly captures the critical feedback loop: when calibration discovers an instrument is out of tolerance, all product inspected with that instrument since the last known-good calibration is suspect and requires investigation via NCR.

**One enhancement:** Add a dashed arrow from OOT Disposition back to Inspection Plans & Records, representing the "suspect product trace" — the system needs to query inspection records to find which lots/serials were inspected with the out-of-tolerance instrument. This is a data lookup, not a process flow per se, but it's the critical operation that makes OOT → NCR actionable. Without it, you know the instrument is bad but you don't know which product is affected.

---

## 7. Procurement Flow

### Current flow has issues

The current flow is: PO → Receive → Store → Inv Retrofit → AP

**Problems:**
1. "Inventory Retrofit" is not a process step (addressed above)
2. Missing Receiving Inspection between Receive and Store (addressed above)
3. The AP step is correct but undersells what happens — AP performs three-way match (PO + receipt + vendor invoice) before approving payment

**Corrected flow should be:**

```
Purchase Orders (AP) → Receive Materials (S-R) → Receiving Inspection (S-R + Quality)
    → Pass: Store in Inventory
    → Fail: Quarantine → NCR
Store in Inventory → AP Three-Way Match → Vendor Payment
```

This accurately represents the procure-to-pay cycle in manufacturing and uses modules that already exist (AP has PO + three-way match, S-R has receiving + inspection routing, Inventory has quarantine status buckets).

---

## 8. Additional Domain Observations

### Calibration as a standalone module — agree, with a caveat

The diagram shows Calibration as a standalone optional module separate from both Maintenance and Quality. This is the right call. Calibration is its own bounded context: it has its own assets (instruments, not machines), its own schedule (calibration intervals, not PM intervals), its own records (calibration certificates with measurement uncertainty), and its own failure mode (OOT, not breakdown).

**Caveat:** The workcenter master (owned by Production, consumed by Maintenance) and the instrument master (owned by Calibration) are both "equipment registries." There's a risk that three modules (Production, Maintenance, Calibration) each build their own equipment/asset model. Consider whether Maintenance's existing asset model can serve as the shared reference for both workcenters and instruments, with Production and Calibration adding their domain-specific attributes via events or API extensions.

### Sales Cycle modules — larger gap than shown

The Sales lane (RFQ → Quote → Sales Order → Order Review) is correctly marked as red/gap, but the diagram may understate the scope. Quoting in manufacturing requires BOM costing (material + labor + overhead markup), which means the Quoting module depends on BOM being built first. This creates a circular dependency in build sequencing: you need BOM to do quoting, but you need sales orders to justify building BOM. The practical answer is that BOM ships first and quoting is manual (spreadsheet) until the Quoting module is built.

### The "Operations" box is the diagram's best insight

The operations box showing the interleaved pattern "Machine → Check → Weld → Check → Heat Treat → Check → Assembly → Final Test" correctly captures how quality inspection is embedded in the production flow, not a separate downstream gate. This architectural decision — Quality-Inspection as governance over production checkpoints, not a separate handoff — is the right model and should be preserved in implementation. In-process inspection is a property of a routing operation, not a standalone step.

---

## Summary of Recommended Changes

| # | Type | Change |
|---|------|--------|
| 1 | **Add box** | Receiving Inspection (green) between Receive Materials and Store in Inventory |
| 2 | **Fix color** | Purchase Orders: RED → GREEN (AP module has POs) |
| 3 | **Remove box** | "Inventory Retrofit" — not a process step; annotate Store in Inventory instead |
| 4 | **Fix arrows** | Remove `Inv Retrofit → BOM`; add `Order Review → BOM` and `BOM → Purchase Orders` |
| 5 | **Add arrows** | MRB disposition paths: Repair→Production, Use-As-Is→FG Receipt, Scrap→Inventory, Return→AP |
| 6 | **Add arrow** | Production (FG Receipt or Operations) → GL (cost posting) |
| 7 | **Add arrow** | Operations → Timekeeping (labor flow) |
| 8 | **Add arrow** | OOT Disposition → Inspection Plans (suspect product trace lookup) |
| 9 | **Split box** | MRB: separate "Scrap" from "Return to Vendor" (different downstream flows) |
| 10 | **Add arrow** | BOM → Inventory (item reference, dashed) |
| 11 | **Add arrow** | Receiving Inspection → Quality Inspection Plans (dashed, criteria lookup) |
| 12 | **Add note** | Annotate BOM → WO gap: "Planning step (manual) — MRP deferred" |

None of these are blocking — the diagram's structure and swim lane organization are sound. These are accuracy and completeness refinements.

---

*Review complete. Verdict: SUGGESTIONS — no fundamental issues, 12 specific improvements listed above.*
