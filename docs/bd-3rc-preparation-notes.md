# bd-3rc Preparation: Cross-Module Invariant Suite

**Bead:** bd-3rc (bd-15.7b) - Cross-Module Invariant Suite (E2E Assertions Across Lifecycle)
**Prepared By:** EmeraldBear
**Date:** 2026-02-15
**Dependencies:** bd-35x (Module-Level Invariant Primitives) - IN_PROGRESS by FuchsiaGrove

---

## Phase 15 Invariants (from ADR-015 + Coordination Rules)

### 1. Mutation Ownership Rule (HARD)
- ❌ **FORBIDDEN:** Direct SQL UPDATE of lifecycle status columns outside lifecycle modules
- ✅ **REQUIRED:** All lifecycle status writes MUST go through `*::lifecycle::*` functions

### 2. Exactly-Once Rule (HARD)
- Side effects (PSP call, finalize emit, notification emit, ledger post) ONLY occur when:
  - Attempt row is newly created, OR
  - Attempt transitions via guard from retry-eligible state
- Duplicate triggers MUST be deterministic no-ops

### 3. State Machine Invariants

**Subscription (subscriptions::lifecycle):**
- States: ACTIVE, PAST_DUE, SUSPENDED
- Exactly one invoice per subscription-cycle (enforced by bd-184)
- Suspension only when allowed (not while UNKNOWN payment exists)

**Invoice (ar::lifecycle):**
- States: OPEN, ATTEMPTING, PAID, FAILED_FINAL
- No duplicate invoice attempts (UNIQUE constraints)
- Exactly one finalization side effect per attempt

**Payment Attempt (payments::lifecycle):**
- States: ATTEMPTING, SUCCEEDED, FAILED_RETRY, FAILED_FINAL, UNKNOWN
- No duplicate payment attempts (UNIQUE constraints)
- UNKNOWN blocks retries and subscription suspension
- Exactly one attempt per retry window [0d, +3d, +7d]

---

## Cross-Module Flows to Test

### Flow 1: Subscription → AR (Invoice Generation)
**Invariant:** Exactly one invoice per subscription-cycle

**Test Scenarios:**
1. **Happy path:** Subscription generates invoice successfully
2. **Duplicate trigger:** Second trigger for same cycle → no-op (no second invoice)
3. **Concurrency:** Parallel triggers for same cycle → exactly one invoice created
4. **State transition:** Invoice creation only happens from valid subscription states

### Flow 2: AR → Payments (Payment Collection)
**Invariant:** No duplicate payment attempts

**Test Scenarios:**
1. **Happy path:** Invoice triggers payment collection successfully
2. **Duplicate trigger:** Second collection request → no-op (no second attempt)
3. **Attempt ledger enforcement:** UNIQUE constraint prevents duplicate (app_id, payment_id, attempt_no)
4. **Retry windows:** Attempts only created at 0d, +3d, +7d windows

### Flow 3: AR → GL (Posting Requests)
**Invariant:** No duplicate ledger postings, GL balanced

**Test Scenarios:**
1. **Happy path:** AR invoice generates GL posting request
2. **Idempotency:** Replay of same event → no duplicate journal entry (source_event_id deduplication)
3. **GL balance:** All journal entries balanced (debit = credit)
4. **Account validation:** Posting only succeeds if accounts exist and are active
5. **Period validation:** Posting rejected if period is closed

### Flow 4: Payments → AR (Payment Status Updates)
**Invariant:** Payment status transitions trigger AR invoice status updates

**Test Scenarios:**
1. **Happy path:** Payment SUCCEEDED → Invoice PAID
2. **Webhook idempotency:** Duplicate webhook → no-op (webhook_event_id deduplication)
3. **State machine:** Payment SUCCEEDED only from ATTEMPTING (lifecycle guard validation)
4. **UNKNOWN protocol:** Payment UNKNOWN blocks invoice finalization

