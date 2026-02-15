# ADR-015: Phase 15 Lifecycle Enforcement Posture (Option B-lite)

**Status:** Accepted
**Date:** 2026-02-15
**Deciders:** ChatGPT (Strategic Architect), PearlOwl (Coordinator)
**Phase:** 15 - Billing Lifecycle Hardening (Deterministic Execution Layer)

---

## Context

Phase 15 transforms billing from "feature-complete modules" into a deterministic, formally guarded, invariant-proven lifecycle engine. The core decision: **where do we enforce lifecycle mutation controls?**

Two options were evaluated:

### Option A: Reinforce In-Place
- Keep route handlers as the mutation sites
- Require all lifecycle-critical mutations to call `transition_guard()` functions
- Add test enforcement to detect/forbid direct status updates that bypass guards
- **Pros:** Minimal refactor footprint, lowest immediate risk, fastest to implement
- **Cons:** HTTP layer remains the mutation owner, higher drift risk, enforcement relies more on tests/conventions

### Option B-lite: Lifecycle Module Per Service
- Introduce lifecycle submodules (`subscriptions::lifecycle`, `ar::lifecycle`, `payments::lifecycle`)
- Move **ONLY** lifecycle-critical mutations into these functions
- Routes become thin orchestrators that call lifecycle functions
- No broad restructure (surgical, not architectural overhaul)
- **Pros:** Single choke-point mutation control per module, lower drift risk, cleaner invariants and concurrency reasoning
- **Cons:** Slight refactor footprint (surgical but real), touches more files than Option A

---

## Decision

**We choose Option B-lite: Lifecycle Module Per Service**

**Rationale:**
1. **Industry best practice:** Lifecycle modules provide clear mutation boundaries and reduce drift risk over time
2. **ChatGPT recommendation:** "Option B-lite is the better long-term posture"
3. **Surgical implementation:** Focused refactor touching only lifecycle-critical paths, not a full rewrite
4. **Enforcement clarity:** Easier to validate "no direct status updates outside lifecycle module" than "all routes must call guards"
5. **Phase 15 principles alignment:** Deterministic behavior, exactly-once guarantees, formal state machines

**ChatGPT Approval:** ✅ "Final approval granted" (2026-02-15)

---

## Enforcement Rules

### Mutation Ownership Rule (HARD)

**After Phase 15 bd-15.3x (Lifecycle Guards) lands:**

❌ **FORBIDDEN:**
- No route/handler may update lifecycle status columns directly
- No SQL `UPDATE` statements on status columns outside lifecycle modules

✅ **REQUIRED:**
- All lifecycle status writes **MUST** go through `*_::lifecycle::*` functions
- Guards inside lifecycle modules are the **only** mutation entry points

**Enforcement:**
- Treat direct SQL updates as **failing defects**
- Code reviews must check for direct status mutations
- Tests must assert routes cannot mutate status without calling lifecycle APIs

**Example:**
```rust
// ❌ FORBIDDEN (after bd-15.3x)
sqlx::query("UPDATE subscriptions SET status = 'SUSPENDED' WHERE id = $1")
  .execute(&pool).await?;

// ✅ REQUIRED
subscriptions::lifecycle::transition_to_suspended(sub_id, reason, &pool).await?;
```

### Exactly-Once Rule (HARD)

**Side effects** (PSP call, finalize emit, notification emit, ledger post) **may only occur** when:
- The attempt row is **newly created**, OR
- The attempt transitions via guard from a **retry-eligible state**

**Duplicate triggers MUST be deterministic no-ops.**

**Enforcement:**
- Use attempt ledger UNIQUE constraints (bd-15.1)
- Use `SELECT FOR UPDATE` for aggregate locks (bd-15.4x)
- Use deterministic idempotency keys (bd-15.2)
- Reject duplicate attempts at DB level (UNIQUE violation → deterministic no-op)

---

## Lifecycle Modules Structure

### subscriptions::lifecycle
**States:** ACTIVE, PAST_DUE, SUSPENDED
**Guard:** `transition_guard(from, to, reason) -> Result<(), TransitionError>`
**Entry Point:** Routes/services call lifecycle functions only
**Bead:** bd-138 (bd-15.3a)

### ar::lifecycle
**States:** OPEN, ATTEMPTING, PAID, FAILED_FINAL
**Guard:** `transition_guard(from, to, reason) -> Result<(), TransitionError>`
**Entry Point:** Finalize/attempt paths call lifecycle functions only
**Bead:** bd-1w7 (bd-15.3b)

### payments::lifecycle
**States:** ATTEMPTING, SUCCEEDED, FAILED_RETRY, FAILED_FINAL, UNKNOWN
**Guard:** `attempt_transition_guard(from, to, reason) -> Result<(), TransitionError>`
**Entry Point:** Webhook handlers call lifecycle functions only
**Bead:** bd-3lm (bd-15.3c)

---

## State Machines and Invariants

### Subscription State Machine
```
ACTIVE ──> PAST_DUE ──> SUSPENDED
  ^                         |
  └─────────────────────────┘
```

**Invariants:**
- Exactly one invoice per subscription-cycle
- Suspension only when allowed (not while UNKNOWN payment exists)

### Invoice State Machine
```
OPEN ──> ATTEMPTING ──> PAID
  |                       |
  └──> FAILED_FINAL <─────┘
```

