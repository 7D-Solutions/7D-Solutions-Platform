Manufacturing Lifecycle Flowchart — Round 2 Consensus Review

All 7 reviewers (PurpleCliff, CopperRiver, DarkOwl, SageDesert, MaroonHarbor, Claude Desktop, ChatGPT) submitted SUGGESTIONS in Round 1. No one blocked. I've consolidated findings into 13 proposed changes below.

YOUR TASK: Review each proposed change and reply with your verdict on EACH item — AGREE, DISAGREE (with reason), or MODIFY (with alternative). This builds consensus before I apply changes.

The draw.io source is at: docs/plans/manufacturing-lifecycle.drawio
The Round 1 brief is at: docs/plans/manufacturing-lifecycle-review-brief.md
Claude Desktop's full review is at: docs/plans/manufacturing-lifecycle-review-claude-desktop.md

=== PROPOSED CHANGES ===

CHANGE 1: Purchase Orders box — RED to GREEN
Claude Desktop found that the AP module already has full PO support: domain (src/domain/po/), HTTP endpoints (src/http/purchase_orders.rs), events (ap.po_created, ap.po_approved, ap.po_closed), PO-to-receipt linking, and three-way match engine. PO is not a gap — it exists in the AP module today. The box should be green and labeled "Purchase Orders (AP module)".
Source: Claude Desktop (verified against codebase)

CHANGE 2: Remove "Inventory Retrofit" box from the process flow
"Inventory Retrofit" is a development task, not a business process step. Users don't perform an "inventory retrofit." The make/buy classification and production movement types are implementation work, not a step in the lifecycle. Instead, add a blue annotation/note on the "Store in Inventory" box indicating retrofit work is needed.
Source: Claude Desktop, DarkOwl

CHANGE 3: Remove the Inventory Retrofit → BOM arrow (a-retro-bom)
This arrow implies that storing materials leads to creating a BOM, which is backwards. BOM exists before procurement and production. This arrow has no real-world equivalent.
Source: Claude Desktop, CopperRiver

CHANGE 4: Add Order Review → BOM arrow (cross-lane, Sales → Engineering)
When a customer order comes in, engineering reviews or creates the BOM for the product. This is the trigger that connects the sales cycle to engineering. Without this arrow, BOM floats with no upstream trigger.
Source: Claude Desktop, SageDesert, CopperRiver

CHANGE 5: Add BOM → Purchase Orders arrow (cross-lane, Engineering → Procurement)
BOM explosion tells procurement what materials to buy. This is the fundamental link between engineering and procurement. Currently absent from the chart.
Source: Claude Desktop, CopperRiver

CHANGE 6: Add BOM → Work Orders arrow (cross-lane, Engineering → Production)
BOM defines what to make; work orders execute it. This is the most important Engineering → Production link. PurpleCliff called this the critical fix. Currently the arrow exists (a-bom-wo) but reviewers want to confirm it's prominent.
Source: PurpleCliff, CopperRiver (NOTE: this arrow already exists in the chart as a-bom-wo. Confirm it's visible and correct.)

CHANGE 7: Add Receiving Inspection step between Receive Materials and Store in Inventory
In manufacturing, materials don't go from dock to stock. They're inspected first against the PO specification. Claude Desktop verified this capability already exists in Shipping-Receiving (inspection_routing service with direct_to_stock and send_to_inspection types). Should be a green box with a branch: pass → store, fail → quarantine → NCR.
Source: Claude Desktop, MaroonHarbor, ChatGPT

CHANGE 8: Reconnect Accounts Payable from Receive Materials (not Inventory Retrofit)
AP performs three-way match: PO + goods receipt + vendor invoice. This is triggered by receiving goods, not by inventory retrofit. With Inv Retrofit removed (Change 2), AP should connect from Receive Materials or Store in Inventory.
Source: PurpleCliff, CopperRiver, DarkOwl, SageDesert, Claude Desktop

CHANGE 9: Add MRB disposition outcome arrows
The MRB box shows three options but no downstream paths. Add:
- Repair/Rework → back to Production Operations (rework work order)
- Use-As-Is → Finished Goods Receipt (accept with deviation)
- Scrap → Inventory (write-off adjustment)
- Return to Vendor → AP/Purchase Orders (debit memo)
Source: DarkOwl, SageDesert, Claude Desktop, ChatGPT

CHANGE 10: Add Quarantine/Hold state before MRB Disposition
Most shops route failed material into a Hold/Quarantine state before the MRB reviews it. This prevents accidental use of suspect material and makes the disposition auditable. Could be a small box or annotation between NCR and MRB.
Source: ChatGPT

CHANGE 11: Add FG Receipt → General Ledger arrow (cross-lane)
Manufacturing cost accounting: when finished goods are received, the cost transfer (WIP → FG) posts to GL. This is the critical financial flow that connects production to accounting. Currently no Production → GL connection exists.
Source: Claude Desktop, PurpleCliff

CHANGE 12: Add Operations → Timekeeping arrow (cross-lane, Production → People)
Production labor collection (who worked on which operation, for how long) flows to Timekeeping for payroll and to GL for cost accumulation. Currently no connection between Production and People & Training lanes.
Source: Claude Desktop

CHANGE 13: Add OOT Disposition → Inspection Plans arrow (Calibration → Quality)
When an instrument is found out of tolerance, the system needs to query inspection records to find which lots/serials were inspected with the bad instrument. This is the "suspect product trace" lookup that makes the existing OOT → NCR arrow actionable.
Source: Claude Desktop

=== ADDITIONAL ITEMS FOR CONSIDERATION ===

These came from individual reviewers. Not proposing them as changes yet, but flag if you think any should be promoted:

A. Expand MRB options: add Sort/100% Screen, Reclassify/Downgrade (CopperRiver, ChatGPT, Claude Desktop)
B. ECO → Document Control arrow for released drawings (ChatGPT)
C. Calibration Records → Doc Mgmt storage link (ChatGPT)
D. Subcontracting/outside processing step (PurpleCliff)
E. Traveler/work instructions callout in Production (ChatGPT)
F. Traceability thread notation for lot/serial flow (ChatGPT)
G. NCR/CAPA → Inspection Plans feedback loop (DarkOwl)
H. Production owns Workcenter → Maintenance consumes arrow (ChatGPT)
I. BOM → Inventory item reference arrow (Claude Desktop)

=== RESPONSE FORMAT ===

For each of the 13 changes: AGREE, DISAGREE (with reason), or MODIFY (with alternative).
For items A-I: PROMOTE (should be a change) or DEFER (leave for later).
