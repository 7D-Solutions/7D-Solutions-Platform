# Test Failures Analysis & Fixes

## Summary

**Initial State:** 74 tests failing (151 passing, 225 total)
**Final State:** 0 tests failing (225 passing, 225 total)

## Root Causes Identified

### 1. Test Isolation: Parallel Execution Race Conditions (63 FK violations)

**Problem:**
- Jest ran 4 integration test files in parallel, sharing a single test database
- Race conditions caused `billing_customer_id` foreign key constraint violations
- Tests tried to create child records (subscriptions, charges) before parent customers existed

**Evidence:**
- Running with `--runInBand` reduced failures from 74 → 11
- Each integration test passed when run individually
- FK violations only occurred when running full suite

**Fix:**
- Added `maxWorkers: 1` to integration project in `jest.config.js`
- Updated `package.json` test script to run unit and integration tests sequentially
- Prevents mock interference between unit and integration test projects

### 2. Missing app_id Parameter (11 failures)

**Problem:**
- `routes.test.js` subscription tests missing `app_id` query parameter
- `requireAppId()` middleware expects `app_id` in params/body/query (middleware.js:16)
- Tests returned 400 "Missing app_id" instead of expected status codes

**Affected Tests:**
- POST /api/billing/subscriptions (6 calls in beforeEach blocks + 1 test)
- POST /api/billing/customers/:id/default-payment-method (1 test)

**Fix:**
- Added `?app_id=trashtech` query parameter to all affected requests
- Pattern: `.post('/api/billing/subscriptions?app_id=trashtech')`

### 3. Webhook Null Check (1 failure)

**Problem:**
- `billingService.real.test.js:298` - test expected webhook record with status 'failed'
- Test passed when run individually, failed in full suite
- Database cleanup race condition in parallel execution

**Fix:**
- Resolved by test isolation fix (#1)
- Webhook failure path already correctly creates and updates webhook records

## Fixes Applied

### Test Standardization

**Files Modified:**
1. `tests/integration/phase1-routes.test.js`
   - Added `setupIntegrationTests()` to `beforeAll`
   - Added `teardownIntegrationTests()` to `afterAll`
   - Already used `cleanDatabase()` from database-cleanup.js

2. `tests/integration/refunds.routes.test.js`
   - Added `setupIntegrationTests()` to `beforeAll`
   - Added `teardownIntegrationTests()` to `afterAll`
   - Already used `cleanDatabase()` from database-cleanup.js

**Result:** All 4 integration test files now use standardized cleanup strategy

### Configuration Changes

1. **jest.config.js**
   ```javascript
   {
     displayName: 'integration',
     maxWorkers: 1  // Run integration tests serially
   }
   ```

2. **package.json**
   ```json
   {
     "test": "npm run test:unit && npm run test:integration"
   }
   ```
   - Runs unit and integration tests sequentially in separate processes
   - Prevents mock interference from unit tests affecting integration tests

### Test Fixes

1. **tests/integration/routes.test.js**
   - Line 168: Added `?app_id=trashtech` to default payment method endpoint
   - Lines 201, 229, 257, 529, 580, 644: Added `?app_id=trashtech` to subscription creation (applied by WildRaven)

## Verification

```bash
npm run test:unit       # ✓ 9 suites, 138 tests passing
npm run test:integration # ✓ 4 suites, 87 tests passing
npm test                # ✓ 13 suites, 225 tests passing
```

## Lessons Learned

1. **Test Isolation is Critical:** Integration tests sharing a database must run serially
2. **Mock Interference:** Unit test mocks can affect integration tests when run in same process
3. **Middleware Requirements:** All API routes with `requireAppId()` need `app_id` in requests
4. **Cleanup Consistency:** Standardized setup/teardown prevents subtle race conditions

## Architecture Notes

- Billing module uses separate database (`DATABASE_URL_BILLING`)
- Integration tests verify test DB name contains `_test` before running
- Database cleanup runs in dependency order: children → parents
- Webhook processing uses idempotency via unique `event_id` constraint
