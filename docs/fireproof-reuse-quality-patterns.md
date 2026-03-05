# Fireproof ERP Reuse Investigation: Calibration Batch Pattern + Quality Inspection Enhancement

**Investigator:** DarkOwl
**Bead:** bd-1sqom
**Date:** 2026-03-05

---

## Executive Summary

The Fireproof gauge domain contains three reusable patterns relevant to the platform's quality-inspection module and manufacturing build:

1. **Batch workflow with item-level step gating** (calibration_batch.rs) — ADAPT-PATTERN
2. **State machine validation with ordinal-based sequencing** (status_machine.rs) — ADAPT-PATTERN
3. **Typed HTTP client + facade pattern** (maintenance/client.rs + facade.rs) — SKIP (already solved differently)

The biggest win is the batch workflow pattern, which directly addresses Phase C2's need for in-process and final inspection. The platform's current quality-inspection module has a flat disposition state machine (pending -> held -> accepted/rejected/released) but no concept of batch grouping or multi-step item workflows — exactly what C2 requires.

---

## Module-by-Module Assessment

### 1. calibration_batch.rs (531 LOC) — ADAPT-PATTERN

**What it does:**
A two-level state machine — batch lifecycle (Draft -> PendingSend -> Sent -> Received -> Completed + Cancelled) and item step lifecycle (Added -> Sent -> ReceivedPass/ReceivedFail -> CertVerified -> LocationVerified -> Released). Key mechanics:

- **Transition table as const array:** `BATCH_TRANSITIONS` and `ITEM_TRANSITIONS` define allowed moves as `(From, To)` pairs. Validation is a simple `.any()` lookup.
- **Ordinal-based step gating:** Each `ItemStep` has a numeric ordinal. The validator distinguishes "step skipped" (forward jump) from "step backward" (regression) using ordinal comparison.
- **Terminal state detection:** `is_terminal()` prevents advancement from ReceivedFail or Released.
- **Mutability gate:** `batch_is_mutable()` locks the batch once it leaves Draft/PendingSend — items can't be added/removed after the batch is sent.
- **Per-step input validation:** Each step has its own input struct (ReceiveInput, CertVerifyInput, LocationVerifyInput, ReleaseInput) with domain-specific validation rules.
- **Branching paths:** ReceivedPass and ReceivedFail share ordinal 3, allowing the pass/fail fork at the receive step.

**Why it matters for the platform:**

The platform's current quality-inspection module (service.rs:229-243) has a simple 2-level disposition machine:
```
pending -> held -> accepted | rejected | released
```