### Flow 5: Payments UNKNOWN → Reconciliation
**Invariant:** UNKNOWN blocks retries and subscription suspension

**Test Scenarios:**
1. **Retry block:** Payment UNKNOWN → retry scheduler skips
2. **Suspension block:** Payment UNKNOWN → subscription NOT suspended
3. **Reconciliation:** UNKNOWN → SUCCEEDED/FAILED_* via reconciliation workflow
4. **Determinism:** Same webhook data → same reconciliation outcome

---

## Test Architecture

### Test Structure
Following Phase 11/12/13 boundary E2E pattern:

```
tests/
  cross_module_e2e_subscription_to_invoice.rs
  cross_module_e2e_invoice_to_payment.rs
  cross_module_e2e_invoice_to_gl.rs
  cross_module_e2e_payment_to_invoice.rs
  cross_module_e2e_payment_unknown_protocol.rs
  cross_module_e2e_replay_safety.rs
  cross_module_e2e_concurrency_safety.rs
  common/
    mod.rs (shared test utilities)
```

### Test Infrastructure Requirements

**Database Pools:**
- GL database (port 5438)
- AR database (port 5436)
- Payments database (port 5437)
- Subscriptions database (port 5435)
- Auth database (port 5434)

**Event Bus:**
- NATS (port 4222)
- Subject prefixes:
  - `subscriptions.events.*`
  - `ar.events.*`
  - `payments.events.*`
  - `gl.events.*`

**Test Patterns:**
- **Serial execution:** `#[serial]` to prevent test interference
- **Polling pattern:** Wait for async event processing (e.g., `poll_for_invoice()`)
- **Cleanup:** Teardown test data after each test
- **Isolation:** Unique tenant_id per test to prevent cross-contamination

### Common Test Utilities (from GL boundary E2E)

```rust
// Get singleton DB pool
async fn get_test_pool(db_name: &str) -> PgPool;

// Setup NATS event bus
async fn setup_nats_bus() -> Arc<dyn EventBus>;

// Publish event and wait for processing
async fn publish_and_wait<T>(
    bus: &Arc<dyn EventBus>,
    subject: &str,
    event: &EventEnvelope<T>,
    duration: Duration,
) -> Result<(), Error>;

// Poll for database record
async fn poll_for_record<T>(
    pool: &PgPool,
    query: &str,
    bind_params: &[&dyn sqlx::Encode<'_, sqlx::Postgres>],
    max_attempts: usize,
    delay: Duration,
) -> Option<T>;

// Assert no duplicate records
async fn assert_unique<T>(
    pool: &PgPool,
    query: &str,
    bind_params: &[&dyn sqlx::Encode<'_, sqlx::Postgres>],
);
```

---

## Test Scenario Details

### Test 1: Subscription → Invoice (Exactly One Per Cycle)

**Setup:**
- Create subscription in ACTIVE state
- Define billing cycle (e.g., 2026-02-01 to 2026-02-28)

**Execute:**
1. Trigger invoice generation event (subscriptions.events.billing_cycle_started)
2. Poll AR database for invoice creation
3. Trigger duplicate event (same cycle)
4. Wait and assert no second invoice

**Assertions:**
- Exactly one invoice created for cycle
- Invoice status = OPEN
- UNIQUE constraint enforced: (subscription_id, cycle_start, cycle_end)

**Variant (Concurrency):**
- Publish 10 parallel events for same cycle
- Assert exactly one invoice created
- Assert other 9 events result in UNIQUE violation (deterministic no-op)

---

### Test 2: Invoice → Payment (No Duplicate Attempts)

**Setup:**
- Create invoice in OPEN state
- Create customer payment method

**Execute:**
1. Trigger payment collection event (ar.payment.collection.requested)
2. Poll Payments database for attempt creation
3. Trigger duplicate event (same invoice)
4. Wait and assert no second attempt

