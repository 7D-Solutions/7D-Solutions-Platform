# MODE_OUTPUT_A8 — Edge-Case Analysis

**Mode:** A8 — Edge-Case  
**Analyst:** Codex  
**Date:** 2026-04-17  
**Scope:** `MODES_CONTEXT_PACK.md` + the 6 listed spec files

## 1. Thesis

The specs are coherent in the happy path, but they leave several edge-condition behaviors undefined at the exact points where operational systems usually fail: after a record is edited, after a workflow is partially completed, after a retry, or after configuration changes mid-flight. In this set, the most important gaps are stale derived fields, mutable stage metadata, append-only audit records without existence proof, stubbed return events that can advance lifecycle too early, non-idempotent training completion side effects, and vendor qualification changes that do not propagate to in-flight work. None of these are catastrophic in isolation, but together they create a class of hard-to-debug pre-launch failures where the system is technically valid and still wrong.

## 2. Top Findings

### §F1 — Complaint due dates can go stale after severity changes

**Evidence:** `docs/architecture/CUSTOMER-COMPLAINTS-MODULE-SPEC.md:51-53`, `:67-70`, `:133-137`, `:171`

**Reasoning chain:** The complaint model treats `due_date` as a derived SLA field, and the spec says it is auto-calculated on triage. But the same module also exposes a general `PUT /complaints/:id` update path while not closed, and there is no rule that recomputes `due_date` when severity changes or when triage is corrected later. In edge cases, the record stays internally consistent while the overdue sweep becomes wrong. That is exactly the kind of latent drift edge-case analysis is meant to catch.

**Severity:** Medium
**Confidence:** 0.91
**So What?:** Add an explicit recomputation rule for `due_date` on severity or triage changes, or freeze the fields that feed SLA once triage is complete.

### §F2 — CRM stage configuration is mutable enough to rewrite active-opportunity meaning

**Evidence:** `docs/architecture/CRM-PIPELINE-MODULE-SPEC.md:57-59`, `:101-105`, `:168-172`, `:202`

**Reasoning chain:** The pipeline stages are tenant-defined and can be added, deactivated, and reordered. The spec also says `opportunities.stage_code` must reference an active row, while historical references to inactive stages remain valid for closed opportunities. What is missing is the edge-case policy for open opportunities when a tenant changes the live stage definition. Reordering or flipping `is_terminal`/`is_win` can silently change the meaning of open deals without a migration step, which will surface later as reporting confusion or stranded opportunities.

**Severity:** High
**Confidence:** 0.88
**So What?:** Make stage changes versioned or require a controlled migration path for open opportunities before stage metadata can be rewritten.

### §F3 — Shop-Floor signoffs are append-only, but not existence-checked

**Evidence:** `docs/architecture/SHOP-FLOOR-GATES-MODULE-SPEC.md:56`, `:94-96`, `:178-179`

**Reasoning chain:** The signoff table tightly constrains `entity_type`, but the write API only asks for `entity_type`, `entity_id`, `role`, `signer_name`, and `action_context`. There is no invariant requiring the referenced entity to exist or belong to the same tenant. In an edge case, a malformed client or a race with a deleted record can create a durable audit row for a non-existent attestation target. Because signoffs are append-only, that bogus record is permanent.

**Severity:** Medium-High
**Confidence:** 0.86
**So What?:** Add write-time existence validation for every whitelisted entity type, and fail closed if the referenced entity is missing or cross-tenant.

### §F4 — Outside-Processing can advance on a return stub before the return is fully described

**Evidence:** `docs/architecture/OUTSIDE-PROCESSING-MODULE-SPEC.md:122-123`, `:145-149`, `:174-175`, `:209`

**Reasoning chain:** Shipping-Receiving is allowed to create a matching return-event stub, and the OP state machine advances from `at_vendor` to `returned` on the first return event. The spec also allows partial returns and mixed-condition returns as a future edge. That means the module can enter review flow before the return is complete or split into the correct condition groups. In the edge case where the stub is enough to satisfy workflow but not enough to describe reality, the module closes over incomplete physical evidence.

**Severity:** Medium
**Confidence:** 0.83
**So What?:** Distinguish `received_stub` from `finalized_return`, and gate review/close on the finalized state or on explicit override.

### §F5 — Training completion is not obviously idempotent

**Evidence:** `docs/architecture/PLATFORM-EXTENSIONS-SPEC.md:188-206`, `:210-211`

**Reasoning chain:** A passed training completion is supposed to create a competence assignment via an existing endpoint, and the result is only optionally reflected back as `resulting_competence_assignment_id`. The spec does not define an idempotency key, a uniqueness rule, or a retry-safe contract for the completion-to-assignment side effect. In the edge case where the completion call is retried after a timeout, the platform can create duplicate competence assignments or mark one completion as passed without a durable assignment link.

**Severity:** Medium
**Confidence:** 0.87
**So What?:** Make completion creation idempotent on `(assignment_id, completed_at)` or write the resulting competence assignment through an outbox with a uniqueness constraint.

### §F6 — Vendor qualification changes do not propagate to in-flight vendor work

