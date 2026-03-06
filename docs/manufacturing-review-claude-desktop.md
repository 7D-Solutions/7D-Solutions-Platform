# Manufacturing Modules Scope Review — Claude Desktop

**Date:** 2026-03-04
**Reviewer:** Claude Desktop (Cowork)
**Bead:** bd-2wahd
**Brief:** `docs/MANUFACTURING-MODULES-REVIEW-BRIEF.md`

---

## Executive Summary

The 4-module proposal (BOM, Production, Quality, MRP) is directionally correct and the platform's existing modules — particularly Inventory, Workflow, and Workforce-Competence — provide stronger foundations than WhiteValley may realize. However, Quality is oversized and should be split, MRP should be deferred entirely, and the retrofit work to existing modules (especially Inventory and Maintenance) is being materially underestimated. I recommend a phased approach: ship BOM + Production first with surgical Inventory retrofits, then Quality (split into two modules), then MRP last — if ever.

---

## A. Module Boundaries

### Is the 4-module split correct?

The split is mostly right, with one exception: **Quality is too big and should become two modules.**

**BOM — Correct as proposed.** Bill of Materials is a distinct domain with its own lifecycle (revisions, effectivity dates, ECO management). It has a clear ownership boundary: "what goes into a finished good and in what quantity." It is not an extension of Inventory (see Section G). BOM owns the structure; Inventory owns the physical stock.

**Production — Correct as proposed.** Production work orders, routings/operations, and shop floor tracking form a cohesive domain. Labor collection overlaps slightly with the existing Timekeeping module, but Production's labor is operation-level (who worked on operation 30 of WO-2026-003 for 2.5 hours) while Timekeeping's labor is payroll/billing-level. These should integrate via events, not merge.

**Quality — Should split into two modules:**

1. **Quality-Inspection** — Inspection plans, sampling rules, first article inspection (FAI), receiving inspection integration. This is the "measure and record" domain. It has a tight integration surface with Shipping-Receiving (inspection_routings already exist) and Inventory (status bucket transitions).

2. **Quality-Nonconformance** — NCR lifecycle, CAPA lifecycle, disposition decisions, root cause analysis. This is the "find problems and fix them" domain. It is workflow-heavy (both NCR and CAPA are multi-step approval processes that map directly onto the existing Workflow engine) and has different consumers, different cadence, and different regulatory weight.

The reason to split: inspection plans are high-frequency, low-latency operations (every receipt, every production lot). NCR/CAPA are low-frequency, high-touch operations (monthly, with weeks-long lifecycles). Coupling them means inspection latency is affected by NCR schema migrations, and NCR approval workflows are constrained by inspection plan release cadence. They also have different proving timelines — inspection is needed at BOM+Production launch; NCR/CAPA can follow later.

**Special process controls** (welding, heat treat, plating parameter tracking) belong in Quality-Inspection as a sub-domain. They are really "specialized inspection plans with process parameter recording" — the same entity model with richer payload fields.

**MRP — Correct as proposed, but should be deferred.** MRP is the most complex module in the proposal and depends on everything else being stable. It is also the module most likely to vary by vertical (job-shop MRP vs. process MRP vs. discrete MRP). Building it prematurely risks locking in assumptions. See Section E.

### Recommended module count: 5 (not 4)

| Module | Scope |
|--------|-------|
| BOM | Structure, revisions, ECO lifecycle, where-used |
| Production | Work orders, routings, shop floor tracking, operation labor |
| Quality-Inspection | Inspection plans, FAI, sampling, special process controls |
| Quality-Nonconformance | NCR, CAPA, disposition, root cause (Workflow-driven) |
| MRP | Material planning, scheduling, demand netting (deferred) |

---

## B. Build Sequencing

### Minimum Viable Manufacturing Stack

The minimum stack to unblock Fireproof is **BOM + Production + surgical Inventory retrofit**. Quality-Inspection follows immediately; Quality-Nonconformance and MRP are later phases.

### Recommended sequence

