# Manufacturing Build Roadmap — Living Document

**Created:** 2026-03-04
**Last Updated:** 2026-03-04
**Owner:** BrightHill (Orchestrator)
**Status:** DRAFT — Under Review

---

## Purpose

This is the single source of truth for the manufacturing build. Every agent working on manufacturing beads must read this document before starting and update it when completing milestones. No exceptions.

**Rule:** Every manufacturing bead includes this instruction in its acceptance criteria:
> Update `docs/plans/MANUFACTURING-ROADMAP.md` — mark the deliverable row(s) you touched as DONE with the date and bead ID. Update specific rows, not just phase status.

---

## Scope Fences (Permanent)

These constraints apply to ALL phases. They don't change without orchestrator + ChatGPT approval.

- **Discrete manufacturing only** — no process/recipe BOM, no repetitive/rate-based, no mixed-mode
- **No backflush in v1** — explicit component issue only (operator scans each part)
- **No MRP/Planning** — manual work order creation
- **No NCR/CAPA lifecycle** — Phase C provides inspection + hold/release only; NCR/CAPA workflow is a separate future module
- **No special process rule catalogs** — platform provides generic evidence capture; aerospace rules live in Fireproof
- **No production scheduling/capacity optimization**
- **No process manufacturing**
- **Tests are integrated** — real Postgres, real NATS, no mocks, no stubs

---

## Phase Summary

| Phase | Goal | Beads | Status |
|-------|------|-------|--------|
| 0 | Design lock — cost rollup + identity graph + naming | 1 | COMPLETE |
| A | Inventory retrofit + BOM core | 2 | COMPLETE |
| B | Production v1 execution spine | 3-4 | NOT STARTED |
| C1 | Quality — Receiving inspection | 1-2 | NOT STARTED |
| C2 | Quality — In-process + final inspection | 2-3 | NOT STARTED |
| D | ECO + Change Control | 2-3 | NOT STARTED |
| E | Maintenance workcenter consumption | 2 | NOT STARTED |

**Dependency chain:** 0 → A → B → C2. C1 depends on A (not B). Phase D can parallel late B / early C. Phase E depends on B.

---

## Phase 0 — Design Lock

**Goal:** Prevent one-way-door mistakes before writing code.

| Deliverable | Status | Bead | Date |
|-------------|--------|------|------|
| Cost rollup flow design (component FIFO → FG unit cost → GL journal) | DONE | bd-p4mx2 | 2026-03-05 |
| Manufacturing identity graph (part/rev, WO/build, lot/serial, inspection IDs) | DONE | bd-p4mx2 | 2026-03-05 |
| WIP representation decision (inventory location vs ledger-only) | DONE | bd-p4mx2 | 2026-03-05 |
| Variance handling policy v1 (explicitly disallow OR define minimal allowed cases) | DONE | bd-p4mx2 | 2026-03-05 |
| GL posting trigger model (which event triggers GL, who posts, minimal payload) | DONE | bd-p4mx2 | 2026-03-05 |
| BOM schema decisions confirmed (depth model, effectivity model, from prerequisites doc) | DONE | bd-p4mx2 | 2026-03-05 |
| Workcenter ownership path confirmed (Production owns from Phase B; Maintenance workcenter_id unvalidated until Phase E) | DONE | bd-p4mx2 | 2026-03-05 |
| Event contract naming review (existing modules + new manufacturing subjects) | DONE | bd-p4mx2 | 2026-03-05 |
| Design doc signed off by all reviewers | DONE | bd-p4mx2 | 2026-03-05 |

**Not in this phase:** Any implementation code.

**Exit criteria:** Design doc approved. Phase A cannot start without this.

---

## Phase A — Inventory Retrofit + BOM Core

**Goal:** Product structure and inventory movements needed for production.

**Depends on:** Phase 0