**Invariants:**
- No duplicate invoice attempts (UNIQUE constraints)
- Exactly one finalization side effect per attempt

### Payment Attempt State Machine
```
ATTEMPTING ──> SUCCEEDED
  |
  ├──> FAILED_RETRY ──> ATTEMPTING (retry window)
  |
  ├──> FAILED_FINAL (terminal)
  |
  └──> UNKNOWN ──> reconciliation ──> SUCCEEDED / FAILED_*
```

**Invariants:**
- No duplicate payment attempts (UNIQUE constraints)
- UNKNOWN blocks retries and subscription suspension
- Exactly one attempt per retry window [0d, +3d, +7d]

---

## Code Review Checklist

**For each lifecycle bead (bd-15.3x through bd-15.6x), reviewers MUST verify:**

### Mutation Ownership
- [ ] All status mutations route through `lifecycle::*` functions
- [ ] No direct SQL `UPDATE` of status columns outside lifecycle module
- [ ] Transition guards reject illegal transitions with zero side effects
- [ ] Tests assert routes cannot mutate status without calling lifecycle API

### Exactly-Once Behavior
- [ ] Attempt row insertion uses UNIQUE constraints
- [ ] Duplicate attempt insertion → deterministic no-op or failure
- [ ] Side effects only occur on successful attempt row creation
- [ ] Concurrency tests prove exactly-once behavior

### Retry Discipline
- [ ] Exactly one attempt per window enforced by UNIQUE (tenant_id, entity_id, attempt_no)
- [ ] Duplicate triggers within window → no second attempt created
- [ ] Side effects only on new attempt creation

### UNKNOWN Protocol (Payments)
- [ ] UNKNOWN blocks retry scheduling
- [ ] UNKNOWN blocks subscription suspension
- [ ] Reconciliation workflow is deterministic and bounded

---

## Consequences

### Positive
1. **Clear mutation boundaries:** Single lifecycle module per service owns all status transitions
2. **Lower drift risk:** Future features cannot bypass lifecycle controls without explicit module changes
3. **Easier invariant reasoning:** All lifecycle logic centralized, easier to verify correctness
4. **Production-grade concurrency:** SELECT FOR UPDATE + attempt ledgers provide exactly-once guarantees
5. **ChatGPT-approved architecture:** Aligns with strategic guidance for deterministic execution

### Negative
1. **Refactor footprint:** Touches more files than Option A (surgical but real changes)
2. **Implementation time:** Slightly longer than Option A due to module creation
3. **Agent coordination:** Requires careful dependency management across 15 beads

### Neutral
1. **No new states beyond specified:** Scope remains bounded to existing lifecycle needs
2. **No configurability:** Fixed retry windows [0d, +3d, +7d], no per-tenant overrides
3. **No scale/perf work:** Focus is correctness, not optimization

---

## Implementation Plan

**Phase 15 Beads (15 total):**

### Track-0: Architecture
- bd-13u (bd-15.0): This ADR ← **Current bead**

### Track-A: Schema & Core
- bd-7gl (bd-15.1): Attempt Ledgers + DB Uniqueness
- bd-1p2 (bd-15.2): Deterministic Idempotency Key Spec

### Track-B: Lifecycle Guards (3 parallel)
- bd-138 (bd-15.3a): Subscriptions Transition Guards
- bd-1w7 (bd-15.3b): AR Invoice Transition Guards
- bd-3lm (bd-15.3c): Payments Attempt Transition Guards

### Track-C: Gating (3 parallel)
- bd-184 (bd-15.4a): Subscriptions Cycle Gating
- bd-3fo (bd-15.4b): AR Finalization Gating
- bd-1wg (bd-15.4c): Payments Gating + Webhook Mutation Order

### Track-B: Advanced Protocols
- bd-2uw (bd-15.5): UNKNOWN Protocol + Deterministic Reconciliation
- bd-8ev (bd-15.6a): Retry Window Discipline (AR)
- bd-1it (bd-15.6b): Retry Window Discipline (Payments) with UNKNOWN Block

### Track-D: Correctness
- bd-35x (bd-15.7a): Module-Level Invariant Primitives
- bd-3rc (bd-15.7b): Cross-Module Invariant Suite

### Track-E: Simulation
- bd-3c2 (bd-15.8): Deterministic Simulation Harness

**Parallelization:** 3-4 agents can work concurrently across tracks

---

## References

- **ChatGPT Handoff:** https://chatgpt.com/g/g-p-698c7e2090308191ba6e6eac93e3cc59/c/6991d3bb-cbb4-8332-b22d-ec317d74cf6c
- **Coordination Rules:** docs/PHASE-15-COORDINATION-RULES.md
- **Phase 15 JSON Spec:** /tmp/phase15-beads-final.json (coordinator artifact)
- **Related ADRs:** None (first ADR for lifecycle enforcement posture)

---

## Notes

**ChatGPT Quote:**
> "If you encode those as acceptance checks across 15.3x–15.6x, the rest of Phase 15 will stay tight. Proceed to create the beads and start execution."

**PearlOwl Implementation Note:**
This ADR completes bd-13u (Track-0 Architecture). Upon completion, bd-7gl (Attempt Ledgers) becomes unblocked and available for implementation agents to claim. bd-7gl is **critical path** as it blocks 7 downstream beads across all tracks.
