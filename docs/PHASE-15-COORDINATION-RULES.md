# Phase 15: Coordination Rules (Hard Requirements)

**Source:** ChatGPT Final Approval (2026-02-15)
**Architecture:** Option B-lite (lifecycle modules per service)

---

## Critical Rules

These rules MUST be enforced across all Phase 15 implementation beads. Violations are treated as failing defects.

### 1. Mutation Ownership Rule (HARD)

**After bd-15.3x lands:**
- ❌ **FORBIDDEN:** No route/handler may update lifecycle status columns directly
- ✅ **REQUIRED:** All lifecycle status writes MUST go through `*_::lifecycle::*` functions (guards inside)
- **Enforcement:** Treat direct SQL updates as a failing defect

**Affected Modules:**
- `subscriptions::lifecycle` (after bd-138 / bd-15.3a)
- `ar::lifecycle` (after bd-1w7 / bd-15.3b)
- `payments::lifecycle` (after bd-3lm / bd-15.3c)

**Examples:**
```rust
// ❌ FORBIDDEN (after bd-15.3x)
sqlx::query("UPDATE subscriptions SET status = 'SUSPENDED' WHERE id = $1")
  .execute(&pool).await?;

// ✅ REQUIRED
subscriptions::lifecycle::transition_to_suspended(sub_id, reason, &pool).await?;
```

---

### 2. Exactly-Once Rule (HARD)

**Side effects** (PSP call, finalize emit, notification emit, ledger post) **may only occur** when:
- The attempt row is **newly created**, OR
- The attempt transitions via guard from a **retry-eligible state**

**Duplicate triggers MUST be deterministic no-ops.**

**Implementation Requirements:**
- Use attempt ledger UNIQUE constraints (bd-7gl / bd-15.1)
- Use SELECT FOR UPDATE for aggregate locks (bd-15.4x)
- Use idempotency keys (bd-1p2 / bd-15.2)
- Reject duplicate attempts at DB level (UNIQUE violation → deterministic no-op)

**Examples:**
```rust
// ✅ Correct: Insert attempt row first, reject duplicates
match insert_attempt_row(&tx, invoice_id, attempt_no).await {
  Ok(_) => {
    // Newly created → proceed with side effects
    send_payment_request(psp_client, invoice).await?;
    emit_finalize_event(outbox, invoice).await?;
  }
  Err(UniqueViolation) => {
    // Duplicate trigger → deterministic no-op
    return Ok(AlreadyProcessed);
  }
}
```

---

## Acceptance Checks

**Encode these rules as acceptance checks across bd-15.3x through bd-15.6x:**

### For Each Lifecycle Bead (15.3a/b/c):
- [ ] All status mutations route through `lifecycle::*` functions
- [ ] No direct SQL UPDATE of status columns outside lifecycle module
- [ ] Transition guards reject illegal transitions with zero side effects
- [ ] Tests assert route cannot mutate status without calling lifecycle API

### For Each Gating Bead (15.4a/b/c):
- [ ] Attempt row insertion uses UNIQUE constraints
- [ ] Duplicate attempt insertion → deterministic no-op or failure
- [ ] Side effects only occur on successful attempt row creation
- [ ] Concurrency tests prove exactly-once behavior

### For Retry Beads (15.6a/b):
- [ ] Exactly one attempt per window enforced by UNIQUE (tenant_id, entity_id, attempt_no)
- [ ] Duplicate triggers within window → no second attempt created
- [ ] Side effects only on new attempt creation

### For Invariant Beads (15.7a/b):
- [ ] Tests assert: mutation ownership rule holds
- [ ] Tests assert: exactly-once behavior under replay
- [ ] Tests assert: duplicate triggers → deterministic no-ops

---

## Phase 15 Bead Mapping

| Logical ID | Actual ID | Title |
|------------|-----------|-------|
| bd-15.0 | bd-13u | ADR: Phase 15 Enforcement Posture (Option B-lite Locked) |
| bd-15.1 | bd-7gl | Attempt Ledgers + DB Uniqueness (AR + Payments) |
| bd-15.2 | bd-1p2 | Deterministic Idempotency Key Spec (Cross-Module Contract) |
| bd-15.3a | bd-138 | Subscriptions Transition Guards (ACTIVE/PAST_DUE/SUSPENDED) |
| bd-15.3b | bd-1w7 | AR Invoice Transition Guards (OPEN/ATTEMPTING/PAID/FAILED_FINAL) |
| bd-15.3c | bd-3lm | Payments Attempt Transition Guards (ATTEMPTING/SUCCEEDED/FAILED_*/UNKNOWN) |
| bd-15.4a | bd-184 | Subscriptions Cycle Gating (Exactly One Invoice Per Cycle) |
| bd-15.4b | bd-3fo | AR Finalization Gating (FOR UPDATE + Attempt Grain) |
| bd-15.4c | bd-1wg | Payments Gating + Webhook Mutation Order (Signature Before Write) |
| bd-15.5 | bd-2uw | UNKNOWN Protocol + Deterministic Reconciliation (Payments) |
| bd-15.6a | bd-8ev | Retry Window Discipline (AR): Attempt 0 / +3d / +7d |
| bd-15.6b | bd-1it | Retry Window Discipline (Payments): Attempt 0 / +3d / +7d with UNKNOWN Block |
| bd-15.7a | bd-35x | Module-Level Invariant Primitives (Unit/Integration per Module) |
| bd-15.7b | bd-3rc | Cross-Module Invariant Suite (E2E Assertions Across Lifecycle) |
| bd-15.8 | bd-3c2 | Deterministic Simulation Harness (6 Cycles, 10–20 Tenants, Replay + Concurrency + UNKNOWN) |

---

## Exit Criteria

Phase 15 is complete when:
- [ ] All 15 beads closed
- [ ] All state machines enforce legal transitions only
- [ ] Exactly-once guarantees proven via tests
- [ ] UNKNOWN protocol fully implemented
- [ ] Retry windows deterministic
- [ ] Invariant suite passes
- [ ] Simulation harness passes 3+ consecutive runs with identical results
- [ ] Both coordination rules enforced and tested

---

**ChatGPT Quote:**
> "If you encode those as acceptance checks across 15.3x–15.6x, the rest of Phase 15 will stay tight. Proceed to create the beads and start execution."
