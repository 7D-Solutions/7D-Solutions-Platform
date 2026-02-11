# AR Migration - Next Steps

**Status:** Testing and validation complete
**Current state:** 24% integration test pass rate (9/37 passing)
**Blockers:** Critical functional issues identified

---

## Recommended Child Beads

Based on validation testing, create the following child beads under **bd-zm6** to complete the migration:

### Critical Priority (P0)

#### 1. bd-zm6.20: Fix GET endpoint 404 issues
**Type:** bug
**Priority:** P0
**Estimate:** 4-6 hours

**Description:**
GET endpoints for individual resources return 404 when records exist.

**Affected endpoints:**
- `GET /api/ar/customers/{id}`
- `GET /api/ar/subscriptions/{id}`
- `GET /api/ar/charges/{id}`
- `GET /api/ar/refunds/{id}`
- `GET /api/ar/webhooks/{id}`
- `GET /api/ar/invoices/{id}`

**Root cause:**
- Path parameter extraction issues
- ID type mismatch (String vs i32)
- SQL query errors

**Acceptance criteria:**
- All GET-by-ID endpoints return 200 when record exists
- Customer, subscription, payment tests pass
- E2E workflows retrieve created records successfully

**Test verification:**
```bash
cargo test --test customer_tests::test_get_customer_success
cargo test --test subscription_tests::test_get_subscription
cargo test --test payment_tests::test_get_charge
```

---

#### 2. bd-zm6.21: Implement idempotency key handling
**Type:** feature
**Priority:** P0
**Estimate:** 6-8 hours

**Description:**
Idempotency keys are not being checked, allowing duplicate requests to create duplicate records.

**Requirements:**
1. Check `idempotency-key` header on POST requests
2. Store key + response in `billing_idempotency_keys` table
3. Return cached response for duplicate keys (200 instead of 201)
4. Handle key expiration (24 hours)
5. Handle concurrent requests with same key (lock or conflict)

**Acceptance criteria:**
- Duplicate POST with same idempotency key returns cached response
- Only one record created despite retry
- Different request with same key returns 409 conflict
- Expired keys allow new request
- All idempotency tests pass (0/3 → 3/3)

**Test verification:**
```bash
cargo test --test idempotency_test
cargo test --test e2e_workflows::test_error_recovery_workflow
```

---

#### 3. bd-zm6.22: Add webhook signature validation
**Type:** security
**Priority:** P0
**Estimate:** 4-6 hours

**Description:**
Tilled webhook endpoint accepts any payload without signature validation (security vulnerability).

**Requirements:**
1. Extract `Tilled-Signature` header
2. Compute HMAC SHA-256 of payload using webhook secret
3. Compare computed signature with provided signature
4. Reject webhooks with invalid/missing signature (401)
5. Log validation failures

**Environment:**
- `TILLED_WEBHOOK_SECRET` - webhook signing secret

**Acceptance criteria:**
- Valid signature → webhook processed
- Invalid signature → 401 Unauthorized
- Missing signature → 401 Unauthorized
- Webhook security tests pass

**Test verification:**
```bash
cargo test --test webhook_tests::test_reject_invalid_signature
cargo test --test webhook_tests::test_receive_valid_webhook
```

---

### High Priority (P1)

#### 4. bd-zm6.23: Fix query filtering issues
**Type:** bug
**Priority:** P1
**Estimate:** 4-6 hours

**Description:**
List endpoints with filters (by customer, by status, by external_id) return empty results.

**Affected endpoints:**
- `GET /api/ar/customers?external_customer_id=X`
- `GET /api/ar/subscriptions?customer_id=X`
- `GET /api/ar/subscriptions?status=active`
- `GET /api/ar/charges?customer_id=X`
- `GET /api/ar/refunds?charge_id=X`
- `GET /api/ar/webhooks?event_type=X`
- `GET /api/ar/webhooks?status=X`

**Root cause:**
- WHERE clause not constructed correctly
- Query parameters not extracted from request
- Column name mismatches