| Deliverable | Status | Bead | Date |
|-------------|--------|------|------|
| Inventory: `source_type` field on receipts (purchase/production/return) | DONE | bd-194cd | 2026-03-05 |
| Inventory: production receipt path (caller-provided unit cost) | DONE | bd-194cd | 2026-03-05 |
| Inventory: issue path with source_type tagging (no work_order_id yet) | DONE | bd-194cd | 2026-03-05 |
| Inventory: make/buy classification on items | DONE | bd-194cd | 2026-03-05 |
| Inventory: event payload extended with source_type | DONE | bd-194cd | 2026-03-05 |
| BOM: module scaffold (bom-rs crate) | DONE | bd-1uy2l | 2026-03-05 |
| BOM: multi-level structure (header, revision, lines) | DONE | bd-1uy2l | 2026-03-05 |
| BOM: date-based effectivity with non-overlapping constraint | DONE | bd-1uy2l | 2026-03-05 |
| BOM: multi-level explosion query with depth guard | DONE | bd-1uy2l | 2026-03-05 |
| BOM: where-used reverse lookup | DONE | bd-1uy2l | 2026-03-05 |
| BOM: events emitted via outbox | DONE | bd-1uy2l | 2026-03-05 |
| GL: consumer branches by source_type (COGS vs WIP) + production receipt GL path | DONE | bd-2vc9u | 2026-03-05 |
| Docker: bom-rs container with compose watch + CI build job | DONE | bd-1mgdw | 2026-03-05 |
| Integration proof: BOM + Inventory end-to-end (5 tests prove Phase A exit criteria) | DONE | bd-2g7el | 2026-03-05 |

**Not in this phase:** ECO lifecycle, workcenters (Production owns from Phase B — no temporary table in Maintenance), inspection bridge, CostBreakdown JSONB, backflush, serial-number effectivity, `produced` entry_type enum (source_type disambiguates).

**Prove at end:**
- Create part + BOM revision + effectivity → query where-used
- Inventory accepts source_type=production receipt path and source_type-tagged issue path via integration tests (retrofit capability — test calls Inventory API directly, no Production module caller yet)
- Existing purchase receipt path unchanged (regression test)
- Events emitted with correct envelope metadata, replay-safe

---

## Phase B — Production v1 Execution Spine

**Goal:** Run a real floor loop: work order → issue components → execute operations → receipt finished goods.

**Depends on:** Phase A

| Deliverable | Status | Bead | Date |
|-------------|--------|------|------|
| Production: module scaffold (production-rs crate) | NOT STARTED | — | — |
| Production: work order lifecycle (create/release/close) | NOT STARTED | — | — |
| Production: workcenter master table (owned by Production) | NOT STARTED | — | — |
| Production: routing/operations model (sequence, workcenter, status) | NOT STARTED | — | — |
| Production: explicit component issue workflow → Inventory | NOT STARTED | — | — |
| Production: FG receipt → Inventory at rolled-up cost | NOT STARTED | — | — |
| Production: timekeeping link (operation events → clock events) — optional B-late | NOT STARTED | — | — |
| Docker: production-rs container with compose watch | NOT STARTED | — | — |

**Not in this phase:** Backflush, CostBreakdown JSONB (deferred to future costing phase), quality inspection execution, ECO, NCR/CAPA, capacity planning, scheduling.

**Prove at end:**
- WO created → components issued (FIFO consumed) → operations completed → FG receipt at rolled-up cost
- Cost rollup arithmetic spot-check: sum of component FIFO costs ≤ FG receipt unit cost
- Workcenter definitions used by operations
- Audit trace: correlation_id chains WO → issue → receipt events

---

## Phase C1 — Quality: Receiving Inspection

**Goal:** Inspection evidence for incoming materials — can ship without waiting for Production.

**Depends on:** Phase A (item/revision anchors, inventory receipts)

| Deliverable | Status | Bead | Date |
|-------------|--------|------|------|
| Inspection: module scaffold (quality-inspection-rs crate) | DONE | bd-2f1xv | 2026-03-05 |
| Inspection: inspection plan model (characteristics, tolerances, sampling) | DONE | bd-1y2nc | 2026-03-05 |
| Inspection: receiving inspection records | DONE | bd-1y2nc | 2026-03-05 |
| Inspection: quarantine/hold before disposition | DONE | bd-16fy6 | 2026-03-05 |
| Inspection: disposition outcomes (accept, reject-to-hold, release) | DONE | bd-16fy6 | 2026-03-05 |
| Inspection: inspector authorization via Workforce-Competence | NOT STARTED | — | — |
| Inspection: S-R event bridge (auto-create receiving inspection) | NOT STARTED | — | — |
| Docker: quality-inspection-rs container with compose watch | DONE | bd-2f1xv | 2026-03-05 |

**Not in this phase:** In-process/final inspection (Phase C2), NCR/CAPA lifecycle, special process catalogs, automated sampling rule libraries.

**Prove at end:**
- Receiving inspection record created from S-R event
- Quarantine/hold enforced before disposition
- Disposition recorded; release emits event that Inventory/Shipping can consume to control usage (end-to-end round-trip)
- Inspector authorization checked and logged
- Evidence query: "show inspection records for part revision / receipt"

---

## Phase C2 — Quality: In-Process + Final Inspection