```
Phase 1 (Months 1-3): BOM + Inventory Retrofit
├── BOM module scaffold + core domain (structure, revisions, where-used)
├── Inventory retrofit: add "produced" entry_type, make/buy classification
├── ECO lifecycle via Workflow (definition + steps, no new module code)
├── Numbering integration (BOM numbers, ECO numbers)
└── Proving: BOM v1.0.0

Phase 2 (Months 2-5, overlapping): Production
├── Production module scaffold (work orders, routings, operations)
├── Inventory integration: reserve components on WO release, issue on operation start, receipt on WO close
├── BOM integration: explode BOM to create WO material list
├── Maintenance retrofit: workcenter master table (shared reference)
├── Shop floor tracking (operation start/complete/scrap)
├── Labor collection (operation-level, events to Timekeeping)
└── Proving: Production v1.0.0

Phase 3 (Months 4-7): Quality-Inspection
├── Inspection plans, sampling rules, test records
├── Shipping-Receiving integration: consume inspection routing events
├── Inventory integration: quarantine → available on inspection pass
├── Workforce-Competence integration: inspector authorization check
├── FAI (first article inspection)
├── Special process controls
└── Proving: Quality-Inspection v1.0.0

Phase 4 (Months 6-9): Quality-Nonconformance
├── NCR lifecycle (Workflow-driven, minimal new domain code)
├── CAPA lifecycle (Workflow-driven)
├── Disposition decisions with Inventory status transfers
├── Root cause analysis records
└── Proving: Quality-Nonconformance v1.0.0

Phase 5 (Months 9-12+): MRP — Only if validated by real usage
├── Demand netting against BOM + Inventory
├── Production scheduling against capacity
├── Reorder point integration with existing Inventory reorder policies
└── Proving: MRP v1.0.0
```

### What can ship incrementally to unblock Fireproof?

BOM alone provides immediate value. Fireproof can define multi-level structures, manage revisions with effectivity dates, and run where-used queries. This unblocks their engineering team even before Production exists. Production unblocks their shop floor. Quality-Inspection unblocks their AS9100 receiving inspection (the Fireproof-specific AS9100 rule sets layer on top of the generic inspection plan engine).

---

## C. Platform vs App-Specific

### WhiteValley's line is mostly in the right place, with two concerns.

**Correctly app-specific:** AS9100 compliance rule sets, ITAR/export control, flowdown clauses, NADCAP tracking, AS9102 report formats. These are aerospace-specific regulatory overlays. They should live in Fireproof's vertical repo as configuration and templates that reference platform module APIs.

**First concern — Inspection plans are generic, but barely.** "Inspection plan with acceptance criteria and sampling rules" sounds generic and is present in manufacturing broadly (ISO 9001, automotive IATF 16949, food HACCP). However, the acceptance criteria model can vary enormously: attribute inspection (pass/fail), variable inspection (measurement within tolerance), visual inspection (inspector judgment), destructive testing (break the part). If the platform module tries to be generic across all these, it becomes a framework rather than a module — and frameworks are scope magnets.

**Recommendation:** Build inspection plans with a deliberately narrow initial scope: attribute-based acceptance criteria (pass/fail per characteristic, with sampling based on lot size) and measurement-based acceptance criteria (value, tolerance, unit). Defer destructive testing and visual inspection to app-specific extensions. This covers 80% of manufacturing inspection needs without becoming a framework.

**Second concern — Special process controls are aerospace wearing a generic hat.** "Track certifications and parameters for welding, heat treat, plating" is described generically but is driven almost entirely by NADCAP/AS9100 requirements. General manufacturing doesn't track plating bath chemistry in their ERP. However, the underlying model (process type → required certifications → parameter recording → acceptance criteria) is reusable, so building it in the platform is defensible if the implementation stays at the "parameter recording" level and doesn't embed aerospace-specific validation rules.

**Recommendation:** Build special process controls as "process parameter recording linked to inspection plans" in the platform. Aerospace-specific parameter sets, validation rules, and NADCAP report formats stay in Fireproof.

### The real boundary test

Ask: "Would TrashTech (waste management vertical) ever use this?" If no, it's app-specific. Inspection plans — yes (incoming material inspection). NCR/CAPA — yes (vehicle damage, route nonconformance). Special process controls — unlikely. MRP — maybe (route planning is a different kind of MRP). BOM — unlikely for TrashTech but yes for any manufacturing vertical.

---

## D. Existing Module Retrofit

### WhiteValley is underestimating the retrofit work.

**Inventory (HIGH effort)**

| Change | Type | Effort | Risk |
|--------|------|--------|------|
| Add `produced` entry_type to ledger enum | Migration + domain | Small | LOW — additive enum value |
| Add make/buy classification | Already possible via classifications system | Minimal | LOW |
| Production receipt path (receive finished goods from WO) | New service endpoint + guard logic | Medium | MEDIUM — must integrate with FIFO layer creation at production cost |
| Component issue path (issue raw materials to WO) | Extend existing issue service with WO reference | Medium | LOW — reservation fulfillment already works |
| Scrap/yield tracking | New adjustment reason codes + variance accounting | Medium | MEDIUM — GL posting for variances needs design |
| Backflush support (auto-issue components on WO completion) | New service: explode BOM, batch-issue components | Large | HIGH — complex transaction spanning BOM read + multi-line issue + FIFO consumption |