**Acceptance criteria:**
- All filtered queries return correct results
- Empty filters return all records (paginated)
- Multiple filters work together (AND logic)
- Filter tests pass in all test suites

**Test verification:**
```bash
cargo test --test customer_tests::test_list_customers_by_external_id
cargo test --test subscription_tests::test_list_by_customer
cargo test --test subscription_tests::test_list_by_status
```

---

#### 5. bd-zm6.24: Implement charge capture and refund operations
**Type:** feature
**Priority:** P1
**Estimate:** 6-8 hours

**Description:**
Payment operations (capture authorized charge, create refund) are not fully implemented.

**Requirements:**

**Charge Capture:**
1. Verify charge exists and status = "authorized"
2. Call Tilled API to capture charge
3. Update charge status to "succeeded"
4. Create event log entry
5. Handle capture amount (partial vs full)

**Refunds:**
1. Verify charge exists and is captured
2. Validate refund amount <= remaining balance
3. Call Tilled API to create refund
4. Create refund record
5. Update charge `amount_refunded_cents`
6. Create event log entry
7. Handle partial and full refunds

**Acceptance criteria:**
- Authorized charge can be captured
- Captured charge can be refunded (full or partial)
- Cannot refund more than charge amount
- Cannot capture twice
- All payment workflow tests pass
- E2E payment workflow completes

**Test verification:**
```bash
cargo test --test payment_tests::test_capture_charge
cargo test --test payment_tests::test_create_refund_full
cargo test --test payment_tests::test_create_refund_partial
cargo test --test e2e_workflows::test_payment_workflow
```

---

#### 6. bd-zm6.25: Standardize error responses and status codes
**Type:** bug
**Priority:** P1
**Estimate:** 2-4 hours

**Description:**
Error responses use inconsistent status codes (422 vs 400, etc.) breaking API contract.

**Issues:**
- Validation errors return 422 instead of 400
- Some errors missing error codes
- Error response format inconsistencies

**Requirements:**
1. Use 400 for validation errors (not 422)
2. Use 409 for conflict errors (duplicate email, etc.)
3. Use 404 for not found
4. Use 422 for unprocessable entity (business logic errors)
5. Consistent error response structure:
   ```json
   {
     "error": {
       "code": "validation_error",
       "message": "Email is required",
       "field": "email"  // optional
     }
   }
   ```

**Acceptance criteria:**
- All validation errors return 400
- Duplicate email returns 409 conflict
- Error format consistent across all endpoints
- Error status code tests pass

**Test verification:**
```bash
cargo test --test customer_tests::test_create_customer_missing_email
cargo test --test customer_tests::test_create_customer_duplicate_email
cargo test --test payment_tests -- --nocapture | grep "status code"
```

---

### Medium Priority (P2)

#### 7. bd-zm6.26: Implement subscription cancellation logic
**Type:** feature
**Priority:** P2
**Estimate:** 4-6 hours

**Description:**
Subscription cancellation endpoint exists but logic incomplete.

**Requirements:**
1. Handle `cancel_at_period_end=true` (soft cancel)
   - Set `cancel_at_period_end=true`
   - Keep status="active" until period end
   - Set `canceled_at` timestamp
2. Handle `cancel_at_period_end=false` (immediate cancel)
   - Set status="canceled"
   - Set `canceled_at` and `ended_at`
   - Call Tilled API to cancel
3. Create event log entry
4. Prevent canceling already-canceled subscriptions

**Acceptance criteria:**
- Soft cancel: subscription stays active until period end
- Immediate cancel: subscription ends immediately
- Cannot cancel twice
- Subscription cancellation tests pass

**Test verification:**
```bash
cargo test --test subscription_tests::test_cancel_at_period_end
cargo test --test subscription_tests::test_cancel_immediately
cargo test --test e2e_workflows::test_subscription_workflow
```

---

#### 8. bd-zm6.27: Implement invoice finalization logic
**Type:** feature
**Priority:** P2
**Estimate:** 4-6 hours

**Description:**
Invoice finalization endpoint exists but transitions not implemented.