This is adequate for Phase C1 (receiving inspection) but insufficient for Phase C2 (in-process + final inspection), which needs:
- Grouping multiple inspections into a batch (e.g., "all in-process checks for WO-1234")
- Multi-step workflows per inspection item (measure -> record -> verify -> disposition)
- Step sequencing enforcement (can't skip measurement to go straight to disposition)
- Batch-level status tracking (are all items in this batch completed?)

**What to adapt (not extract verbatim):**

The calibration batch code is gauge-specific (gauge_id, certificate_number, calibration lab concepts). The *pattern* to adapt is:

1. **Batch entity + item entity with independent state machines** — the batch tracks overall lifecycle, each item tracks its own step progression.
2. **Const transition tables with validate function** — the platform already uses this pattern in production/operations.rs (pending -> in_progress -> completed) and quality-inspection/service.rs (disposition transitions). Standardize the approach.
3. **Ordinal-based step gating with skip/backward detection** — more sophisticated than what production operations currently does (which just checks `current != Pending` or `current != InProgress`).
4. **Per-step input validation structs** — clean separation of "what evidence is required at this step."

**Concrete application for Phase C2:**

```
InspectionBatch (batch-level):
  Draft -> InProgress -> Review -> Completed (+ Cancelled)

InspectionItem (item-level):
  Scheduled -> Measured -> Recorded -> Verified -> Dispositioned
  (with branching: Measured -> Failed at any point)
```

The batch groups inspection items for a work order or lot. Each item progresses through measurement -> recording -> verification -> disposition. The ordinal system prevents skipping verification to jump to disposition.

**LOC estimate:** ~200 LOC for the generic batch + item state machine types. The validation functions, transition tables, and input validation structs would be written fresh for quality-inspection's domain (not lifted from gauge code).

---

### 2. status_machine.rs (657 LOC) — ADAPT-PATTERN

**What it does:**
A comprehensive 12-status transition matrix for gauge lifecycle (Available, CheckedOut, PendingQc, CalibrationDue, OutOfService, etc.). Key mechanics:

- **Exhaustive matrix testing:** Every (from, to) pair is checked in tests — not just happy paths.
- **Error code mapping:** Each forbidden transition gets a specific error code (RETIRED_IS_TERMINAL, CHECKOUT_BLOCKED, CALIBRATION_SEND_BLOCKED). The error message includes from/to metadata.
- **Calculated status with priority ordering:** Derives the "should-be" status from multiple inputs (retirement state, calibration due date, checkout state) using a priority chain.
- **Eligibility checks:** `can_checkout()` provides a pre-flight check before attempting a transition.

**Comparison with platform:**

| Feature | Platform quality-inspection | Platform production ops | Fireproof status_machine |
|---------|---------------------------|------------------------|-------------------------|
| Transition validation | Match on current → allowed list | Status enum comparison | Const array + `.any()` lookup |
| Error codes | Generic message string | InvalidTransition { from, to } | Mapped codes + metadata HashMap |
| Exhaustive testing | No | No | Yes — full matrix |
| Pre-flight eligibility | No | PredecessorNotComplete check | `can_checkout()` |
| Calculated status | No | No | Priority-based derivation |

**What to adapt:**

The platform should not extract the gauge-specific status_machine.rs. But two patterns are worth standardizing:

1. **Exhaustive matrix tests** — every state machine in the platform should have a test that iterates all (from, to) pairs and asserts allowed vs. forbidden. Currently, neither quality-inspection nor production operations does this. This is a testing discipline, not code to extract.

2. **Error metadata on transitions** — the platform's current `QiError::Validation(String)` loses structured information. Fireproof's `DomainValidationError::with_metadata(msg, code, HashMap)` is richer. The platform's `event-bus` crate already has structured error patterns; quality-inspection should adopt similar structured error codes for disposition transitions.

**LOC estimate:** 0 LOC to extract. This is a pattern to follow when writing new state machines, not code to lift.

---

### 3. maintenance/client.rs (841 LOC) + facade.rs (688 LOC) — SKIP

**What it does:**

`MaintenanceClient` is a typed HTTP client with retry logic (exponential backoff), token-based auth, and generic GET/POST/PATCH helpers. The facade layer (facade.rs) translates between UI-facing DTOs (CreateGaugeRequest) and service DTOs (CreateAssetRequest), providing a thin orchestration layer.

**Why SKIP:**

- **Typed HTTP clients are vertical-specific.** Fireproof needs to call platform services because it's a separate application. Platform modules communicate via events (NATS) and direct database access, not HTTP-to-HTTP calls. The platform does not need a client SDK for its own modules.
- **The retry/backoff pattern is already standard.** Any vertical building on the platform can replicate this pattern — it's well-known Rust with reqwest.
- **The facade pattern is relevant but vertical-specific.** Fireproof maps gauge concepts to maintenance assets. A second vertical (food manufacturing) would have different facade mappings. This isn't extractable — it's a reference implementation.

**One exception:** If the platform ever publishes official client SDKs for verticals to consume, the `MaintenanceClient` pattern is the right template. But that's a Phase E+ concern, not current roadmap work.

---

## Cross-Cutting Observation: State Machine Consistency

The platform currently has three independent state machine implementations:

1. **quality-inspection/service.rs:229-243** — `validate_disposition_transition()` using match on current status returning allowed slice
2. **production/operations.rs:182-208** — Inline status check (`if current != OperationStatus::Pending`)
3. **workflow/domain/types.rs** — No state machine validation (uses `InstanceStatus` enum but transitions are handled elsewhere)

Fireproof's calibration_batch.rs and status_machine.rs both use the same pattern: **const transition table + validate function + ordinal for sequencing**. This is more composable and testable than the ad-hoc approaches in the platform.

**Recommendation:** Do NOT extract a shared crate yet. The three platform state machines are simple enough that they don't need abstraction. But when building Phase C2's batch inspection workflow, follow the Fireproof pattern (const table + validate fn + ordinal gating) rather than the current ad-hoc approaches. If a fourth state machine appears, consider a `state-machine-core` utility crate at that point.

---

## Mapping to Roadmap Deliverables

| Roadmap Phase | Deliverable | Fireproof Pattern Applicable | How |
|---------------|-------------|------------------------------|-----|
| **C1** | Inspector authorization via Workforce-Competence | No | Already scoped as workforce-competence integration |
| **C2** | In-process inspection records (linked to operations) | **Yes — calibration_batch pattern** | Batch groups inspection items per operation/WO. Item step gating enforces measure -> record -> verify -> disposition sequence. |
| **C2** | Final inspection records | **Yes — calibration_batch pattern** | Final inspection is a batch of all characteristics checked before FG receipt release. Batch-level "Completed" gates the release. |
| **C2** | Production event bridge (auto-create in-process inspections) | Partially — receipt_event_bridge pattern already in C1 | Extend existing bridge to listen for `production.operation_completed` and auto-create in-process inspection batches. |
| **E** | Maintenance workcenter consumption | No — client.rs is vertical-specific | Workcenter already owned by Production. Maintenance module consumes via events. |

---

## Recommended Beads for Extraction/Implementation

### Bead 1: "Batch inspection workflow model for Phase C2"
- Define `InspectionBatch` and `InspectionBatchItem` entities in quality-inspection module
- Implement batch-level state machine (Draft -> InProgress -> Review -> Completed + Cancelled)
- Implement item-level step gating with ordinal enforcement (Scheduled -> Measured -> Recorded -> Verified -> Dispositioned)
- Add const transition tables and validate functions following calibration_batch.rs pattern
- Exhaustive matrix tests for both batch and item state machines
- ~200-300 LOC new domain code

### Bead 2: "Production event bridge for in-process inspections"
- Subscribe to `production.operation_completed` events
- Auto-create inspection batches grouped by work order
- Link inspection items to specific operations via operation_id
- Reuse the existing receipt_event_bridge dedup pattern (quality_inspection_processed_events table)
- ~150 LOC

### Bead 3: "Final inspection gate for FG receipt"
- Create final inspection batch when all operations for a WO are completed
- Batch completion emits event that Production can consume to gate FG receipt
- Optional hold/release integration with existing disposition workflow
- ~150 LOC

---

## Summary Table

| Fireproof Component | LOC | Verdict | Reason |
|---------------------|-----|---------|--------|
| calibration_batch.rs | 531 | **ADAPT-PATTERN** | Batch + item dual state machine with ordinal gating directly applies to C2 inspection batches. Don't extract code; follow the pattern. |
| status_machine.rs | 657 | **ADAPT-PATTERN** | Exhaustive matrix testing and error code mapping are best practices to adopt. Don't extract code. |
| maintenance/client.rs | 841 | **SKIP** | Typed HTTP client for vertical-to-platform calls. Platform modules use events, not HTTP clients. |
| maintenance/facade.rs | 688 | **SKIP** | Vertical-specific DTO mapping. Reference only. |
| gauge_entity.rs, calibration.rs, etc. | ~1,973 | **SKIP** | Gauge-specific domain logic. Not extractable. |
