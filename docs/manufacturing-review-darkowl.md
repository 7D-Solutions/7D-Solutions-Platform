# Manufacturing Modules Scope Review - DarkOwl

## 1. Executive Summary
The 4-module proposal is directionally right, but the proposed scope is too broad for a first platform release. BOM and Production should be standalone modules; Quality should be split into a narrow core now and advanced QMS later. With five implementation agents, full scope is closer to a 9-12 month program, while a constrained manufacturing MVP can ship in about 3-4 months.

## 2. Answers to A-G

### A. Module Boundaries
The split is mostly correct, but Quality is overloaded.

- `BOM` should stay separate from Inventory. Versioned multi-level structure, effectivity windows, and where-used are graph concerns, not item-master concerns.
- `Production` should stay separate from Maintenance. Production order orchestration is not maintenance work-order management.
- `Quality` is too big as currently defined. It should be split into:
  - `Quality Core` (inspection plans, disposition/NCR basics, inventory quality hold/release integration)
  - `Quality Actions` (full CAPA lifecycle, special process governance, deep compliance workflows)
- `MRP/Planning` is valid as a separate module, but should not be in phase 1.

### B. Build Sequencing
The proposed order (BOM -> Production -> Quality -> MRP) is correct, but the minimum viable stack should start with retrofit work before new modules.

Minimum viable manufacturing stack to unblock Fireproof:

1. Retrofit foundations in existing modules:
- Inventory: make/buy flag, production issue/receipt movement types, WIP/quarantine handling for production flow
- Maintenance: workcenter master + uptime/downtime usable by production
- Numbering/Workflow: entity templates for BOM revisions and production orders
2. `BOM Lite`: multi-level BOM + revisions + effectivity + where-used
3. `Production Lite`: create/release/close work order, issue/receipt with Inventory, simple operation progress
4. `Quality Core`: receiving/in-process inspections, hold/disposition path only

Defer for later increments:
- full CAPA program
- advanced special process controls
- finite-capacity scheduling and optimizer-grade MRP

### C. Platform vs App-Specific
Boundary is real but currently blurry.

- `Inspection plans` are generic if modeled as reusable characteristics/tolerances/sampling policies, without encoding aerospace standards directly.
- `Special process controls` are partly generic (capture operator qualification, machine certification, process parameter evidence), but rule interpretation/compliance thresholds are often vertical-specific.
- Aerospace-specific rule sets (AS9100/NADCAP/AS9102/ITAR) should remain app-specific plugins/policies, not platform core.

Practical boundary test: if capability is needed by automotive, medical device, and industrial manufacturing with only policy differences, keep it in platform. If value depends on one standard's clause semantics, keep it app-specific.

### D. Existing Module Retrofit
Retrofit effort is likely underestimated.

High-impact retrofit areas:
- Inventory:
  - production transaction semantics (issue-to-WIP, receipt-from-production, scrap, rework)
  - stronger reservation semantics by reference type/state for work orders
  - possible UoM and traceability constraints across BOM vs inventory units
- Maintenance:
  - workcenter master + calendar model required before capacity-aware planning is credible
  - mapping downtime events to planning/dispatch behavior
- Workflow:
  - reusable lifecycle templates can help, but approval/escalation policy design per manufacturing entity is non-trivial
- Workforce-Competence:
  - qualification checks at operation/inspection signoff points need low-latency integration patterns
- Shipping-Receiving:
  - receiving inspection routing should integrate with Quality disposition and inventory hold/release

This is not just additive module work; it is cross-module contract hardening.

### E. Scope and Risk
For five implementation agents:

- Full proposed scope (BOM + Production + full Quality + MRP): approximately 9-12 months if done to platform standards (event contracts, integration tests, migration safety, tenant guarantees).
- Constrained MVP (BOM Lite + Production Lite + Quality Core, no full MRP): approximately 3-4 months.

I would defer:
- full CAPA engine
- special process domain depth beyond evidence capture + qualification checks
- finite-capacity and optimization-heavy planning

### F. Dependencies and Integration
Dependency graph:

- `BOM` -> Inventory, Numbering
- `Production` -> BOM, Inventory, Workflow (plus optional Maintenance integration for workcenter availability)
- `Quality Core` -> Inventory, Workflow, Workforce-Competence, Maintenance (calibration context)
- `MRP/Planning` -> BOM, Inventory, Production

Circular dependency risks:
- Production <-> Quality (quality holds, rework loops)
- Production <-> MRP (execution feedback vs planning outputs)
- MRP <-> Inventory reservations/availability assumptions

Mitigation: keep module boundaries event-driven (state snapshots + commands), avoid synchronous dependency chains across domains.

Must-have day-one integrations:
- Production issue/receipt into Inventory
- BOM revision/effectivity checks at Production order release
- Inventory quality hold/release state consumed by Quality
- Workforce qualification check on critical inspection/operation signoff

Can wire later:
- Maintenance downtime to scheduling optimization
- Shipping/RMA to NCR automation
- MRP-generated procurement suggestions to upstream modules

### G. Alternative Approaches
Some scope can be delivered by extending existing modules, but not all.

- BOM as Inventory extension: only viable for a very limited kit/assembly model. Full revisioned multi-level BOM with where-used should be a separate module.
- Production as Maintenance extension: not recommended. Work-order semantics differ too much.
- Quality as pure Workflow extension: partially viable for lifecycle orchestration, but inspection data model and disposition logic justify a dedicated Quality Core module.
- MRP as standalone now: not required. Start with a planning service or limited planning features inside Production, then graduate to dedicated MRP when data maturity and retrofit contracts stabilize.

## 3. Top 3 Risks
1. Retrofit underestimation across Inventory and Maintenance causes schedule slip before new modules stabilize.
2. Cross-module event contract churn creates rework and brittle integrations (especially around Production <-> Quality <-> Inventory).
3. Aerospace bias leaks into platform abstractions, reducing reuse and increasing long-term maintenance cost.

## 4. Recommended Approach
Build in stages with explicit scope gates:

1. Complete retrofit foundations in Inventory/Maintenance/Workflow/Workforce integrations.
2. Ship BOM Lite.
3. Ship Production Lite with strict Inventory integration.
4. Ship Quality Core only (inspection + hold/disposition + basic NCR).
5. Defer full CAPA and deep special process controls to later releases.
6. Defer full MRP until execution data quality and workcenter/capacity contracts are proven in production.

Reject the all-at-once 4-module build as currently scoped.