**Requirements:**
1. Verify invoice status = "draft"
2. Validate invoice has line items and total > 0
3. Transition status: "draft" → "open"
4. Set `finalized_at` timestamp
5. Call Tilled API to finalize (if using Tilled invoices)
6. Create event log entry
7. Prevent finalizing already-finalized invoices

**Acceptance criteria:**
- Draft invoice can be finalized → "open"
- Cannot finalize twice
- Finalized invoice cannot be edited
- Invoice workflow test passes

**Test verification:**
```bash
cargo test --test e2e_workflows::test_invoice_workflow
```

---

#### 9. bd-zm6.28: Fix multi-tenant app_id filtering
**Type:** bug
**Priority:** P2
**Estimate:** 3-4 hours

**Description:**
app_id filtering not consistently applied, potential data leakage.

**Requirements:**
1. Extract `app_id` from auth middleware (currently hardcoded "default_app")
2. Add app_id to all WHERE clauses:
   - List customers
   - List subscriptions
   - List charges
   - List refunds
   - List invoices
   - All queries
3. Verify record belongs to app_id before update/delete
4. Add app_id to all INSERT statements

**Acceptance criteria:**
- Customers from app1 cannot see app2 data
- Update/delete operations verify app_id ownership
- All records created with correct app_id
- Multi-tenant test passes

**Test verification:**
```bash
cargo test --test e2e_workflows::test_multi_tenant_isolation
```

---

#### 10. bd-zm6.29: Implement webhook replay functionality
**Type:** feature
**Priority:** P2
**Estimate:** 3-4 hours

**Description:**
Webhook replay endpoint exists but not functional.

**Requirements:**
1. Verify webhook exists
2. Check current status (processed, failed, pending)
3. If `force=false`: only replay failed webhooks
4. If `force=true`: replay any webhook
5. Reprocess webhook event
6. Update webhook status and timestamps
7. Create new event log entry

**Acceptance criteria:**
- Failed webhook can be replayed
- Processed webhook cannot be replayed without force flag
- Force flag allows replaying any webhook
- Webhook replay tests pass

**Test verification:**
```bash
cargo test --test webhook_tests::test_replay_failed_webhook
cargo test --test webhook_tests::test_replay_processed_no_force
cargo test --test webhook_tests::test_replay_processed_with_force
```

---

## Testing and Validation Beads

#### 11. bd-zm6.30: Run load tests and validate performance
**Type:** task
**Priority:** P2
**Estimate:** 2-3 hours
**Dependencies:** bd-zm6.20-29 (all functional issues fixed)

**Description:**
Execute load tests and verify performance meets targets.

**Requirements:**
1. Install Artillery: `npm install -g artillery`
2. Run load test: `artillery run tests/load/ar-load-test.yml`
3. Verify meets targets:
   - Error rate < 1%
   - p95 latency < 500ms
   - p99 latency < 1000ms
4. Generate performance report
5. Compare Rust vs Node.js performance

**Deliverables:**
- Load test results report
- Performance comparison data
- Recommendations for optimization (if needed)

---

#### 12. bd-zm6.31: Run comparison tests (Rust vs Node.js)
**Type:** task
**Priority:** P2
**Estimate:** 2-3 hours
**Dependencies:** bd-zm6.20-29 (all functional issues fixed)

**Description:**
Run comparison script to verify Rust implementation matches Node.js behavior.

**Requirements:**
1. Start Node.js AR service (if still available)
2. Run: `./tests/compare-implementations.sh`
3. Verify:
   - Response structures match
   - Status codes match
   - Data format consistent
4. Measure performance differences
5. Generate comparison report

**Deliverables:**
- Comparison test report
- Response parity verification
- Performance improvement metrics

---

#### 13. bd-zm6.32: Execute data migration and validation
**Type:** task
**Priority:** P1
**Estimate:** 3-4 hours
**Dependencies:** bd-zm6.20-29 (all functional issues fixed)

**Description:**
Migrate production data from MySQL to PostgreSQL and validate integrity.