**Evidence:** `docs/architecture/PLATFORM-EXTENSIONS-SPEC.md:223-248`, especially `:233-248`

**Reasoning chain:** The AP extension blocks new PO creation for `unqualified` and `disqualified` vendors, but that is only a point-in-time gate on `POST /api/ap/pos`. The spec does not say what happens to open POs, outside-processing orders, or other vendor-linked flows when a vendor is reclassified after the PO is already open. In edge cases, the vendor can move from qualified to disqualified while work is in flight, and nothing in the spec forces the rest of the platform to notice.

**Severity:** Medium
**Confidence:** 0.84
**So What?:** Add a vendor-qualification change consumer plan: open vendor-linked workflows should be flagged, frozen, or reapproved when status changes to restricted/disqualified.

## 3. Risks Identified

| Risk | Severity | Likelihood | Notes |
|------|----------|------------|-------|
| Complaint SLA sweeps drift after post-triage edits | Medium | High | §F1 |
| Open pipeline stages change meaning mid-quarter | High | Medium | §F2 |
| Bogus signoff records become permanent audit artifacts | Medium-High | Medium | §F3 |
| OP review/close can run off incomplete return data | Medium | Medium-High | §F4 |
| Duplicate competence assignments on retry | Medium | Medium | §F5 |
| Vendor disqualification does not affect in-flight work | Medium | Medium | §F6 |

## 4. Recommendations

| Priority | Recommendation | Effort | Expected Benefit |
|----------|----------------|--------|------------------|
| P0 | Add explicit derived-field refresh rules for complaint SLA fields | Low | Keeps overdue automation honest after edits |
| P0 | Version or migrate CRM pipeline stage definitions before live edits | Medium | Prevents semantic drift in open opportunities |
| P1 | Validate signoff targets exist and belong to the tenant | Low | Prevents durable bogus attestations |
| P1 | Split OP return stubs from finalized returns | Medium | Prevents workflow from closing on incomplete physical evidence |
| P1 | Make training completion idempotent | Medium | Prevents duplicate competence assignments under retries |
| P2 | Add consumer behavior for vendor qualification changes | Low | Surfaces mid-flight vendor risk across modules |

## 5. New Ideas and Extensions

### Incremental

- Add `due_date_recomputed_at` to complaint records so SLA drift is observable.
- Add a `stage_version` field to CRM opportunities so stage-definition changes are explicit.

### Significant

- Introduce a `finalized` state for outside-processing returns, separate from mere receipt.
- Require completion side effects in Workforce-Competence to be keyed by an idempotency token shared across retries.

### Radical

- Maintain a small platform-wide “derived field refresh” contract for any module that computes deadline-like fields from mutable inputs.

## 6. Assumptions Ledger

1. `PUT /api/customer-complaints/complaints/:id` can change severity or other SLA-driving fields before close.
2. Tenant admins are expected to mutate CRM pipeline configuration after opportunities already exist.
3. The signoff write path does not currently perform a synchronous existence check against the referenced shop-floor entity.
4. Shipping-Receiving’s return stub is allowed to be created before all return details are complete.
5. Training completion retries are plausible in the deployment environment and need to be safe.
6. AP vendor status changes can happen while other vendor-linked work is still open.

## 7. Questions for Project Owner

1. Should complaint due dates be recomputed automatically when severity changes after triage?
2. Are stage-definition edits allowed on live CRM pipelines, or must they be versioned and migrated?
3. Should signoff writes fail if the referenced entity does not exist at write time?
4. Is a return stub sufficient to enter review, or must OP wait for a finalized return record?
5. Do training completions need at-least-once or exactly-once side effects?
6. What should happen to open vendor-linked workflows when a vendor becomes disqualified?

## 8. Points of Uncertainty

- The specs do not say whether complaint severity is mutable after triage or only before it.
- The CRM spec allows stage reconfiguration, but it does not say whether open opportunities are migrated when stages are changed.
- The signoff model does not define cross-table existence validation, so I cannot tell whether this is intentionally deferred or simply omitted.
- The outside-processing return stub may already be finalized by Shipping-Receiving in practice, but the spec does not spell that out.
- The training completion endpoint may be wrapped by a hidden idempotency layer elsewhere, but that is not visible in the spec set.
- AP may already have a downstream vendor-risk process outside these docs, but nothing in the reviewed specs describes it.

## 9. Agreements and Tensions with Other Perspectives

- This mode agrees with the adversarial and root-cause perspectives that the dangerous failures live at workflow boundaries, not in core domain definitions.
- The tension is emphasis: those perspectives focus on control and ownership gaps, while this one focuses on rare-but-real state transitions, retries, and mutable configuration.
- I would expect strong overlap on the CRM, OP, and shop-floor findings, but this mode is narrower about how the failure emerges in the edge case rather than who owns the missing seam.

## 10. Confidence: 0.86

Calibration note: the confidence is high on the existence of the edge cases because the specs explicitly expose the relevant transitions and update paths. It is lower than a code-backed audit because I did not verify handler implementations, database constraints, or hidden idempotency logic.