**Goal:** Production-integrated inspection — in-process checks between operations and final inspection before release.

**Depends on:** Phase B (production operations), Phase C1 (inspection model + scaffold)

| Deliverable | Status | Bead | Date |
|-------------|--------|------|------|
| Inspection: in-process inspection records (linked to operations) | NOT STARTED | — | — |
| Inspection: final inspection records | NOT STARTED | — | — |
| Inspection: production event bridge (auto-create in-process inspections) | NOT STARTED | — | — |

**Not in this phase:** NCR/CAPA lifecycle, special process catalogs, automated sampling rule libraries.

**Prove at end:**
- In-process checks recorded between production operations
- Final inspection recorded before shipment/customer acceptance; FG receipt may be gated by hold/release policy
- Evidence query: "show inspection records for WO / lot / part revision"

---

## Phase D — ECO + Change Control

**Goal:** Formalize change control evidence for audit governance.

**Depends on:** Phase A (BOM). Can parallel late Phase B / early Phase C.

| Deliverable | Status | Bead | Date |
|-------------|--------|------|------|
| ECO: entity + workflow template integration | NOT STARTED | — | — |
| ECO: links to BOM revision changes + released docs | NOT STARTED | — | — |
| ECO: numbering integration for ECO identifiers | NOT STARTED | — | — |

**Not in this phase:** Deviation/waiver systems.

**Prove at end:**
- ECO created → approvals → BOM revision superseded with effectivity date
- Related doc revisions released alongside ECO
- Query: "ECO history for part" and "BOM rev effective on date X"

---

## Phase E — Maintenance Workcenter Consumption

**Goal:** Close the loop between production execution and maintenance.

**Depends on:** Phase B (workcenter master)

| Deliverable | Status | Bead | Date |
|-------------|--------|------|------|
| Maintenance: consumes Production workcenters (events/API) | NOT STARTED | — | — |
| Maintenance: downtime events linked to workcenter/asset | NOT STARTED | — | — |
| Production: downtime signals for breakdown triggers | NOT STARTED | — | — |

**Not in this phase:** Full CMMS expansion, scheduling/planning.

**Prove at end:**
- Workcenter list consistent across Production and Maintenance
- Downtime recorded and traceable from production event to maintenance record

---

## Audit Readiness Checklist

At the end of Phase C2, Fireproof can demonstrate:

| Capability | Source Phase |
|------------|-------------|
| Controlled product structure (BOM revisions + effectivity) | A |
| Controlled execution (work orders + operations + material trace) | B |
| Cost rollup evidence (FIFO consumption → FG receipt) | B |
| Receiving inspection governance + executed records | C1 |
| Inspector authorization + audit trail | C1 |
| Quarantine/hold discipline | C1 |
| In-process + final inspection evidence | C2 |
| Calibration traceability for inspection equipment | Existing (Maintenance) |
| Inspector/operator competency records | Existing (Workforce-Competence) + C1 integration |

**If audit requires change control evidence:** Phase D must complete before audit date.
**Note:** Calibration and competency records exist in platform today. Verify Workforce-Competence stores point-in-time records (not just current state) for historical audit proof.

---

## Deferred Items

Items explicitly excluded from this roadmap. Will be addressed in future program phases.

| Item | Reason | Earliest Revisit |
|------|--------|-------------------|
| CostBreakdown JSONB (material/labor/overhead split) | Over-engineering for v1 — source_type + unit_cost sufficient | After Phase B |
| Backflush consumption | Explicit issue only in v1 | After Phase B proven |
| Serial-number effectivity | Date-based only in v1 | After Phase A proven |
| NCR/CAPA lifecycle | Separate module — Phase C provides hold/release only | After Phase C2 |
| MRP / production scheduling | Manual WO creation in v1 | Future program phase |
| Process manufacturing | Discrete only | Out of scope |
| Special process rule catalogs | Platform provides generic evidence; rules live in Fireproof | Future Fireproof concern |
| `produced` entry_type enum | source_type already disambiguates receipts | Revisit if needed |
| Deviation/waiver systems | Beyond ECO scope | After Phase D |

---

## Key Decisions

| Decision | Outcome | Date |
|----------|---------|------|
| Workcenter ownership | Production owns from Phase B. No temporary table in Maintenance during Phase A. Maintenance references bare IDs until Phase E. | 2026-03-04 |
| CostBreakdown JSONB | Deferred beyond Phase B v1. source_type + caller-provided unit_cost is sufficient. | 2026-03-04 |
| Phase C split | Split into C1 (receiving inspection, depends on A) and C2 (in-process/final, depends on B). Lets quality start earlier. | 2026-03-04 |
| source_type values | purchase / production / return (return added as harmless enum value) | 2026-03-04 |