**Requirements:**
1. Create data migration script (or use existing)
2. Run migration in staging environment
3. Run validation: `./tests/validate-data-migration.sh`
4. Verify:
   - 100% record count match
   - Data integrity (checksums)
   - Foreign key relationships intact
   - No orphaned records
5. Generate validation report
6. Create rollback plan

**Deliverables:**
- Data migration script
- Data validation report (100% pass rate)
- Rollback procedure document

---

## Execution Strategy

### Phase 1: Critical Fixes (1 week)
**Goal:** Resolve P0 blockers

1. **Day 1-2:** bd-zm6.20 (GET endpoints)
2. **Day 3-4:** bd-zm6.21 (Idempotency)
3. **Day 5:** bd-zm6.22 (Webhook security)

**Milestone:** Integration test pass rate: 24% → 60%

---

### Phase 2: Core Functionality (1 week)
**Goal:** Complete payment and query functionality

4. **Day 1-2:** bd-zm6.23 (Query filtering)
5. **Day 3-4:** bd-zm6.24 (Capture/refunds)
6. **Day 5:** bd-zm6.25 (Error standardization)

**Milestone:** Integration test pass rate: 60% → 85%

---

### Phase 3: Feature Completion (3-4 days)
**Goal:** Complete remaining features

7. **Day 1:** bd-zm6.26 (Subscription cancellation)
8. **Day 2:** bd-zm6.27 (Invoice finalization)
9. **Day 3:** bd-zm6.28 (Multi-tenant filtering)
10. **Day 4:** bd-zm6.29 (Webhook replay)

**Milestone:** Integration test pass rate: 85% → 100%

---

### Phase 4: Testing and Validation (3-4 days)
**Goal:** Verify production readiness

11. **Day 1:** bd-zm6.30 (Load tests)
12. **Day 2:** bd-zm6.31 (Comparison tests)
13. **Day 3:** bd-zm6.32 (Data migration)
14. **Day 4:** Final validation and documentation

**Milestone:** Production ready, all tests passing

---

## Success Criteria

### Before Production Deployment

- [ ] Integration test pass rate: 100% (37/37 passing)
- [ ] E2E workflow tests: 100% (7/7 passing)
- [ ] Idempotency tests: 100% (3/3 passing)
- [ ] Load test: < 1% error rate, p95 < 500ms
- [ ] Comparison test: Response parity verified
- [ ] Data validation: 100% integrity verified
- [ ] Security: Webhook signature validation active
- [ ] Documentation: Migration guide complete

### Post-Deployment Monitoring

- Error rate < 0.1%
- p95 latency < 200ms
- Zero data inconsistencies
- Webhook processing success rate > 99%
- No rollbacks required

---

## Resources

### Documentation
- Migration validation report: `docs/ar-migration-validation-report.md`
- Test suite: `tests/run-validation-suite.sh`
- Load test config: `tests/load/ar-load-test.yml`
- Comparison script: `tests/compare-implementations.sh`
- Data validation: `tests/validate-data-migration.sh`

### Test Commands
```bash
# Run all validation tests
./tests/run-validation-suite.sh

# Run specific test suite
cargo test --test customer_tests
cargo test --test subscription_tests
cargo test --test payment_tests
cargo test --test webhook_tests
cargo test --test idempotency_test
cargo test --test e2e_workflows

# Run with output
cargo test --test customer_tests -- --nocapture

# Run single test
cargo test --test customer_tests::test_get_customer_success -- --exact

# Load tests (after fixes)
artillery run tests/load/ar-load-test.yml

# Comparison tests (after fixes)
./tests/compare-implementations.sh

# Data validation (after migration)
./tests/validate-data-migration.sh
```

---

**Total Estimated Effort:** 40-55 hours (2-3 weeks)
**Critical Path:** bd-zm6.20 → bd-zm6.21 → bd-zm6.22 → bd-zm6.32 (data migration)
**Current Blocker:** bd-zm6.20 (GET endpoint 404s) prevents all E2E workflows

**Recommendation:** Start with bd-zm6.20 immediately as it unblocks most other work.
