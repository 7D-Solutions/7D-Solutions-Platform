# Manufacturing Modules Scope Review — SageDesert

## Executive Summary
The proposed manufacturing expansion is directionally right, but the current 4-module shape hides major lifecycle and retrofit complexity. The biggest hole is Quality: it is not one module, it is at least three bounded contexts with very different state machines and evidence models. I recommend shipping a narrow, production-enabling core first (BOM + minimal Production + receiving/in-process quality gates) and deferring advanced quality and MRP optimization until retrofit foundations are stable.

## A. Module Boundaries
The 4-way split is close, but not clean enough for delivery sequencing.

- `BOM` should remain separate from Inventory. BOM is configuration/versioned product structure with effectivity and change governance, not stock state.
- `Production` should remain separate from Maintenance. Production executes manufacturing intent; Maintenance restores capability.
- `Quality` is too broad as proposed and should be split logically (even if initially in one repo/module):
  - Quality Planning/Execution (inspection plans, execution records, acceptance/reject)
  - Quality Events (NCR lifecycle)
  - Corrective System (CAPA lifecycle)
  - Special Process Assurance (parameter/certification records)
- `MRP/Planning` should be split into:
  - Material planning engine (netting/explosion suggestions)
  - Scheduling/finite capacity sequencing (later phase)

If kept as only 4 modules on paper, we should still enforce these sub-boundaries in APIs and event contracts from day one.

## B. Build Sequencing
The proposed order is valid, but MVP should be narrower than "BOM -> Production -> Quality -> MRP" as full modules.

Minimum viable manufacturing stack to unblock Fireproof:
- Phase 1: BOM core + Inventory retrofits (make/buy, production issue/receipt movements, tighter reservation semantics).
- Phase 2: Production core (work order lifecycle + component issue/finished good receipt + basic operation completion).
- Phase 3: Quality gate minimum (receiving/in-process/final pass-fail records and hold/release integration), not full NCR/CAPA.
- Phase 4: NCR and CAPA.
- Phase 5: MRP suggestions only (time-phased netting). Defer finite-capacity scheduling.

This gets manufacturing transactions running without waiting for full quality system and scheduling sophistication.

## C. Platform vs App-Specific Boundary
WhiteValley’s boundary is mostly right, but two areas need stricter generic definitions.

- Inspection plans with acceptance criteria and sampling rules can be generic if expressed as neutral constructs: characteristic, method, sample size rule, acceptance rule, and outcome.
- It becomes app-specific when encoded with aerospace standards, report templates, or compliance-specific mandatory fields.
- Special process controls are generic at the evidence level (machine, operator qualification, parameter capture, cert linkage, traceability).
- They become app-specific when the rule catalogs and accreditation logic (AS9100/NADCAP/ITAR-specific obligations) are baked into core state transitions.

Real boundary: platform stores auditable evidence + generic workflow hooks; app layers compliance rulebooks and external reporting formats.

## D. Existing Module Retrofit
Retrofit scope is currently underestimated.

Inventory likely needs:
- Item manufacturing policy (`make/buy`, possibly `phantom`, `subcontract` later).
- Inventory transaction extensions for production issue/return/receipt/scrap.
- Strong lot/serial genealogy hooks for component-to-parent trace.
- Reservation model updates for operation-level allocation and backflush semantics.

Maintenance likely needs:
- Workcenter master data (capacity units, calendars, constraints).
- Explicit machine-to-workcenter association.
- Downtime event semantics consumable by planning/scheduling.

Workflow likely needs:
- More reusable lifecycle templates and SLA defaults for NCR/CAPA/ECO.

Shipping-Receiving likely needs:
- Deeper receiving-inspection bridge into quality records and quarantine release criteria.

Numbering/Workforce-Competence likely need smaller changes, but integration touchpoints will still be non-trivial.

## E. Scope and Risk
This is much closer to a 12-month program than a 3-month delivery if implemented as full 4 modules with proper retrofits and integrated testing.

With 5 implementation agents:
- 3 months: possible for a constrained "manufacturing core" (BOM + basic Production transactions + minimal quality gates).
- 6-9 months: realistic for robust NCR/CAPA and stable cross-module integration.
- 9-12 months: realistic for dependable MRP plus scheduling and operational hardening.

What I would defer:
- Finite-capacity scheduling optimization.
- Advanced sampling rule engines.
- Special-process deep certification workflows beyond evidence capture.

## F. Dependencies and Integration
Dependency graph (high level):
- Inventory <- BOM (BOM references Inventory items)
- BOM <- Production (Production consumes BOM revisions)
- Inventory <-> Production (issue/receipt/reservations; avoid cyclic service calls via events)
- Production -> Quality (inspection triggers, nonconformance origins)
- Workforce-Competence -> Quality/Production (qualification checks)
- Maintenance -> Production/MRP (capacity/downtime inputs)
- BOM + Inventory + Production -> MRP

Potential circular dependency risk:
- Production needing synchronous Quality approval to proceed while Quality depends on Production operation state.
- MRP writing reservations directly while Inventory publishes availability events back to MRP recomputation.

Avoid cycles by contract:
- Use event-driven state propagation and explicit "decision ownership" per entity.
- Keep synchronous calls to validation-only paths, not cross-module state mutation.

Must-have day one integrations:
- BOM <-> Inventory item linkage.
- Production <-> Inventory issue/receipt.
- Production -> minimal quality disposition (pass/hold/fail).
- Quality hold/release -> Inventory status buckets.

Can wire later:
- CAPA automation triggers from NCR trends.
- Maintenance downtime feedback into MRP optimization.
- Advanced workforce qualification gating rules.

## G. Alternative Approaches
Some scope can be delivered by extending existing modules, but not all should.

- Do not fold BOM into Inventory core. It will pollute inventory semantics and create governance/versioning coupling.
- Do extend Inventory transaction taxonomy for production movements rather than creating a separate stock ledger.
- Do not implement Production as Maintenance work-order variants; lifecycle intent and cost/accounting semantics differ.
- Quality execution records could initially reuse Workflow + generic evidence storage patterns before full dedicated quality bounded contexts mature.
- MRP should start as a planner service consuming existing events/data, not as a deep rewrite of Inventory or Production.

## Top 3 Risks
1. Boundary erosion: Quality and Production concerns blend, creating tangled state machines and untestable cross-module behavior.
2. Retrofit underestimation: Inventory and Maintenance changes are foundational and could consume the first delivery windows.
3. Premature MRP ambition: pushing scheduling/optimization early will stall core transaction reliability.

## Recommended Approach
1. Approve manufacturing expansion, but reframe as a staged program with explicit sub-boundaries (especially inside Quality and MRP).
2. Build first: BOM core, Inventory retrofits, Production core transaction loop, minimal quality gates/holds.
3. Defer: full NCR/CAPA automation depth, special-process rule engines, finite-capacity scheduling.
4. Require architecture guardrails now: no cross-module mutation cycles, event contract versioning, and integration test suites per critical flow (issue -> build -> inspect -> receipt).
5. Do not create all beads at once; create phase-gated bead sets with exit criteria tied to end-to-end manufacturing transaction success.
