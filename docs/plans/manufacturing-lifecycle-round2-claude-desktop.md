# Manufacturing Lifecycle — Round 2 Verdicts — Claude Desktop

**Date:** 2026-03-04
**Reviewer:** Claude Desktop (Cowork)

---

## Proposed Changes (1–13)

### CHANGE 1: Purchase Orders box — RED to GREEN
**AGREE.** I verified this in Round 1. AP has `src/domain/po/`, HTTP endpoints, events (`ap.po_created`, `ap.po_approved`, `ap.po_closed`), PO-to-receipt linking via `src/domain/receipts_link/`, three-way match via `src/domain/match/engine.rs`, and an inventory receipt consumer at `src/consumers/inventory_item_received.rs`. This is not a gap — it's a built, functioning module domain. Green, labeled "Purchase Orders (AP)".

### CHANGE 2: Remove "Inventory Retrofit" box from the process flow
**AGREE.** A lifecycle diagram should show business process steps, not development work items. The retrofit work (make/buy classification, production entry types) is real and important but belongs in the implementation plan, not the process flow. A blue annotation on "Store in Inventory" is the right treatment.

### CHANGE 3: Remove the Inventory Retrofit → BOM arrow
**AGREE.** This arrow is logically backwards. Materials don't cause BOMs to exist. BOMs are created by engineering and drive procurement. Removing this arrow is a prerequisite for Changes 4 and 5 to make sense.

### CHANGE 4: Add Order Review → BOM arrow
**AGREE.** This is the correct upstream trigger for engineering. In make-to-order manufacturing, a sales order triggers BOM review or creation. Without this arrow, the Engineering lane has no input — BOM appears to spontaneously generate itself.

### CHANGE 5: Add BOM → Purchase Orders arrow
**AGREE.** BOM explosion determines what raw materials and components need to be procured. This is the fundamental engineering-to-procurement link. It's the single most important cross-lane arrow that's missing from the current diagram.

### CHANGE 6: Add BOM → Work Orders arrow (confirm existing)
**AGREE.** The arrow already exists in the draw.io source as `a-bom-wo` (source: `step-bom`, target: `step-wo`). I confirmed this during my Round 1 parse of the XML. It's present and correct. No action needed unless it's visually obscured — in which case, make it more prominent (thicker stroke or bolder color).

### CHANGE 7: Add Receiving Inspection step
**AGREE.** This was my top finding in Round 1. The platform already has this built in Shipping-Receiving: `inspection_routing` service with `direct_to_stock` and `send_to_inspection` types, plus dedicated HTTP endpoint at `src/http/inspection_routing.rs`. The box should be green (capability exists) with a branch: pass → store, fail → quarantine → NCR. Three reviewers independently flagged this.

### CHANGE 8: Reconnect AP from Receive Materials or Store in Inventory
**MODIFY.** The connection should be from **Store in Inventory** to AP, not from Receive Materials. The reason: AP's three-way match requires the goods receipt record (which is created when materials are stored in inventory), not just the receiving dock event. The flow is: Receive → Inspect → Store → AP matches (PO + receipt + vendor invoice). Connecting AP directly from "Receive Materials" skips the inspection and storage steps that generate the receipt record AP needs.

If we're being precise: AP triggers on the `inventory.item_received` event (AP already has a consumer for this: `src/consumers/inventory_item_received.rs`). So the arrow should originate from the Store in Inventory box.

### CHANGE 9: Add MRB disposition outcome arrows
**AGREE.** The MRB box is currently a dead end — problems go in but nothing comes out. All four paths should be shown:
- Repair/Rework → Production (rework WO)
- Use-As-Is → FG Receipt (accept with deviation)
- Scrap → Inventory (adjustment/write-off)
- Return to Vendor → AP (debit memo, replacement PO)

These are the four standard MRB dispositions. Each has a distinct downstream module owner — that's exactly the kind of integration point this diagram should surface.

### CHANGE 10: Add Quarantine/Hold state before MRB Disposition
**MODIFY.** The concept is correct — suspect material should be quarantined before MRB reviews it. But adding a box risks cluttering the Quality lane. Inventory already has quarantine as a status bucket (available / quarantine / damaged), and the NCR creation itself implies a hold. I'd handle this as an annotation on the NCR/CAPA box: "Material quarantined pending MRB review" with a dashed arrow to Inventory's quarantine status. This keeps the Quality lane at three boxes (Inspection Plans, NCR/CAPA, MRB Disposition) while making the hold state explicit.

### CHANGE 11: Add FG Receipt → GL arrow
**AGREE.** This is the WIP → Finished Goods cost transfer — one of the most important journal entries in manufacturing accounting. Without this arrow, the diagram shows goods being produced but never hitting the financial statements. The arrow should be a dashed cross-lane line from FG Receipt down to GL, labeled "cost transfer" or "WIP → FG posting".

### CHANGE 12: Add Operations → Timekeeping arrow
**AGREE.** Production labor collection is distinct from payroll timekeeping but feeds into it. Operation-level labor (operator X worked 2.5 hours on operation 30 of WO-2026-003) flows to Timekeeping for payroll aggregation and to GL for cost accumulation. This is a real integration point that should be visible.

### CHANGE 13: Add OOT Disposition → Inspection Plans arrow
**AGREE.** This is the "suspect product trace" — when an instrument is found out of tolerance, you must look up every lot/serial inspected with that instrument since the last known-good calibration. The OOT → NCR arrow already exists (from Round 1), but without this arrow back to Inspection Plans, there's no way to identify which product is affected. It's what makes OOT findings actionable rather than just recorded.

---

## Verdicts Summary (Changes 1–13)

