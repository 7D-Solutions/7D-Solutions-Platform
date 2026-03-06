# Manufacturing Build Roadmap — Claude Desktop Review

**Date:** 2026-03-04
**Reviewer:** Claude Desktop
**Document reviewed:** `docs/plans/MANUFACTURING-ROADMAP.md`
**Verdict:** APPROVED WITH NOTES (6 notes, 0 blocking)

---

## Overall Assessment

The roadmap is well-structured, correctly incorporates the 7-reviewer consensus and the prerequisites doc, and makes several smart deferral decisions (CostBreakdown to Phase B, workcenter directly in Production instead of the Maintenance transfer plan, S-R bridge removed from Phase A). The phase gating is sound and the scope fences are the right ones.

The notes below are refinements, not objections.

---

## Question 1: Does the phasing protect the cost rollup one-way door?

**Yes — the phasing protects it correctly.**

The cost rollup flows through three phases in the right order:

- **Phase 0** locks the design (cost rollup flow is deliverable #1).
- **Phase A** builds the interface: `source_type`, `produced` entry type, caller-provided `unit_cost_minor` on production receipts. Inventory doesn't compute cost — it accepts what it's given. This is the contract surface.
- **Phase B** builds the producer: Production accumulates component FIFO costs + labor + overhead, calls Inventory's receipt API with the rolled-up total. `CostBreakdown` JSONB lands here (correctly deferred — no producer exists in Phase A).

The one-way door is the Inventory receipt interface shape. Phase 0 designs it, Phase A implements it, Phase B consumes it. If Phase A's interface is wrong, Phase B is forced to work around it. But the prerequisites doc already defines the contract in detail (Section 1), so Phase 0's job is to confirm and sign off rather than invent from scratch.

**One note:** The Phase A "Prove at end" test says *"Issue components from Inventory (FIFO consumption) → post production receipt."* Since Production doesn't exist in Phase A, this test calls the Inventory receipt API directly with `source_type: "production"`. That's the right thing to do — it proves the interface works before any consumer exists — but the bead acceptance criteria should make this explicit so the implementing agent doesn't go looking for a Production caller.

---

## Question 2: Is Phase 0 (design lock) sufficient?

**Mostly yes — but two items need clarification.**

The five deliverables are well-chosen:

1. **Cost rollup flow design** — The prerequisites doc (Section 1) provides the contract. Phase 0 confirms it.
2. **Manufacturing identity graph** — This is new and valuable. A coherent identity scheme (how part numbers, revision numbers, WO numbers, lot numbers, inspection IDs, and ECO numbers relate) prevents naming collisions across modules. Good addition.
3. **WIP representation decision** — Also new and important. The choice between "WIP is an inventory location" vs. "WIP is a ledger-only concept in Production" affects the GL posting model and Inventory's location semantics. This must be settled before Phase A touches Inventory.
4. **Event contract naming** — Essential. The drill-down review flagged dot-vs-underscore inconsistencies in the lifecycle diagram. Locking the naming convention prevents downstream rework.
5. **Design doc signed off by all reviewers** — Good governance.

**What's unclear:**

- **BOM design decisions (depth, effectivity):** The prerequisites doc settled these (D6, D7, D8) — unlimited depth with 20-level guard, date-based effectivity only, `effectivity_type` enum seam. Are these considered already locked, or do they need re-confirmation in Phase 0? The roadmap doesn't list them as Phase 0 deliverables, but they're architectural decisions that affect the BOM schema. **Recommendation:** Add a line item to Phase 0: "BOM schema decisions confirmed (depth model, effectivity model)" — even if it's just a rubber stamp of the prerequisites doc. This makes it explicit that the implementing agent in Phase A doesn't need to re-litigate these choices.

- **Workcenter ownership path:** The roadmap chose a different path than the prerequisites doc. The prerequisites doc proposed temporary Maintenance ownership in Phase A with transfer to Production in Phase B. The roadmap instead defers workcenter entirely to Phase B (Production builds it from scratch, Maintenance consumes via events in Phase E). This is actually cleaner — no ownership transfer, no event namespace migration — but it means Maintenance's `workcenter_id` on `DowntimeEvent` remains a bare UUID with no FK target until Phase E. **Recommendation:** Phase 0 should explicitly note: "Workcenter master deferred to Phase B. Maintenance `workcenter_id` remains unvalidated until Phase E." This prevents a future agent from adding a workcenter table in Maintenance without realizing the roadmap already decided against it.

---

## Question 3: Are the "prove at end" acceptance tests right?

**Yes — they're at the right granularity with a few gaps to fill.**

| Phase | Tests | Assessment |
|-------|-------|------------|
| A | BOM creation + explosion + where-used; Inventory production receipt + component issue; events with correct envelope | **Good.** Covers the two key deliverables. Add: "existing purchase receipt behavior unchanged (regression)" — Inventory is the most load-bearing module and the retrofit must not break it. |
| B | Full WO floor loop; workcenter usage; correlation_id chain | **Good.** The correlation_id chain test is especially important — it proves the audit trail across BOM → issue → receipt. Add: "cost rollup arithmetic spot-check — sum of component FIFO costs ≤ FG receipt unit cost" to catch rounding or accumulation errors. |
| C | Receiving/in-process/final inspection; inspector authorization; evidence query | **Good.** The evidence query ("show inspection records for WO / lot / part revision") is the right audit question. Add: "quarantine/hold → disposition → release changes inventory status bucket" to prove the Inventory integration end-to-end. |
| D | ECO → approval → BOM revision superseded; doc releases; query by part/date | **Good.** No gaps. |
| E | Workcenter consistency across Production/Maintenance; downtime traceability | **Good.** Minimal and appropriate for the scope. |

**Summary of suggested additions:**

1. Phase A: regression test for existing purchase receipt path
2. Phase B: cost rollup arithmetic spot-check
3. Phase C: quarantine → disposition → inventory status bucket round-trip

---

## Question 4: Is anything missing for aerospace auditors?

**The audit readiness checklist covers the new manufacturing capabilities well. Two existing platform capabilities should be listed for completeness.**

The checklist at the bottom of the roadmap correctly claims that after Phase C, Fireproof can demonstrate: controlled product structure, controlled execution, cost rollup evidence, inspection governance, inspector authorization, and quarantine discipline. These are the manufacturing-specific audit requirements.

However, an AS9100 or NADCAP auditor will also ask about:

1. **Calibration traceability:** "Show me the calibration records for the equipment used to inspect this part." The platform already has this — Maintenance's calibration lifecycle (events: `maintenance.calibration.created`, `maintenance.calibration.completed`, `maintenance.calibration.status_changed`). The roadmap should list this as an existing capability that Fireproof can reference. It's not missing — it's just not mentioned, and auditors will ask.

2. **Training/competency records:** "Show me that the inspector was qualified on the date of inspection." The platform already has this via Workforce-Competence. Phase C's "inspector authorization via Workforce-Competence" covers the real-time check, but auditors also want historical proof — "this person was authorized at this timestamp." **Recommendation:** Confirm that Workforce-Competence stores point-in-time competency records (not just current state) or note that this needs verification.

3. **Document control:** ECOs in Phase D provide change control, but auditors also expect controlled work instructions, inspection procedures, and drawing revisions. If the Document-Control module exists or is planned, list it. If not, note that Fireproof handles document control in its vertical layer.

**Recommendation:** Add two rows to the audit readiness checklist:

| Capability | Source Phase |
|---|---|
| Calibration traceability for inspection equipment | Existing (Maintenance) |
| Inspector/operator competency records | Existing (Workforce-Competence) + Phase C integration |

This makes the checklist a complete audit-readiness picture rather than just a manufacturing-additions picture.

---

## Question 5: Is Phase D (ECO) correctly positioned as parallelizable?

**Yes — ECO is correctly parallelizable with late B / early C.**

Phase D's dependency is only on Phase A (BOM). ECO needs:

- `bom_revisions` table (to supersede revisions) — exists after Phase A
- Workflow module (for approval lifecycle) — already exists
- Numbering module (for ECO identifiers) — already exists

Phase D does NOT need:

- Production work orders (ECOs change the product definition, not the execution plan)
- Inspection records (ECOs can trigger new inspection plans, but that's a downstream consumer, not a dependency)

The roadmap says "Can parallel late Phase B / early Phase C" which is conservative and correct. In theory, Phase D could start as soon as Phase A completes. The "late B" qualifier is probably pragmatic — don't split agent attention across too many fronts — rather than a hard dependency. That's fine.

**One consideration:** Phase C's "prove at end" doesn't reference ECO-driven changes, and Phase D's "prove at end" doesn't reference inspection plan updates. This means the two are truly independent in v1, which is correct. In a future version, an ECO might trigger a new inspection plan (per the synthesis doc's item G: "NCR/CAPA → Inspection Plans feedback loop"), but that's post-v1.

---

## Question 6: Does the update log structure work?

**It works for tracking what changed. It should track one more thing: proof artifacts.**

The current structure:

| Date | Phase | What Changed | Who |
|------|-------|-------------|-----|

This is sufficient for change tracking. Each bead updates this log when it marks a deliverable DONE. The bead ID column in the deliverable tables provides traceability back to the implementation.

**Recommended addition:** A `Proof` column (or a separate "proof register" section) that links to test evidence. When a phase's "prove at end" tests pass, the log entry should reference the proof:

| Date | Phase | What Changed | Who | Proof |
|------|-------|-------------|-----|-------|
| 2026-03-15 | A | Inventory retrofit complete | bd-xxxxx | `e2e-tests/manufacturing/phase_a_inventory.rs` |
| 2026-03-20 | A | BOM module v1.0.0 proven | bd-yyyyy | `modules/bom/tests/integration/` |

This matters because:

- The roadmap is a living audit artifact. An orchestrator reviewing it 3 months from now should be able to trace from "DONE" to the test that proved it.
- Aerospace auditors specifically look for objective evidence of verification. The proof link is that evidence.
- It prevents "DONE but untested" drift — if there's no proof link, the deliverable isn't truly done.

If a separate proof column feels too heavy, an alternative is: each "prove at end" section gets a `Proof artifacts` sub-row that's filled in when the phase completes. Either way, the key is linking completion claims to test evidence.

---

## Summary of Notes

| # | Note | Severity | Recommendation |
|---|------|----------|---------------|
| 1 | Phase A "prove at end" should clarify that production receipt test calls API directly (no Production module yet) | Minor | Add note to bead acceptance criteria |
| 2 | Phase 0 should list BOM design decisions (depth, effectivity) as confirmed items | Minor | Add deliverable line: "BOM schema decisions confirmed" |
| 3 | Phase 0 should note workcenter ownership path decision explicitly | Minor | Add note: workcenter deferred to Phase B, Maintenance workcenter_id unvalidated until Phase E |
| 4 | "Prove at end" sections need 3 additional tests (see Q3 table) | Medium | Add regression test (A), cost arithmetic check (B), quarantine round-trip (C) |
| 5 | Audit readiness checklist should list existing platform capabilities (calibration, competency) | Medium | Add 2 rows to checklist |
| 6 | Update log should track proof artifacts linking DONE claims to test evidence | Medium | Add Proof column or proof register section |

---

*Review complete. The roadmap is ready for implementation with the notes above incorporated.*
