# Manufacturing Modules Scope Review — PurpleCliff

**Date:** 2026-03-04
**Bead:** bd-2wahd.1 (child of bd-2wahd)

## Executive Summary

The 4-module split is directionally correct but the scope is dangerously large. BOM and Production are genuinely generic and belong in the platform. Quality is too fat — it's really 2-3 modules wearing a trenchcoat. MRP should be deferred entirely until the first three are battle-tested. I'd estimate 6-8 months for BOM + Production + a slimmed Quality, assuming 5 agents — not 3 months. Rushing this doubles the platform surface area and creates a long stabilization tail we can't afford with a paying customer waiting.

---

## A. Module Boundaries

**BOM: Correct as scoped.** Multi-level BOM, revisions, effectivity dates, ECOs — this is a clean, well-bounded domain. ~8-10K LOC estimated (comparable to maintenance at 7.5K).

**Production: Correct as scoped.** Work orders, routings, shop floor tracking, labor collection. Clear lifecycle, clear boundaries. ~12-15K LOC estimated (comparable to shipping-receiving at 7.7K plus more state machine complexity).

**Quality: Too big. Split it.**
- **Inspection** (plans, FAI, receiving inspection) — one module. This is the "check things" lifecycle.
- **Nonconformance** (NCR + CAPA) — separate module. This is the "something went wrong, track the fix" lifecycle. Distinct state machines, distinct actors, distinct reporting needs.
- Special process controls could live in Workforce-Competence as an extension (it already tracks certifications and qualifications). Or it's a thin layer on top of Inspection. Either way, it doesn't need its own module.

Bundling all of Quality into one module would produce a 15-20K LOC crate with 4 independent state machines. That violates the "one bead = one concern" principle at the module level. It would be the largest module in the platform by far.

**MRP: Defer entirely.** MRP is a computation engine, not a CRUD domain. It consumes BOM + Inventory + Production as inputs and produces purchase/production suggestions as outputs. It has no business existing until BOM and Production are stable and integrated. Building it now means building against moving targets.

## B. Build Sequencing

**Minimum viable manufacturing stack for Fireproof:**

1. **BOM** (no dependencies beyond Inventory items + Numbering) — build first
2. **Production** (depends on BOM + Inventory issue/receipt + Workflow) — build second
3. **Inspection** (the slimmed Quality) — build third, integrates with shipping-receiving's existing `inspection_routing`

**What can ship incrementally:**
- BOM alone is useful day one — engineers can define part structures, revisions, ECOs
- Production without MRP means manual work order creation (which is how most shops start anyway)
- Inspection without NCR means you can inspect but nonconformances are tracked outside the system initially

**What to defer:**
- NCR/CAPA module — Phase N+1, after inspection is proven
- MRP — Phase N+2, after production is generating real data
- Special process controls — extend Workforce-Competence later

## C. Platform vs App-Specific

WhiteValley's line is mostly right, but I'm skeptical about two items:

**Inspection plans with acceptance criteria and sampling rules** — This IS generic manufacturing. Every manufacturer inspects incoming material and in-process work. The acceptance criteria structure (characteristic + tolerance + sampling method) is standard across industries. AQL sampling tables are ISO 2859 — not aerospace-specific.

**Special process controls** — This is WHERE I'd draw the line more carefully. The concept of "this process requires certified operators and controlled parameters" is generic. But the specific process types (welding per AWS D1.1, heat treat per AMS 2750, plating per ASTM B633) are deeply industry-specific. The platform should provide "process control records linked to competence requirements" — the specific process catalogs belong in Fireproof.

**The real boundary test:** Can a food manufacturer, an auto parts shop, and an aerospace company all use the same module with different configuration? If yes, it's platform. If you need aerospace domain knowledge to understand the data model, it's app-specific.

## D. Existing Module Retrofit

This is where the brief underestimates the work. The retrofits are not trivial.

### Inventory (the biggest hit)
- **Make/buy flag on items:** Currently no `procurement_type` or equivalent on the `Item` struct. Need to add a field, migration, and update all item creation paths. Low risk but touches a mature module (22K LOC).
- **Production receipt path:** `receipt_service.rs` currently handles purchase receipts. Production receipts (manufactured goods into stock) need a distinct `reference_type` and potentially different costing logic (standard cost vs actual accumulated cost). This is not a one-line change.
- **BOM hierarchy queries:** Inventory has no concept of parent/child items. Where-used queries ("what assemblies use this part?") belong in BOM, not Inventory. But Inventory needs to know "this item is a BOM header" for validation (can't issue a BOM header directly without exploding it).

### Maintenance
- **Workcenter master table:** `workcenter_id` exists as an optional UUID on `DowntimeEvent` but there's no `workcenters` table, no CRUD, no validation. Production needs workcenters as a first-class entity (capacity, scheduling, routing operations). This is a new sub-domain within maintenance — maybe 1-2K LOC.
- **Parts integration with Inventory:** Maintenance work order parts are currently ad-hoc (text description fields based on the `parts.rs` in work_orders). Production will need actual inventory issue/receipt integration for material consumption. This retrofit could cascade.

### Shipping-Receiving
- **Inspection routing already exists** (`direct_to_stock` / `send_to_inspection`). This is a natural hook for the Inspection module. But currently it just sets a routing flag — there's no actual inspection record created. The integration means: when routing = `send_to_inspection`, create an inspection record in the new Inspection module via NATS event.

### Workflow
- **No retrofit needed.** `entity_type` is already free-form string. ECOs, NCRs, CAPAs can all use Workflow as-is. This is the one module that was designed for exactly this scenario.

### Workforce-Competence
- **Minor retrofit.** Inspector qualifications and special process certifications fit the existing competence artifact model. May need a "check authorization at point in time" query that doesn't exist yet (or may — I'd need to verify).