**Assertions:**
- Exactly one payment attempt created (attempt_no = 0)
- Attempt status = ATTEMPTING
- UNIQUE constraint enforced: (app_id, payment_id, attempt_no)
- Idempotency key matches: `payment:attempt:{app_id}:{payment_id}:0`

**Variant (Retry Windows):**
- Create attempts at 0d, +3d, +7d
- Assert each window creates exactly one attempt
- Assert attempt_no increments: 0, 1, 2

---

### Test 3: Invoice → GL (No Duplicate Postings)

**Setup:**
- Create invoice with line items
- Create accounts in Chart of Accounts (AR, Revenue, Tax)
- Create open accounting period

**Execute:**
1. Publish gl.events.posting.requested
2. Poll GL database for journal entry creation
3. Replay same event (duplicate event_id)
4. Assert no second journal entry

**Assertions:**
- Exactly one journal entry created
- Journal entry balanced: SUM(debits) = SUM(credits)
- Idempotency: source_event_id prevents duplicate
- Account validation: All accounts exist and are active
- Period validation: Period is open (not closed)

**Variant (Account Inactive):**
- Mark one account as inactive
- Publish posting request
- Assert posting rejected (account validation failure)
- Assert event sent to DLQ

---

### Test 4: Payment → Invoice (Status Updates)

**Setup:**
- Create invoice in ATTEMPTING state
- Create payment attempt in ATTEMPTING state

**Execute:**
1. Publish payments.payment.succeeded event
2. Poll Payments database for attempt status = SUCCEEDED
3. Poll AR database for invoice status = PAID
4. Publish duplicate success webhook
5. Assert no second status update

**Assertions:**
- Payment attempt status = SUCCEEDED
- Invoice status = PAID
- Webhook idempotency: webhook_event_id prevents duplicate
- Lifecycle guard: ATTEMPTING → SUCCEEDED is valid transition
- GL posting triggered (invoice payment recorded)

**Variant (Invalid Transition):**
- Create payment attempt in SUCCEEDED state
- Publish payments.payment.failed event
- Assert lifecycle guard rejects transition (SUCCEEDED → FAILED_FINAL illegal)
- Assert payment status unchanged

---

### Test 5: Payment UNKNOWN → Retry Block

**Setup:**
- Create subscription in PAST_DUE state
- Create invoice in ATTEMPTING state
- Create payment attempt in UNKNOWN state

**Execute:**
1. Trigger retry scheduler (simulated time advance to +3d)
2. Poll AR database for invoice attempts
3. Assert no new invoice attempt created (UNKNOWN blocks retry)
4. Trigger reconciliation (payment resolved to SUCCEEDED)
5. Trigger retry scheduler again
6. Assert invoice status updated to PAID

**Assertions:**
- UNKNOWN blocks retry: No new attempt created at +3d window
- UNKNOWN blocks suspension: Subscription NOT suspended
- Reconciliation resolves UNKNOWN → SUCCEEDED
- After reconciliation, retry window proceeds normally

**Variant (Reconciliation to FAILED):**
- Reconcile UNKNOWN → FAILED_FINAL
- Assert retry window opens at +3d (failed state allows retry)
- Assert new attempt created (attempt_no = 1)

---

### Test 6: Replay Safety (Full Flow)

**Setup:**
- Create full billing flow: Subscription → Invoice → Payment → GL

**Execute:**
1. Publish subscription billing cycle event
2. Wait for invoice creation
3. Publish payment collection event
4. Wait for payment attempt creation
5. Publish payment succeeded webhook
6. Wait for GL posting
7. **Replay all events** (same event_id)
8. Wait and poll all databases

**Assertions:**
- Exactly one invoice created (subscription-cycle UNIQUE)
- Exactly one payment attempt created (payment-attempt UNIQUE)
- Exactly one GL journal entry created (source_event_id deduplication)
- All status transitions occurred exactly once
- No side effects duplicated (no double PSP calls, no double GL posts)

---

### Test 7: Concurrency Safety (Parallel Operations)

