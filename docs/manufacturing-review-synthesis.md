# Manufacturing Modules — Review Synthesis

**Date:** 2026-03-04
**Bead:** bd-2wahd
**Reviews collected from:** SageDesert, DarkOwl, PurpleCliff, CopperRiver, Grok (adversarial), ChatGPT, Claude Desktop
**All reviews complete.**

---

## Consensus Summary

Five independent reviews (4 agents + Grok adversarial) reached strong agreement on all key questions. This synthesis captures where they align and the few areas of disagreement.

**Notable divergence:** CopperRiver suggests BOM + Quality-Core (NCR) first, deferring Production to Phase B — arguing that NCR is non-negotiable for aerospace auditors. Other reviewers prioritize BOM → Production → Inspection. Both sequences are valid; the key decision is whether quality documentation or production execution is more urgent for Fireproof's first audit.

---

## 1. Module Boundaries — Unanimous Positions

### BOM: Approved as separate module (4/4)
- BOM is a versioned product structure graph with effectivity dates and change governance
- Does NOT belong in Inventory — that would couple revision lifecycle to item lifecycle
- Estimated ~8-10K LOC (PurpleCliff)

### Production: Approved as separate module (4/4)
- Work order lifecycle, routings, material issue/receipt, operation tracking
- Does NOT belong in Maintenance — different intent (manufacture vs. repair), different data model
- Estimated ~12-15K LOC (PurpleCliff)

### Quality: TOO BIG — must be split (4/4)
All reviewers independently concluded Quality is overloaded. Consensus split:
- **Inspection** (plans, FAI, receiving/in-process/final inspection) — build now
- **NCR/CAPA** (nonconformance reports, corrective/preventive action) — defer
- **Special process controls** — defer or extend Workforce-Competence later
- PurpleCliff: "2-3 modules wearing a trenchcoat"
- SageDesert: "at least three bounded contexts with very different state machines"

### MRP: DEFER entirely (4/4)
- MRP is a computation engine that consumes BOM + Inventory + Production
- Building it now means building against moving targets
- Start with manual work order creation (how most shops start anyway)

### CopperRiver's alternative: Quality-Core before Production
CopperRiver argues BOM + NCR should come before Production, because aerospace auditors look for: (1) product structure definitions and (2) quality nonconformance tracking first. Production can start as manual work orders. This is a valid alternative phasing.

### Special process controls: CopperRiver says reject outright
CopperRiver argues special process controls are aerospace-specific, not generic manufacturing. "A food manufacturer doesn't track weld certs." Workforce-Competence already handles operator certifications — that's the platform layer. The aerospace-specific rules belong in Fireproof. Other reviewers suggested deferring; CopperRiver says reject.

### One disagreement: Grok suggested combining BOM + Production
All four agent reviewers explicitly rejected this, with reasoning:
- BOM is configuration/structure, Production is execution
- They have independent revision lifecycles
- Different teams interact with each (engineering vs. shop floor)

**Decision: Keep BOM and Production as separate modules.**

---

## 2. Build Sequencing — Consensus Phasing

### Phase A: Foundation (~6-8 weeks)
1. **Retrofit existing modules:**
   - Inventory: add `procurement_type` (make/buy), production receipt reference type, WIP movement types
   - Maintenance: add workcenter master table (capacity, calendars)
   - Shipping-Receiving: wire inspection_routing to create real inspection records
2. **Build BOM module** (fewest dependencies: just Inventory items + Numbering)

### Phase B: Core Manufacturing (~6-8 weeks)
3. **Build Production module:**
   - Work order lifecycle (create/release/track/close)
   - BOM explosion for material requirements
   - Material issue/receipt via Inventory
   - Basic operation completion tracking
   - Workflow integration for approvals

### Phase C: Quality Gates (~4-5 weeks)
4. **Build Inspection module** (slimmed Quality):
   - Inspection plans (characteristics, tolerances, sampling)
   - Receiving inspection (integrated with Shipping-Receiving)
   - In-process and final inspection
   - Hold/release integration with Inventory status buckets
   - Workforce-Competence integration for inspector qualifications

