# Manufacturing Modules Scope Review — CopperRiver

**Date:** 2026-03-04
**Reviewer:** CopperRiver
**Bead:** bd-2wahd

---

## Executive Summary

The 4-module proposal is directionally correct but too ambitious for a first-customer push. BOM and a stripped-down Quality (NCR only) are the minimum viable manufacturing stack. Production and MRP should be deferred — they are the most complex modules and Fireproof can start with manual scheduling. The retrofit cost to Inventory and Maintenance is real but manageable if scoped tightly.

---

## A. Module Boundaries

The 4-module split is mostly right, with one exception: **Quality is too big.**

Quality as described contains four distinct lifecycle domains:
1. Inspection plans (definition + execution)
2. NCR (nonconformance lifecycle)
3. CAPA (corrective/preventive action lifecycle)
4. Special process controls (certifications + parameter tracking)

These should be split into at least two modules:
- **Quality-Core**: Inspection plans + NCR. These are tightly coupled — an inspection failure creates an NCR. This is day-one manufacturing.
- **CAPA**: Separate lifecycle. Root cause analysis → corrective action → verification. This is important but not blocking for initial production. NCR can reference a CAPA by ID without needing the CAPA module to exist yet.
- **Special process controls**: This is borderline app-specific (see section C). Defer or leave in Fireproof.

BOM, Production, and MRP boundaries look clean as proposed.

## B. Build Sequencing

**Minimum viable manufacturing stack: BOM + Quality-Core (NCR).**

Rationale:
1. **BOM** is the foundation — you cannot do production without knowing what goes into a part. It's also relatively simple (hierarchical data + revision control, similar to what we already did with item revisions).
2. **NCR** is the quality gate — aerospace customers cannot ship without documenting and dispositioning nonconformances. This is non-negotiable for Fireproof.
3. **Production** can start as manual work orders — Maintenance already has an 8-state work order state machine that could be templated. Formal routing/operations can come later.
4. **MRP** is the most complex and least urgent. Early customers can use spreadsheets or manual planning. MRP done wrong is worse than no MRP.

Suggested sequence:
1. BOM (2-3 weeks with one agent)
2. Quality-Core: NCR + inspection plans (3-4 weeks)
3. Production (4-6 weeks — routing, shop floor, labor)
4. CAPA (2 weeks, simple lifecycle driven by Workflow)
5. MRP (6-8 weeks — this is an optimization engine, not a CRUD module)

## C. Platform vs App-Specific

WhiteValley draws the line wrong in one place: **special process controls are aerospace-specific wearing a generic hat.**

- "Track certifications and parameters for welding, heat treat, plating" — this is AS9100/NADCAP territory. A food manufacturer doesn't track weld certs; a consumer goods company doesn't track plating parameters.
- Generic manufacturing needs *process capability* (can this machine produce to spec?). Aerospace needs *special process qualification* (is this specific welding operator NADCAP-certified for this alloy?). These are fundamentally different.

**Recommendation:** Special process controls stay in Fireproof. Workforce-Competence already handles operator certifications and qualification checks — that's the platform layer. The aerospace-specific rules about which certifications apply to which processes are app logic.

Inspection plans with acceptance criteria and sampling rules are more defensibly generic. Any manufacturer that does incoming inspection needs them. But keep them simple: the platform should define "check dimension X against tolerance Y" — not AQL sampling tables or skip-lot programs, which are quality engineering features that belong in the app layer.

## D. Existing Module Retrofit

The brief understates the retrofit work. Here's what I found from reading the actual code:

**Inventory (substantial):**
- No `make_buy` flag on `Item`. This is a schema migration + API change to items.rs. Not hard, but touches a mature module (128 .rs files).
- No production receipt path — `receipt_service.rs` handles purchase receipts. Production receipts need a different flow (receive finished goods, backflush component consumption). This is a new service, not a tweak.
- `reservation_service.rs` has `reference_type: Option<String>` — this already supports arbitrary reference types like "production_order". No change needed here.
- Item revisions with effectivity dates exist and are well-built. BOM revisions can follow the same pattern.
- Genealogy (lot split/merge) exists but is limited to same-item lots. Production transforms multiple items into one — that's a cross-item genealogy extension.

**Maintenance (moderate):**
- `workcenter_id` exists on DowntimeEvent as `Option<Uuid>` but there's no workcenter master table. Need to add: workcenter CRUD, capacity tracking, calendar.
- Work order parts are ad-hoc text descriptions, not linked to inventory items. To track production parts consumption, this needs retrofit to reference inventory item IDs. But: this may be better handled in the Production module's own work orders rather than retrofitting Maintenance WOs.

**Workflow (minimal):**
- Already entity-agnostic with free-form `entity_type`. ECO, NCR, CAPA can all use existing definitions, instances, and routing. Holds primitive exists for quality/engineering holds.
- No changes needed — this module was designed for exactly this kind of extension.

**Shipping-Receiving (minimal):**
- `inspection_routing` already has `DirectToStock` / `SendToInspection` routing decisions. This is the natural hook for receiving inspection triggered by Quality's inspection plans.
- Integration is event-based: ship `sr.receipt_routed_to_inspection.v1`, Quality listens.

**Workforce-Competence (none):**
- Already has competence artifacts, operator assignments with expiry, acceptance authorities with capability scope. Quality can query this to validate inspector qualifications. No changes needed.