**Total retrofit estimate:** 3-5 beads across Inventory, Maintenance, and Shipping-Receiving before the new modules can even integrate properly.

## E. Scope and Risk

**Current platform:** 22 modules, ~307K LOC (modules) + ~65K LOC (platform infra) = ~372K LOC across 1,243 source files.

**Proposed addition:** 4 new modules (I'd say 3 + defer MRP). Even with 3 modules, that's roughly 25-35K new LOC, plus 5-10K of retrofits to existing modules. That's a 10-12% increase in codebase size.

**Effort estimate with 5 agents:**
- BOM: 3-4 weeks (clean domain, few integrations)
- Production: 5-7 weeks (complex state machine, many integrations)
- Inspection: 3-4 weeks (moderate complexity, shipping-receiving integration)
- Retrofits: 2-3 weeks (scattered across modules, each small but coordination-heavy)
- Integration testing + stabilization: 3-4 weeks
- **Total: ~4-5 months realistic, 6-8 months with the inevitable surprises**

**What I'd defer beyond MRP:**
- Labor collection in Production (track it manually initially, add the module later)
- CAPA (start with NCR only if we must, but I'd defer the whole NCR/CAPA module)
- Production scheduling (manual sequencing first)

## F. Dependencies and Integration

```
                    Numbering
                       |
                       v
  Inventory -------> BOM <-------- Workflow (ECO approvals)
     |                |
     |                v
     +----------> Production <---- Workflow (WO approvals)
     |                |
     |                v
     |           Shop Floor
     |                |
     v                v
  Shipping -------> Inspection <-- Workforce-Competence (inspector quals)
  Receiving           |
                      v
                 NCR/CAPA <------- Workflow (disposition approvals)
                 (DEFERRED)
                      |
                      v
                    MRP
                 (DEFERRED)
```

**No circular dependencies.** The graph is a clean DAG.

**Must-have day one integrations:**
- BOM → Inventory (item lookups for BOM components)
- Production → BOM (explode BOM for work order)
- Production → Inventory (issue materials, receipt finished goods)
- Production → Workflow (work order approval lifecycle)

**Wire later:**
- Inspection → Shipping-Receiving (inspection on receipt)
- Inspection → Workforce-Competence (inspector authorization check)
- Production → Maintenance (workcenter availability/downtime)

## G. Alternative Approaches

**Could BOM be an extension of Inventory's item hierarchy?**

No. I considered this carefully. Inventory's `items.rs` is a flat master table — SKU, GL accounts, tracking mode. A BOM is a recursive parent-child graph with quantities, effectivity dates, and revision control. Cramming BOM into Inventory would:
1. Violate the 500 LOC file limit immediately (items.rs + BOM logic)
2. Make Inventory's already-large codebase (22K LOC) even bigger
3. Couple BOM revision lifecycle to inventory item lifecycle (they're independent — you can revise a BOM without changing the item)

**Could Production reuse Maintenance work orders?**

Tempting — maintenance already has an 8-state work order machine. But the domains are different: maintenance WOs repair/maintain assets; production WOs manufacture products. The state machines look similar but the data is different (BOM explosion, routing operations, material issues, labor collection vs. parts/labor on assets). Sharing the state machine would create a God object. Better to follow the existing pattern: each module owns its own WO lifecycle, communicates via events.

**Could Inspection live inside Shipping-Receiving?**

Partially — receiving inspection already has a hook there. But in-process inspection (during production) and final inspection (before shipment) don't belong in shipping-receiving. A standalone Inspection module that integrates with both shipping-receiving AND production is the cleaner design.

---

## Top 3 Risks

1. **Retrofit cascade:** Touching Inventory (the platform's most mature and complex module at 22K LOC) to add manufacturing concepts risks destabilizing existing functionality. Every inventory change needs full regression testing. This is the #1 risk because Inventory is load-bearing for the paying customer.

2. **Scope creep through "generic" claims:** WhiteValley has incentive to push aerospace-specific features into the platform ("it's generic!"). Each accepted feature increases platform maintenance burden forever. We need a rigorous "3-industry test" for every feature: would a food manufacturer need this? If not, it's app-specific.

3. **Integration testing complexity:** Manufacturing modules are deeply interconnected. A BOM change affects Production, which affects Inventory, which affects GL. End-to-end test scenarios will be long and fragile. Our no-mocks testing policy (which is correct) means these tests need real BOM → Production → Inventory → GL flows, which will be slow and complex to set up.

---

## Recommended Approach

### Phase A: Foundation (BOM + Retrofits)
- Build BOM module (clean, few dependencies)
- Retrofit Inventory: add `procurement_type` (make/buy/both), production receipt reference type
- Retrofit Maintenance: add workcenter master table
- **~6-8 weeks with 3 agents**

### Phase B: Core Manufacturing (Production)
- Build Production module (depends on BOM being stable)
- Wire Production → Inventory (material issue/receipt)
- Wire Production → Workflow (approvals)
- **~6-8 weeks with 3 agents**

### Phase C: Quality — Inspection Only
- Build Inspection module (receiving + in-process + final)
- Wire to Shipping-Receiving inspection routing
- Wire to Workforce-Competence for inspector quals
- **~4-5 weeks with 2 agents**

### Defer to Future Phases
- NCR/CAPA module
- MRP/Planning module
- Labor collection detail
- Production scheduling/capacity
- Special process controls (extend Workforce-Competence)

**Total for Phases A-C: ~4-5 months.** This gets Fireproof a working BOM → Production → Inspection flow without overcommitting the platform.