**Setup:**
- Create 10 subscriptions with same billing cycle date
- Create 10 invoices with payment collection triggers

**Execute:**
1. Publish 10 parallel subscription billing events (same tenant, same cycle)
2. Publish 10 parallel payment collection events (same invoice)
3. Publish 10 parallel GL posting events (same source_event_id)
4. Wait for all processing to complete
5. Poll databases and count records

**Assertions:**
- Each subscription → exactly one invoice (10 invoices total)
- Each invoice → exactly one payment attempt (10 attempts total)
- Each posting → exactly one journal entry (10 entries total)
- UNIQUE constraints enforced under concurrency
- SELECT FOR UPDATE prevents race conditions

---

## Performance Guardrails

**Per ChatGPT guidance from Phase 11/12:**
- E2E tests should complete in < 5 seconds each
- Polling max attempts: 10 (with 200ms delay = 2s max wait)
- No table scans (use indexed queries for polling)
- Serial execution to prevent DB connection pool exhaustion

---

## Implementation Strategy

### Step 1: Review bd-35x Output (blocked on FuchsiaGrove)
- Understand module-level invariant primitives
- Identify reusable helpers for cross-module tests
- Check for any new test utilities added

### Step 2: Create Test Infrastructure
- Common module with DB pools (singleton pattern from Phase 12)
- NATS bus setup helpers
- Polling utilities (from GL boundary E2E pattern)
- Event publishing helpers

### Step 3: Implement Test Suites (7 files)
- Subscription → Invoice E2E
- Invoice → Payment E2E
- Invoice → GL E2E
- Payment → Invoice E2E
- Payment UNKNOWN protocol E2E
- Replay safety E2E
- Concurrency safety E2E

### Step 4: CI Integration
- Add cross_module_e2e test job to CI workflow
- Require all tests pass before merge
- Document test failure modes

### Step 5: Documentation
- Update TESTING.md with cross-module E2E patterns
- Document invariant verification checklist
- Add troubleshooting guide for test failures

---

## Success Criteria (from bd-3rc description)

- [ ] Exactly one invoice per cycle enforced (subscriptions → AR)
- [ ] No duplicate attempts (AR → Payments, attempt ledger UNIQUE)
- [ ] No duplicate ledger postings (AR → GL, source_event_id deduplication)
- [ ] GL balanced (all journal entries: debit = credit)
- [ ] Suspension only when allowed (UNKNOWN blocks suspension)
- [ ] Notifications exactly once (event deduplication)
- [ ] Invalid webhooks mutate nothing (signature validation + lifecycle guards)
- [ ] Replay safety (same event → same outcome, no duplicate side effects)
- [ ] Concurrency safety (parallel triggers → deterministic state)

---

## References

- **Phase 15 Coordination Rules:** docs/PHASE-15-COORDINATION-RULES.md
- **ADR-015:** docs/architecture/decisions/ADR-015-phase15-lifecycle-enforcement-posture.md
- **GL Boundary E2E Pattern:** modules/gl/tests/boundary_e2e_nats_posting.rs
- **Phase 11/12/13 E2E Tests:** modules/gl/tests/boundary_e2e_*.rs
- **bd-35x (Dependency):** Module-Level Invariant Primitives (FuchsiaGrove)

---

## Notes

**Key Learning from bd-1p2 Preparation:**
- Thorough preparation → zero ramp-up time execution
- 289-line prep doc for bd-1p2 enabled 5-minute implementation
- Same approach here: comprehensive scenario design before coding

**ChatGPT Approval Confidence:**
- Follow Phase 11/12 boundary E2E patterns (proven successful)
- Assert Phase 15 invariants explicitly (mutation ownership, exactly-once)
- Replay + concurrency tests are non-negotiable (deterministic execution layer)
- Performance guardrails: < 5s per test, no table scans

**Parallelization Opportunity:**
- Can implement 7 test files independently
- Each test suite focuses on one cross-module flow
- Serial execution flag prevents test interference