The backflush case is the hardest. When a production order completes, the system must atomically issue all component materials per the BOM, create FIFO layer consumptions for each, and receipt the finished good — all in one transaction. This touches the ledger, layers, on-hand projections, and reservations simultaneously. Getting the cost flow right (component costs roll up into finished good unit cost) is the make-or-break integration.

**Maintenance (MEDIUM effort)**

| Change | Type | Effort | Risk |
|--------|------|--------|------|
| Workcenter master table | New entity + CRUD + migration | Medium | LOW — clean addition |
| Link work orders to workcenters | Add FK, update state machine guards | Small | LOW |
| Parts integration with Inventory (issue from stock) | New event consumer or API call on WO parts add | Medium | MEDIUM — Maintenance parts are currently text-only |
| Downtime → capacity impact calculation | New domain logic for production scheduling | Large | HIGH — requires workcenter capacity model that doesn't exist |

The workcenter master is the key retrofit. Production needs workcenters for routing operations ("operation 20 runs on CNC-Mill-3"). Maintenance needs workcenters for downtime tracking and preventive maintenance scheduling. The question is who owns the workcenter master. I recommend: **Production owns workcenter master; Maintenance consumes it via events or HTTP API.** Maintenance's `workcenter_id` on downtime events already exists as a free-form UUID — it just needs a source of truth.

**Shipping-Receiving (LOW effort)**

Inspection routing hooks (`direct_to_stock` / `send_to_inspection`) already exist. The Quality-Inspection module consumes these events and creates inspection tasks. No S-R retrofit needed beyond wiring the event consumer.

**Workflow (NO effort)**

Already entity-agnostic. ECO, NCR, and CAPA lifecycles are workflow definitions (configuration), not code changes. Define the steps, routing rules, and context schema — then create instances. The hold primitive supports quality/engineering/material holds out of the box.

**Workforce-Competence (NO effort)**

Authorization check API (`check_authorization(operator_id, capability_scope, timestamp)`) already exists. Production and Quality-Inspection call it at operation start and inspector sign-off. No retrofit needed.

**Numbering (NO effort)**

Already supports arbitrary entity types. `{ entity: "work_order" }`, `{ entity: "eco" }`, `{ entity: "ncr" }` — just configuration. Gap-free mode is ready for regulated serial number sequences.

### Total retrofit estimate

Inventory retrofit is 60% of the work. Plan 4-6 weeks of dedicated effort on Inventory alone, with careful attention to the backflush/cost-rollup path. Maintenance workcenter master is another 1-2 weeks. Everything else is integration wiring.

---

## E. Scope and Risk

### This is a 9-12 month effort, not 3.

With 5 implementation agents and the phased approach above:

| Phase | Duration | Agents | Constraint |
|-------|----------|--------|------------|
| BOM + Inventory retrofit | 8-10 weeks | 2 agents | Inventory backflush is critical path |
| Production | 10-14 weeks | 2-3 agents | Routing/operations model is complex; shop floor tracking has many edge cases |
| Quality-Inspection | 8-10 weeks | 2 agents | Sampling rules and inspection plan versioning take time |
| Quality-Nonconformance | 6-8 weeks | 1-2 agents | Mostly Workflow definitions; NCR/CAPA domain is well-understood |
| MRP | 12-16 weeks | 2-3 agents | Explosion algorithm, netting, scheduling — each is substantial |

With overlap between phases: **9-12 months total.** MRP alone could be 3-4 months.

### What to defer

1. **MRP — defer indefinitely.** Let Fireproof build demand planning as an app-specific tool using BOM and Inventory HTTP APIs. If a second vertical needs MRP, then extract the generic parts. Building generic MRP before having one working implementation is premature abstraction at the domain level.

2. **Production scheduling / capacity planning — defer.** Ship production work orders with manual scheduling first. Automated scheduling against workcenter capacity is an optimization, not a launch requirement.

3. **Backflush — defer to Phase 2.** Ship Production with explicit component issue (operator scans each component). Backflush (auto-issue on WO close) is a convenience feature that can follow.

### Without MRP, this is a 6-9 month effort. That's the honest timeline.

---

## F. Dependencies and Integration

### Dependency Graph