---

## Update Log

| Date | Phase | What Changed | Who | Proof |
|------|-------|-------------|-----|-------|
| 2026-03-04 | — | Document created from ChatGPT roadmap + 7-reviewer synthesis | BrightHill | — |
| 2026-03-04 | All | Incorporated 5 agent reviews + ChatGPT review: split Phase C, trimmed Phase A, deferred CostBreakdown JSONB, added variance policy + GL trigger model to Phase 0, added Deferred Items + Key Decisions sections | BrightHill | — |
| 2026-03-04 | All | Incorporated Claude Desktop review: added BOM/workcenter decisions to Phase 0, added regression test (A), cost arithmetic check (B), quarantine round-trip (C1), added existing capabilities to audit checklist, added Proof column | BrightHill | — |
| 2026-03-05 | 0 | Phase 0 design lock document drafted (bd-p4mx2): cost rollup flow, identity graph, WIP decision, variance policy, GL trigger model, BOM/workcenter confirmations, event naming review. Pending sign-off. | MaroonHarbor | docs/plans/MANUFACTURING-DESIGN-LOCK.md |
| 2026-03-05 | A | Inventory retrofit complete (bd-194cd): source_type on receipts + ledger + events, production/return receipt paths, make/buy classification with Guard→Mutation→Outbox pattern, event contract extended. 237 unit tests pass. Integration tests blocked by pre-existing DB TLS issue (bd-194cd.1). | MaroonHarbor | modules/inventory/tests/phase_a_integration.rs |
| 2026-03-05 | A | BOM core module complete (bd-1uy2l): scaffold, header/revision/line CRUD, date-based effectivity with exclusion constraint, multi-level explosion with depth guard + cycle detection, where-used reverse lookup, outbox events (6 event types). 5 unit + 7 integration tests pass against real Postgres. | PurpleCliff | modules/bom/tests/bom_integration.rs |
| 2026-03-05 | A | GL consumer source_type branching (bd-2vc9u): item_issued branches purchase→COGS / production→WIP, item_received production→FG receipt (DR INVENTORY / CR WIP). Unknown source_type hard-fails. New SourceDocType variants (ProductionIssue, ProductionReceipt). 5 integration tests pass against real GL DB. | CopperRiver | modules/gl/tests/gl_inventory_source_type_test.rs |
| 2026-03-05 | A | BOM Docker/CI wiring (bd-1mgdw): Dockerfile.workspace (multi-stage cargo-chef), compose service on port 8107, gateway depends_on, CI build-bom job, fixed port conflict (8098→8107). Service catalog auto-updated. | PurpleCliff | modules/bom/Dockerfile.workspace |
| 2026-03-05 | C1 | Quality inspection scaffold complete (bd-2f1xv): quality-inspection-rs crate with Axum app, health/ready/version endpoints, Prometheus metrics, outbox pattern, migration (inspection_plans, inspections, dispositions, outbox, processed_events). Docker container + compose service on port 8106, DB on port 5459. Builds and passes all tests. | DarkOwl | modules/quality-inspection/ |
| 2026-03-05 | A | Integration proof complete (bd-2g7el): 5 e2e tests against real Postgres — BOM structure/effectivity/where-used/explosion, production receipt with source_type, issue with source_type tagging, purchase receipt regression, depth guard. All Phase A exit criteria proven. | CopperRiver | e2e-tests/tests/manufacturing_phase_a_e2e.rs |
| 2026-03-05 | C1 | Inspection plan model + receiving inspection core (bd-1y2nc): characteristics JSONB, tolerances, sampling method/size on plans. Receiving inspections with receipt_id/part_id/part_revision anchors. Query by part-rev and by receipt. Plan activation workflow. Events via outbox (plan_created, inspection_recorded). Permission constants added. 6 integration tests pass against real Postgres. | DarkOwl | modules/quality-inspection/tests/inspection_integration.rs |
| 2026-03-05 | C1 | Quarantine/hold + disposition outcomes (bd-16fy6): disposition state machine on inspections (pending→held→accepted/rejected/released). Hold enforced before final disposition. 4 event types emitted (held, released, accepted, rejected) with full envelope metadata. Inspector ID + reason tracked. HTTP endpoints for all transitions. 5 new integration tests (valid transitions, illegal transitions rejected, event emission verified). 11 total tests pass. | DarkOwl | modules/quality-inspection/tests/inspection_integration.rs |