**Numbering (none):**
- Entity-agnostic atomic allocation already exists. New entity types (ECO, NCR, CAPA, production order) just need new numbering patterns configured. No code changes.

## E. Scope and Risk

**This is a 6-month effort minimum with 5 agents, not 3 months.** Here's my math:

- BOM: ~80 .rs files (comparable to maintenance at 52 files, but more complex hierarchy)
- Production: ~120 .rs files (comparable to shipping-receiving at 66 files, but with routing/operations/labor which are each substantial subdomains)
- Quality-Core (NCR + inspection): ~60 .rs files
- CAPA: ~30 .rs files
- MRP: ~100 .rs files (planning engine + demand netting + scheduling)
- Inventory retrofit: ~20 .rs files modified/added
- Maintenance retrofit: ~10 .rs files

Total: ~420 new .rs files + ~30 modified. That's a 34% increase in codebase size (1,243 current → ~1,663).

**What I'd defer:**
1. MRP entirely — it's the biggest, most complex, and least tested module type. Manual planning for V1.
2. CAPA — NCRs can reference a future CAPA ID without the module existing.
3. Labor collection in Production — track what was made, not who did it, for V1.

**What I'd reject:**
1. Special process controls (aerospace-specific, as argued above)

## F. Dependencies and Integration

```
Numbering ──────────────┐
                        │
Inventory ──────────┐   │
                    ├── BOM ──── Production ──── MRP
Workforce-Comp ─┐   │          │     │
                │   │          │     │
Maintenance ────┼───┘     Workflow   │
                │          │         │
                └── Quality-Core ────┘
                    │
Shipping-Receiving ─┘
```

**No circular dependencies.** The graph is a clean DAG.

**Must-have day one integrations:**
1. BOM → Inventory (component items lookup)
2. NCR → Workflow (NCR approval/disposition lifecycle)
3. Quality-Core → Workforce-Competence (inspector qualification check)
4. Quality-Core → Shipping-Receiving (receiving inspection trigger)

**Wire later:**
1. Production → Inventory (material issue/receipt — can be manual first)
2. Production → BOM (auto-explode BOM for work order — can be manual first)
3. MRP → everything (this is the last module built)

## G. Alternative Approaches

**BOM as Inventory extension — no.** I considered this because Inventory already has item revisions and hierarchy concepts (genealogy). But:
- Genealogy tracks material transformation (lot A became lots B+C). BOM tracks *design* relationships (to make part X, you need 3 of Y and 2 of Z). Different semantics.
- BOM needs its own revision lifecycle, effectivity dating, and ECO workflow. Bolting this onto Inventory would bloat an already-large module (128 files) past maintainability.
- The 500 LOC file limit would be under constant pressure.

**Production as Maintenance extension — tempting but wrong.** Maintenance WOs have an 8-state machine, parts, labor — similar structure. But:
- Maintenance WOs are *reactive* (fix broken things). Production WOs are *planned* (make N units of part X per BOM).
- Production needs routing (sequence of operations), which has no analog in Maintenance.
- Maintenance parts are ad-hoc text; production parts must be inventory-integrated with BOM explosion.
- Forcing these together would create a confused module trying to serve two masters.

**NCR as Workflow instance — partially.** NCR's approval lifecycle should absolutely be driven by Workflow instances. But NCR needs its own domain model (defect codes, disposition options, material review board records, affected lots/serials). Workflow provides the engine; Quality-Core provides the domain.

**Best alternative approach:** Build BOM as a standalone module. Build Quality-Core as a standalone module that delegates to Workflow for lifecycle management. Defer Production and MRP. This gives Fireproof the ability to define what goes into their products and track quality issues — the two things aerospace auditors will look for first.

---

## Top 3 Risks

1. **Inventory retrofit underestimated.** Adding production receipt to a mature module with 128 files and extensive test coverage is high-risk for regressions. The make/buy flag is easy; the backflush consumption path is not. Every test that touches receipts could break.

2. **Quality scope creep.** "Inspection plans with acceptance criteria and sampling rules" sounds simple until you realize every industry has its own inspection standards. The platform version needs to be deliberately minimal — store the plan definition, record pass/fail results, trigger NCR on failure. If it tries to encode sampling logic (AQL tables, skip-lot, tightened/normal/reduced), it becomes a quality engineering tool that no two industries agree on.

3. **MRP complexity trap.** If MRP starts before BOM and Production are stable, it will build on sand. MRP is also the only module that does non-trivial computation (BOM explosion, netting, scheduling) rather than CRUD + events. It needs different testing strategies (deterministic planning scenarios, not just integration tests). The team has no prior experience with planning engines.

---

## Recommended Approach

**Phase A (immediate, 5-6 weeks):**
- Build BOM module (multi-level structure, revisions, effectivity, ECO via Workflow)
- Build Quality-Core module (inspection plans, NCR lifecycle via Workflow)
- Add `make_buy` flag to Inventory items (minor schema change)

**Phase B (after Phase A stabilizes, 4-6 weeks):**
- Build Production module (work orders, routing, operations, shop floor tracking)
- Retrofit Inventory for production receipts and backflush consumption
- Add workcenter master to Maintenance

**Phase C (defer until Phase B is proven, 6-8 weeks):**
- Build MRP/Planning module
- Build CAPA module
- Advanced quality features (sampling, statistical process control)

**Reject entirely:**
- Special process controls (app-specific, not platform)

Total: ~16-20 weeks with 5 agents working in parallel, assuming no scope creep. Plan for 6 months.