```
                    ┌──────────────┐
                    │   Numbering  │ (existing, no changes)
                    └──────┬───────┘
                           │ allocates IDs for
                           ▼
┌─────────────┐     ┌──────────────┐     ┌──────────────────────┐
│  Inventory  │◄────│     BOM      │────►│       Workflow        │
│ (retrofit)  │     │   (NEW)      │     │  (existing, config)   │
│             │     └──────┬───────┘     │  ECO definitions      │
│ items,      │            │             └──────────┬─────────────┘
│ lots,       │            │ explodes into           │ drives approvals for
│ status      │            ▼                         ▼
│ buckets,    │     ┌──────────────┐     ┌──────────────────────┐
│ reservations│◄────│  Production  │────►│    Maintenance        │
│             │     │   (NEW)      │     │  (retrofit: workcenter│
│             │     │              │     │   master)             │
│             │     └──────┬───────┘     └──────────────────────┘
│             │            │
│             │            │ triggers inspection
│             │            ▼
│             │     ┌──────────────────┐
│             │◄────│ Quality-         │────►┌─────────────────────┐
│             │     │ Inspection (NEW) │     │ Workforce-Competence │
│             │     └──────┬───────────┘     │ (existing, no changes│
│             │            │                 │ — authorization API)  │
│             │            │ feeds findings   └─────────────────────┘
│             │            ▼
│             │     ┌──────────────────┐
│             │◄────│ Quality-         │────►┌─────────────────────┐
│             │     │ Nonconformance   │     │ Workflow             │
│             │     │ (NEW)            │     │ (NCR/CAPA defs)     │
└─────────────┘     └──────────────────┘     └─────────────────────┘

        ┌───────────────────┐
        │ Shipping-Receiving │
        │ (existing)         │───► Quality-Inspection
        │ inspection_routings│     (consumes routing events)
        └───────────────────┘

        ┌───────────────────┐
        │      MRP          │ (DEFERRED)
        │ depends on ALL    │
        │ of the above      │
        └───────────────────┘
```

### Circular dependency analysis

**No circular dependencies.** The dependency flow is strictly:

- BOM → Inventory (reads items), Numbering (allocates IDs)
- Production → BOM (reads structure), Inventory (reserves/issues/receipts), Maintenance (reads workcenter availability), Workflow (ECO approvals)
- Quality-Inspection → Inventory (status transitions), Workforce-Competence (authorization checks), Shipping-Receiving (consumes routing events)
- Quality-Nonconformance → Workflow (drives NCR/CAPA lifecycles), Inventory (disposition → status transfers), Quality-Inspection (feeds findings)

All dependencies are unidirectional. Production never calls Quality; Quality never calls Production. BOM never calls Production. This is correct and should be preserved.

### Must-have day one vs wire later

**Must-have at BOM launch:**
- Inventory item read (BOM references items by item_id)
- Numbering allocation (BOM and ECO numbers)

**Must-have at Production launch:**
- BOM explosion (read BOM structure to build WO material list)
- Inventory reserve + issue + receipt (component consumption and finished good receipt)
- Workcenter master (Production owns it; Maintenance consumes)

**Must-have at Quality-Inspection launch:**
- Shipping-Receiving inspection routing events (already emitted)
- Inventory status transfer API (quarantine → available)
- Workforce-Competence authorization check API (already exists)

**Wire later:**
- Production → Timekeeping labor sync
- Production → GL cost posting (production variances)
- Quality-Nonconformance → Maintenance corrective work orders
- Quality-Inspection → Production (in-process inspection holds)
- All MRP integrations

---

## G. Alternative Approaches

### Could BOM be an extension of Inventory's item hierarchy?

**No. This is the wrong abstraction.** Inventory owns "what physical stock exists and where." BOM owns "what components make up a finished good and in what quantity." These are fundamentally different questions with different lifecycles:

- An item can exist in Inventory without being part of any BOM (a standalone purchased part).
- A BOM can reference items that have zero on-hand quantity (design-phase BOM before purchasing).
- BOM revisions follow engineering change processes with effectivity dates that are independent of inventory receipt dates.
- BOM has multi-level hierarchy (assembly → sub-assembly → component) that Inventory's flat item list cannot represent.

Inventory's item revision system tracks "the item's definition changed" (new GL account, new inspection policy). BOM revision tracks "the recipe changed" (component A replaced with component B). These are different axes of change.

Putting BOM into Inventory would violate the module standard's core principle: a module is a self-contained business capability. It would also create a God Module — Inventory is already the largest unproven module in the platform with 20+ migration files and 17+ event types.

### Could Production be an extension of Maintenance work orders?

**Tempting but wrong.** Maintenance work orders and production work orders share surface-level similarity (both have states, parts, labor, and assigned technicians). But the domains diverge quickly:

- Production work orders have routings (ordered sequence of operations at specific workcenters). Maintenance work orders do not.
- Production work orders consume BOM-specified materials. Maintenance work orders consume ad-hoc parts.
- Production work orders create finished goods (inventory receipt). Maintenance work orders restore asset functionality.
- Production has yield, scrap, rework concepts. Maintenance does not.

However, **the workcenter master should be shared.** Production creates and owns workcenter definitions. Maintenance references them for downtime tracking and preventive maintenance scheduling. This is a legitimate shared reference — not a shared module, but a shared entity exposed via API/events from Production to Maintenance.

### Could Quality be handled by extending Workflow + Inventory?

**Partially.** NCR and CAPA lifecycles can absolutely be driven by Workflow definitions with no new module code — define the steps, routing rules, and context schema. The NCR/CAPA domain data (root cause categories, disposition options, corrective action records) does need a home, which is why Quality-Nonconformance exists as a thin domain module on top of Workflow.

Inspection plans, however, cannot be handled by Workflow alone. They require domain-specific logic: sampling rule evaluation, acceptance criteria matching, test result recording, statistical process control. This is genuine domain code that belongs in Quality-Inspection.

---

## Top 3 Risks

### 1. Inventory backflush / cost rollup is the technical crux

When a production order completes, the system must atomically: read BOM for component list, issue each component from Inventory (consuming FIFO layers at their historical cost), and receipt the finished good (at rolled-up component cost + labor + overhead). This is a cross-module transaction that touches BOM (read), Inventory (multi-line issue + receipt + FIFO layer consumption + on-hand update), and Production (WO cost accumulation). Getting the cost flow right is non-trivial and any error produces incorrect financial statements. This integration must be designed before a single line of BOM or Production code is written.

### 2. Scope creep from "generic" to "framework"

Manufacturing is a domain where every vertical has opinions. Discrete manufacturing, process manufacturing, job-shop, repetitive, mixed-mode — each has different BOM structures (single-level vs. multi-level vs. recipe/formula), different production models (work order vs. process order vs. repetitive schedule), and different quality requirements. If the platform tries to accommodate all of them from day one, these modules will never ship. The brief describes discrete manufacturing with engineering change control — that's the scope. Process manufacturing (batch/formula) and repetitive manufacturing (rate-based) should be explicitly out of scope for v1.0.0.

### 3. Five new modules + retrofits strain the versioning system

Today there are 5 proven modules and 17 unproven. This proposal adds 4-5 more unproven modules and requires non-trivial changes to Inventory (which is itself unproven). If Inventory is proved first (as it should be — it's the most mature unproven module), then manufacturing retrofits to Inventory require version bumps, revision entries, and potentially breaking changes. The sequencing matters: prove Inventory before the manufacturing retrofit, or accept that the retrofit happens while Inventory is still unproven. I recommend the latter — prove Inventory after the manufacturing receipt/issue paths are integrated, not before.

---

## Recommended Approach

### Build order

1. **BOM** — Start immediately. Standalone value, minimal dependencies. 2 agents, 8-10 weeks to v1.0.0.
2. **Inventory retrofit** — Start in parallel with BOM. Add `produced` entry type, production receipt path, component issue path. 1 agent, 4-6 weeks.
3. **Production** — Start when BOM is at v0.3.0+ (core structure stable). 2-3 agents, 10-14 weeks to v1.0.0.
4. **Quality-Inspection** — Start when Production is at v0.3.0+. 2 agents, 8-10 weeks to v1.0.0.
5. **Quality-Nonconformance** — Start when Workflow definitions for NCR/CAPA are designed. 1-2 agents, 6-8 weeks to v1.0.0.

### Explicitly defer

- **MRP** — Do not build until BOM + Production + Quality have been in production for at least one quarter. Let Fireproof build a vertical-specific planning tool first. Extract generic MRP only when a second vertical needs it.
- **Production scheduling / capacity planning** — Manual scheduling for v1. Automated scheduling is an optimization.
- **Backflush** — Explicit component issue for v1. Backflush is a convenience feature for v2.
- **Process manufacturing (recipe/formula)** — Out of scope for v1. Discrete manufacturing only.

### Reject

Nothing outright. The proposal is sound. The adjustments are: split Quality, defer MRP, and be honest about the 9-month timeline.

### One non-negotiable prerequisite

Before any manufacturing module bead is created: **design the finished-good cost rollup flow end-to-end.** Document exactly how component costs flow through FIFO layers into finished good unit cost. Get sign-off from the GL module owner (even though GL is unproven, the cost accounting model must be correct). This is the architectural one-way door for manufacturing. Get it wrong and every financial report is wrong.

---

*Review complete. Direct questions to the orchestrator (BrightHill) via bead bd-2wahd.*