### Defer to Future Phases
- NCR/CAPA module
- MRP/Planning module
- Labor collection detail (manual tracking initially)
- Production scheduling/capacity optimization
- Special process controls (extend Workforce-Competence later)
- Advanced sampling rule engines

---

## 3. Platform vs App-Specific Boundary

**Consensus boundary test** (PurpleCliff's formulation, endorsed by all):
> "Can a food manufacturer, an auto parts shop, and an aerospace company all use the same module with different configuration? If yes → platform. If you need aerospace domain knowledge to understand the data model → app-specific."

| Feature | Verdict | Reasoning |
|---------|---------|-----------|
| Inspection plans + acceptance criteria | Platform | ISO 2859 sampling is industry-standard, not aerospace-specific |
| Special process evidence capture | Platform | Generic: operator qual + machine cert + parameter record |
| Special process rule catalogs | App-specific | NADCAP/AWS D1.1/AMS 2750 are industry-specific |
| AS9100 compliance rules | App-specific | Aerospace standard |
| ITAR/export control | App-specific | Defense-specific |
| AS9102 FAI report format | App-specific | Aerospace format (concept is generic) |
| NADCAP accreditation | App-specific | Aerospace special processes |

---

## 4. Retrofit Scope — Underestimated

All reviewers flagged this. PurpleCliff estimates 3-5 retrofit beads before new modules can integrate.

### Inventory (biggest hit — 22K LOC mature module)
- Add `procurement_type` field (make/buy/both) to Item
- Production receipt path (distinct from purchase receipt — different costing)
- Production issue movement type (issue-to-WIP)
- Scrap/rework transaction types
- Stronger reservation semantics for work order references

### Maintenance
- Workcenter master table (currently just a UUID reference on DowntimeEvent)
- Capacity units, calendars, machine-to-workcenter association
- Downtime event semantics consumable by production

### Shipping-Receiving
- inspection_routing currently sets a flag but creates no inspection record
- Need: event-driven bridge to Inspection module

### Workflow — No retrofit needed
- entity_type is already free-form string — ECOs, NCRs can use it as-is

### Workforce-Competence — Minor
- May need "check authorization at point in time" query
- Otherwise fits existing competence artifact model

---

## 5. Timeline Estimates

| Scope | Estimate | Source |
|-------|----------|--------|
| Full 4 modules as proposed | 9-18 months | All reviewers |
| MVP (BOM + Production + Inspection + retrofits) | 3-5 months | DarkOwl, PurpleCliff |
| Realistic with surprises | 6-8 months | PurpleCliff |

---

## 6. Top Risks (ranked by frequency across reviews)

1. **Retrofit cascade** (4/4): Touching Inventory (most mature, load-bearing module) risks destabilizing existing functionality. Every change needs full regression. This is the #1 risk because Inventory is critical for the paying customer.

2. **Aerospace bias leaking into platform** (3/4): WhiteValley has incentive to push aerospace-specific features as "generic." Each accepted feature increases permanent maintenance burden. Rigorous 3-industry test required.

3. **Cross-module event contract churn** (3/4): Manufacturing modules are deeply interconnected. BOM change → Production → Inventory → GL. Event contracts will evolve during development, creating rework.

4. **Scope creep through incremental additions** (2/4): Even the "slimmed" scope roughly doubles the platform's domain surface area.

---

## 7. Dependency Graph (Clean DAG — No Circular Dependencies)

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
Shipping -------> Inspection <-- Workforce-Competence
Receiving                            (inspector quals)
                    |
                    v
               NCR/CAPA (DEFERRED)
                    |
                    v
                  MRP (DEFERRED)
```

**Must-have day-one integrations:**
- BOM → Inventory (item lookups for components)
- Production → BOM (explode for work order)
- Production → Inventory (issue materials, receipt finished goods)
- Production → Workflow (approval lifecycle)

**Wire later:**
- Inspection → Shipping-Receiving (inspection on receipt)
- Inspection → Workforce-Competence (inspector authorization)
- Production → Maintenance (workcenter availability)

---

## 8. Recommendation to ChatGPT for Bead Planning

Based on unanimous reviewer consensus, the planning session should produce beads for:

**Track 1: Retrofits (can parallelize)**
- Inventory: procurement_type + production movements
- Maintenance: workcenter master table
- (Shipping-Receiving inspection bridge can wait until Inspection module exists)

**Track 2: BOM module (after Inventory retrofit)**
- Multi-level BOM structure
- BOM revisions + effectivity dates
- ECO lifecycle (via Workflow)
- Where-used queries

**Track 3: Production module (after BOM)**
- Work order lifecycle
- Basic routing/operations
- Material issue/receipt (Inventory integration)
- Operation completion tracking

**Track 4: Inspection module (after Production)**
- Inspection plans
- Receiving/in-process/final inspection
- Hold/release (Inventory status integration)

**Explicitly NOT in scope:**
- MRP/Planning
- NCR/CAPA
- Labor collection detail
- Production scheduling/capacity
- Special process controls

---

## 9. Claude Desktop Additions (Review #7)

Claude Desktop's review added several insights beyond the other 6 reviewers:

### Non-Negotiable Prerequisite: Cost Rollup Design
Before any manufacturing bead is created, design the finished-good cost rollup flow end-to-end. When a production order completes: read BOM for components → issue each component from Inventory (consuming FIFO layers at historical cost) → receipt finished good at rolled-up component cost + labor + overhead. This is cross-module (BOM read + Inventory multi-line issue/receipt + Production WO cost accumulation) and any error produces incorrect financial statements. **This is the architectural one-way door.**

### Workcenter Ownership
Production owns the workcenter master table. Maintenance consumes it via events/HTTP API. Maintenance's existing `workcenter_id` on DowntimeEvent gets a source of truth.

### Backflush Deferred
Ship Production v1 with explicit component issue (operator scans each part). Backflush (auto-issue on WO close) is v2 — it's the most complex Inventory integration (atomic multi-line issue + FIFO consumption + cost rollup).

### Inventory Proving Sequencing
Prove Inventory AFTER manufacturing retrofits, not before. Otherwise the retrofit requires version bumps and potentially breaking changes to a proven module.

### Scope Fence: Discrete Manufacturing Only
Process manufacturing (recipe/formula BOM), repetitive manufacturing (rate-based), and mixed-mode are explicitly out of scope for v1. The proposal describes discrete manufacturing with engineering change control — that's the scope.

### Module Count: 5
Claude Desktop recommends 5 modules: BOM, Production, Quality-Inspection, Quality-Nonconformance, MRP (deferred). Special process controls live inside Quality-Inspection as "specialized inspection plans with process parameter recording."

### Timeline: 9-12 months (6-9 without MRP)
Most detailed estimate of all reviewers, with per-phase agent allocation and duration.

---

## 10. Final Consensus Summary

| Question | Consensus | Dissent |
|----------|-----------|---------|
| BOM separate? | Yes (7/7) | None |
| Production separate? | Yes (7/7) | Grok suggested combine with BOM (rejected by all agents) |
| Quality split? | Yes (7/7) | Minor: 2-way vs 3-way vs 4-way split |
| MRP defer? | Yes (7/7) | None |
| Retrofits underestimated? | Yes (7/7) | None |
| Phase order | BOM → Production → Inspection (5/7) | CopperRiver: BOM → Quality → Production. ChatGPT: majority unless hard audit date |
| Special process controls | Defer or include in Inspection (mixed) | CopperRiver: reject outright. Claude Desktop: include in Inspection |
| Timeline (no MRP) | 6-9 months (consensus range) | DarkOwl optimistic at 3-4 months |
| Cost rollup prerequisite | Claude Desktop: must design before any beads | Not raised by others but no disagreement |
