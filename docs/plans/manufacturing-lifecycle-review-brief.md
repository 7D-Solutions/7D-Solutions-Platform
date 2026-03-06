# Manufacturing Lifecycle Flowchart — Review Brief

**File:** `docs/plans/manufacturing-lifecycle.drawio`
**Prepared by:** BrightHill (Orchestrator)
**Date:** 2026-03-04

## What This Is

A comprehensive draw.io flowchart mapping the entire manufacturing lifecycle from RFQ to RMA, color-coded by module status:

- **Green** = Platform module exists and is built
- **Amber** = Proposed new module (from manufacturing review)
- **Red** = Gap — nothing exists yet
- **Blue** = Existing module needs retrofit

## Swim Lanes (top to bottom)

### 1. Sales Cycle (red border — gaps)
RFQ Received → Quote/Estimate → Sales Order → Order Review

### 2. Procurement & Materials (green border — mostly exists)
Purchase Orders → Receive Materials → Store in Inventory → Inventory Retrofit → Accounts Payable

- **Purchase Orders** is a gap (red box) — no module yet
- **Inventory Retrofit** is blue — needs make/buy classification, production issue/receipt movement types

### 3. Engineering (amber border — proposed)
BOM → ECO/Change Control → Document Control

- BOM and ECO are proposed (amber)
- Document Control already exists (green)

### 4. Production (amber border — proposed, two-row layout)
**Row 1 (making):** Work Orders → Material Issue → Operations
**Row 2 (checking & receiving):** In-Process Checks → Final Inspection → FG Receipt

- Operations box shows interleaved quality: "Machine → Check → Weld → Check → Heat Treat → Check → Assembly → Final Test"
- In-Process Checks and Final Inspection have **green borders** = Quality-Inspection module touchpoints embedded in Production
- FG Receipt is blue (needs inventory retrofit)

### 5. Quality Management (amber border — proposed)
Inspection Plans & Records → NCR/CAPA → **MRB Disposition**

- Inspection Plans = governance layer (defines checks, sampling, acceptance criteria)
- NCR/CAPA = proposed, later phase
- **MRB Disposition** (new) shows three standard options:
  - Repair / Rework
  - Use-As-Is (concession)
  - Scrap / Return to Vendor

### 6. Calibration (amber border — standalone optional module)
Instrument Records → Calibration Schedule → Calibration Records & Certificates → OOT Disposition

- Separate from Maintenance (different domain: measurement accuracy vs machine upkeep)
- Separate from Quality (serves quality but is its own bounded context)
- **OOT Disposition → NCR/CAPA** feedback arrow: when an instrument is found out of tolerance, trace back suspect product and create NCRs

### 7. Equipment Maintenance (green border — exists)
Preventive Maintenance → Downtime Tracking → Workcenter Master → Repair/Corrective WOs

- Workcenter Master is blue (needs retrofit — owned by Production, consumed by Maintenance)

### 8. People & Training (green border — exists)
Workforce Competence → Operator Qualifications → Inspector Authorization → Timekeeping

### 9. Fulfillment & Finance (green border — exists)
Ship to Customer → Invoice → Collect Payment → General Ledger

### 10. Post-Sale (red border — gaps)
Customer Support → RMA → NCR from RMA

## Cross-Lane Arrows (right-margin corridors to avoid overlap)

| Arrow | Route | Style |
|-------|-------|-------|
| FG Receipt → Ship | x=1200 corridor | Solid gray |
| Operations → Downtime | x=1250 corridor | Dashed green (breakdown triggers maintenance) |
| Operator Quals → Operations | x=1300 corridor | Dashed green (must be qualified) |
| Inspector Auth → Inspection Plans | x=1350 corridor | Dashed green (must be authorized) |
| OOT Disposition → NCR/CAPA | Direct | Dashed amber (OOT triggers NCR) |
| Calibration Schedule → Inspection Plans | Direct | Dashed green (cal validates inspection equipment) |
| Inspection fails → NCR | Direct | Dashed red |

## Right-Side Info Panels

1. **Infrastructure (Cross-Cutting):** Identity/Auth, NATS, Numbering, Notifications, Workflow, Party, Reporting, Subscriptions
2. **Missing Modules:** RFQ, Quoting, Sales Orders, Purchase Orders, Customer Support
3. **Proposed Modules (Reviewed):** BOM, Production, Quality-Inspection, Quality-NCR/CAPA, Calibration, MRP (rejected)

## Key Architectural Decisions Embedded

1. Quality inspection is **embedded in the Production flow** (in-process checks between operations), not a separate gate after production
2. Quality Management lane is **governance** (plans, records, NCR/CAPA), not execution
3. Calibration is **standalone optional** — not part of Maintenance or Quality
4. OOT findings **trigger NCRs** for suspect product traceability
5. MRB disposition has three paths: repair, use-as-is, scrap
6. Workcenter master **owned by Production**, consumed by Maintenance
7. No MRP module — rejected, Fireproof builds their own planning against BOM/Inventory APIs

## What to Review

1. **Completeness** — Are any major process steps missing from the lifecycle?
2. **Accuracy** — Are the module assignments correct (green/amber/red/blue)?
3. **Flow logic** — Do the arrows represent the correct process flow?
4. **Cross-lane relationships** — Are all important integration points shown?
5. **MRB Disposition** — Are the three options correct? Missing any standard dispositions?
6. **Calibration → NCR feedback** — Is this modeled correctly?
7. **Procurement flow** — Does PO → Receive → Store → Inv Retrofit → AP make sense?
8. **Anything from your domain expertise** that would improve accuracy

## Response Format

Reply with:
- **APPROVED** if no issues found
- **SUGGESTIONS** with specific items if you have improvements
- **BLOCKED** with reason if something is fundamentally wrong