| Change | Verdict | Notes |
|--------|---------|-------|
| 1 | **AGREE** | PO exists in AP, verified |
| 2 | **AGREE** | Not a process step |
| 3 | **AGREE** | Logically backwards |
| 4 | **AGREE** | Correct upstream trigger |
| 5 | **AGREE** | Critical missing link |
| 6 | **AGREE** | Already exists, confirm visible |
| 7 | **AGREE** | Already built in S-R, green box |
| 8 | **MODIFY** | Arrow from Store in Inventory → AP, not Receive Materials → AP |
| 9 | **AGREE** | Four disposition paths needed |
| 10 | **MODIFY** | Annotation on NCR/CAPA box, not a separate box |
| 11 | **AGREE** | WIP → FG cost transfer |
| 12 | **AGREE** | Production labor → payroll |
| 13 | **AGREE** | Suspect product trace |

---

## Additional Items (A–I)

### A. Expand MRB options: Sort/100% Screen, Reclassify/Downgrade
**PROMOTE.** Sort/100% Screen is a common fifth disposition in manufacturing — segregate the batch and inspect every unit to separate good from bad. It's distinct from Repair (no rework needed) and Use-As-Is (not accepting the whole lot). Reclassify/Downgrade (accept at lower spec for different end-use) is also standard. Both are real dispositions that affect downstream module flows. Adding them to the MRB box alongside Change 9 is minimal additional effort.

### B. ECO → Document Control arrow for released drawings
**PROMOTE.** Engineering change orders produce updated drawings and specifications that must be revision-controlled. Doc-Mgmt already exists as a platform crate (`platform/doc-mgmt`). The ECO → Document Control connection is how released engineering documents get versioned and distributed. This is a natural cross-lane arrow that should be on the diagram — especially since the Document Control box already exists in the Engineering lane.

### C. Calibration Records → Doc Mgmt storage link
**DEFER.** Correct in principle (calibration certificates are documents that need retention), but it's a storage/archival concern, not a process flow. Every module produces documents that could link to Doc Mgmt. Showing this one selectively would imply Calibration is special when it isn't. If we add it, we'd need to add similar arrows from Inspection Records → Doc Mgmt, NCR Records → Doc Mgmt, etc. Better to note Doc Mgmt as cross-cutting infrastructure in the right-side info panel.

### D. Subcontracting/outside processing step
**DEFER.** Subcontracting (send material to an outside processor for heat treat, plating, etc., then receive it back) is a real manufacturing process that's absent from the diagram. However, it's a significant scope addition — it involves Shipping-Receiving (outbound to vendor, inbound return), AP (vendor billing for processing), and Production (operation routing to external workcenter). This is better handled as a Phase 2 extension to Production rather than adding it to the lifecycle diagram now. Including it prematurely would suggest it's in scope for the initial build.

### E. Traveler/work instructions callout in Production
**DEFER.** Work instructions / travelers are important on the shop floor, but they're a document type (handled by Doc Mgmt) rather than a process step. The Operations box already implies that operators follow instructions. Adding a callout risks cluttering the Production lane. This is an implementation detail for the Production module's design doc, not the lifecycle overview.

### F. Traceability thread notation for lot/serial flow
**DEFER.** Lot/serial traceability is critical in manufacturing and the platform supports it (Inventory has full lot/serial tracking, genealogy via split/merge, and serial lifecycle). But representing traceability as a thread across the diagram would be a different visualization (a data flow overlay) rather than a process flow change. It would complicate the diagram without adding process clarity. Better as a separate traceability diagram or as a callout in the Inventory module's vision doc.

### G. NCR/CAPA → Inspection Plans feedback loop
**PROMOTE.** When a CAPA closes, it often results in updated inspection plans — tighter tolerances, additional characteristics, changed sampling rules. This is the continuous improvement loop that manufacturing quality systems depend on. Without it, the diagram shows problems being found and fixed but never feeding back into prevention. A dashed arrow from NCR/CAPA back to Inspection Plans & Records closes the loop and is architecturally important (Quality-Nonconformance emits events that Quality-Inspection consumes to update plans).

### H. Production owns Workcenter → Maintenance consumes arrow
**PROMOTE.** This was a key finding in my Round 1 review. The diagram shows Workcenter Master as a blue (retrofit) box in the Maintenance lane, but the architectural decision is that Production owns the workcenter master and Maintenance consumes it. There should be a dashed arrow from Workcenter Master (moved to or referenced from the Production lane) down to Maintenance's Downtime Tracking. This makes the ownership boundary visible on the diagram, which is exactly what a lifecycle chart should do.

### I. BOM → Inventory item reference arrow
**DEFER.** Correct that BOM references Inventory items, but this is a data reference (BOM component → item_id FK), not a process flow. Every module that references items has this relationship. Showing it for BOM but not for Production, Shipping-Receiving, etc. would be inconsistent. The BOM → Purchase Orders arrow (Change 5) already implies the material connection. The item reference is an implementation detail for the BOM module's contract, not a lifecycle process step.

---

## Additional Items Summary

| Item | Verdict | Reason |
|------|---------|--------|
| A | **PROMOTE** | Standard MRB dispositions, minimal effort to add |
| B | **PROMOTE** | ECO → Doc Control is a natural existing connection |
| C | **DEFER** | Storage concern, not process flow; would need many similar arrows |
| D | **DEFER** | Real but significant scope; Phase 2 extension |
| E | **DEFER** | Implementation detail, not process step |
| F | **DEFER** | Different visualization type; separate diagram |
| G | **PROMOTE** | Closes the continuous improvement loop |
| H | **PROMOTE** | Makes ownership boundary visible |
| I | **DEFER** | Data reference, not process flow |

---

*Round 2 complete. 11 AGREE, 2 MODIFY, 4 PROMOTE, 5 DEFER.*
