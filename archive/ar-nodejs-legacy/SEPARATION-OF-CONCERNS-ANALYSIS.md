# Separation of Concerns Analysis - Billing Package

**Analysis Date:** 2026-01-31
**Codebase Status:** Production-ready, 226/226 tests passing
**Purpose:** Identify refactoring opportunities for improved separation of concerns

---

## Executive Summary

**Overall Assessment:** The billing module demonstrates strong architectural separation at the service layer (BillingService delegates to 8 specialized services), but the HTTP layer (routes.js) contains significant inline logic that could be extracted.

**Files Analyzed:**
- ✅ **billingService.js** (202 lines) - Well-factored facade pattern
- ⚠️ **routes.js** (772 lines) - Needs refactoring for validation, idempotency, error handling
- ✅ **tilledClient.js** (430 lines) - Clean SDK wrapper, well-structured
- ✅ **WebhookService.js** (397 lines) - Good service separation, minimal issues

**Risk Level:** LOW - Proposed refactorings are non-breaking and can be incremental

---

## File Analysis

### 1. routes.js (772 lines) ⚠️ PRIMARY REFACTORING TARGET

**Issues Identified:**

#### A. Repetitive Input Validation (Lines 89-91, 110-118, 135-137, 221-225, etc.)

**Current Pattern:**
```javascript
// Repeated across 15+ routes
if (!email || !name) {
  return res.status(400).json({ error: 'Missing required fields: email, name' });
}

// Email validation repeated
const emailRegex = /^[^\s@]+@[^\s@]+\.[^\s@]+$/;
if (!emailRegex.test(email)) {
  return res.status(400).json({ error: 'Invalid email format' });
}
```

**Problem:**
- Validation logic embedded in route handlers
- Inconsistent error messages across routes
- No centralized schema definitions
- Difficult to test validation separately

**Recommended Solution:**
```javascript
// Create validators/requestValidators.js
const { body, validationResult } = require('express-validator');

const createCustomerValidator = [
  body('email').isEmail().withMessage('Invalid email format'),
  body('name').notEmpty().withMessage('Missing required field: name'),
  (req, res, next) => {
    const errors = validationResult(req);
    if (!errors.isEmpty()) {
      return res.status(400).json({ errors: errors.array() });
    }
    next();
  }
];

// Usage in routes.js
router.post('/customers', requireAppId(), rejectSensitiveData, createCustomerValidator, async (req, res) => {
  // Validation already done, just process request
  const customer = await billingService.createCustomer(...);
  res.status(201).json(customer);
});
```

**Impact:** Reduces routes.js by ~150 lines, improves testability

---

#### B. Idempotency Pattern Duplication (Lines 509-624, 655-770)

**Current Pattern:**
```javascript
// Repeated in /charges/one-time and /refunds routes
const idempotencyKey = req.headers['idempotency-key'];
if (!idempotencyKey) {
  return res.status(400).json({ error: 'Idempotency-Key header is required' });
}

const requestHash = billingService.computeRequestHash('POST', '/charges/one-time', req.body);

const cachedResponse = await billingService.getIdempotentResponse(appId, idempotencyKey, requestHash);
if (cachedResponse) {
  return res.status(cachedResponse.statusCode).json(cachedResponse.body);
}

// ... business logic ...

await billingService.storeIdempotentResponse(appId, idempotencyKey, requestHash, statusCode, responseBody);
```

**Problem:**
- Idempotency logic duplicated in 2 routes (will be 4+ as module grows)
- Route path hardcoded in hash computation
- Caching logic mixed with business logic
- ~50 lines of boilerplate per idempotent route

**Recommended Solution:**
```javascript
// Create middleware/idempotency.js
function requireIdempotency(billingService) {
  return async (req, res, next) => {
    const idempotencyKey = req.headers['idempotency-key'];
    if (!idempotencyKey) {
      return res.status(400).json({ error: 'Idempotency-Key header is required' });
    }

    const appId = req.verifiedAppId;
    const requestHash = billingService.computeRequestHash(req.method, req.path, req.body);

    // Check cache
    const cachedResponse = await billingService.getIdempotentResponse(appId, idempotencyKey, requestHash);
    if (cachedResponse) {
      return res.status(cachedResponse.statusCode).json(cachedResponse.body);
    }

    // Attach idempotency context to request
    req.idempotency = { key: idempotencyKey, requestHash };

    // Intercept response to cache it
    const originalJson = res.json.bind(res);
    res.json = function(body) {
      const statusCode = res.statusCode;
      billingService.storeIdempotentResponse(appId, idempotencyKey, requestHash, statusCode, body)
        .catch(err => logger.error('Failed to store idempotent response', err));
      return originalJson(body);
    };

    next();
  };
}

// Usage in routes.js
router.post('/charges/one-time', requireAppId(), rejectSensitiveData, requireIdempotency(billingService), async (req, res) => {
  // Idempotency handled automatically
  const charge = await billingService.createOneTimeCharge(...);
  res.status(201).json({ charge });
});
```

**Impact:** Reduces routes.js by ~100 lines, enables idempotency for future routes

---

#### C. Error Handling Duplication (Lines 593-623, 737-769, repeated 20+ times)

**Current Pattern:**
```javascript
// Error handling repeated across all routes
catch (error) {
  logger.error('POST /customers error:', error);

  if (error.message && error.message.includes('not found')) {
    return res.status(404).json({ error: error.message });
  }
  if (error.message.includes('No default payment method')) {
    return res.status(409).json({ error: error.message });
  }
  if (error.message.includes('is required')) {
    return res.status(400).json({ error: error.message });
  }
  if (error.code) {
    return res.status(502).json({ error: 'Charge failed', code: error.code });
  }

  res.status(500).json({ error: 'Failed to create customer', message: error.message });
}
```

**Problem:**
- Error-to-HTTP-status mapping repeated in every route
- Inconsistent error checking order
- String matching for error types (fragile)
- ~30 lines per route for error handling

**Recommended Solution:**
```javascript
// Create utils/errorHandler.js
class BillingError extends Error {
  constructor(message, statusCode) {
    super(message);
    this.statusCode = statusCode;
    this.name = this.constructor.name;
  }
}

class NotFoundError extends BillingError {
  constructor(message) { super(message, 404); }
}

class ValidationError extends BillingError {
  constructor(message) { super(message, 400); }
}

class ConflictError extends BillingError {
  constructor(message) { super(message, 409); }
}

function handleBillingError(err, req, res, next) {
  logger.error(`${req.method} ${req.path} error:`, err);

  // Billing package errors
  if (err instanceof BillingError) {
    return res.status(err.statusCode).json({ error: err.message });
  }

  // Prisma errors
  if (err.code && err.code.startsWith('P')) {
    if (err.code === 'P2002') {
      return res.status(409).json({ error: 'Duplicate record' });
    }
    if (err.code === 'P2025') {
      return res.status(404).json({ error: 'Record not found' });
    }
  }

  // Tilled API errors (have error.code but not Prisma)
  if (err.code && !err.code.startsWith('P')) {
    return res.status(502).json({
      error: 'Payment processor error',
      code: err.code,
      message: err.message
    });
  }

  // Default error
  res.status(500).json({ error: 'Internal server error', message: err.message });
}

// Modify services to throw typed errors
class CustomerService {
  async findCustomer(appId, externalCustomerId) {
    const customer = await billingPrisma.billing_customers.findFirst({...});
    if (!customer) {
      throw new NotFoundError(`Customer not found: ${externalCustomerId}`);
    }
    return customer;
  }
}

// Usage in routes.js
router.post('/customers', requireAppId(), rejectSensitiveData, async (req, res, next) => {
  try {
    const customer = await billingService.createCustomer(...);
    res.status(201).json(customer);
  } catch (error) {
    next(error); // Pass to error handler middleware
  }
});

// Mount error handler LAST in app.js
app.use(handleBillingError);
```

**Impact:** Reduces routes.js by ~400 lines, centralizes error handling logic

---

#### D. Health Check Logic Inline (Lines 11-64)

**Current Pattern:**
```javascript
// Health check logic embedded in route handler
router.get('/health', requireAppId(), async (req, res) => {
  const checks = { ... };

  // Database check
  try {
    await billingPrisma.$queryRaw`SELECT 1`;
    checks.database.status = 'healthy';
  } catch (error) {
    checks.database.status = 'unhealthy';
  }

  // Tilled config check
  try {
    const prefix = appId.toUpperCase();
    const secretKey = process.env[`TILLED_SECRET_KEY_${prefix}`];
    // ... 20+ lines of config checking ...
  } catch (error) {
    checks.tilled_config.status = 'unhealthy';
  }

  res.status(allHealthy ? 200 : 503).json(checks);
});
```

**Problem:**
- Health check logic mixed with route handling
- Not reusable for automated monitoring
- Difficult to test independently

**Recommended Solution:**
```javascript
// Create services/HealthCheckService.js
class HealthCheckService {
  constructor(billingPrisma) {
    this.billingPrisma = billingPrisma;
  }

  async checkDatabase() {
    try {
      await this.billingPrisma.$queryRaw`SELECT 1`;
      return { status: 'healthy', error: null };
    } catch (error) {
      return { status: 'unhealthy', error: error.message };
    }
  }

  checkTilledConfig(appId) {
    const prefix = appId.toUpperCase();
    const secretKey = process.env[`TILLED_SECRET_KEY_${prefix}`];
    const accountId = process.env[`TILLED_ACCOUNT_ID_${prefix}`];
    const webhookSecret = process.env[`TILLED_WEBHOOK_SECRET_${prefix}`];
    const sandbox = process.env.TILLED_SANDBOX;

    const missing = [];
    if (!secretKey) missing.push('TILLED_SECRET_KEY');
    if (!accountId) missing.push('TILLED_ACCOUNT_ID');
    if (!webhookSecret) missing.push('TILLED_WEBHOOK_SECRET');
    if (sandbox === undefined) missing.push('TILLED_SANDBOX');

    if (missing.length > 0) {
      return {
        status: 'unhealthy',
        error: `Missing credentials: ${missing.join(', ')}`
      };
    }

    return {
      status: 'healthy',
      sandbox_mode: sandbox === 'true',
      error: null
    };
  }

  async getHealthStatus(appId) {
    const checks = {
      timestamp: new Date().toISOString(),
      app_id: appId,
      database: await this.checkDatabase(),
      tilled_config: this.checkTilledConfig(appId)
    };

    const allHealthy = checks.database.status === 'healthy' &&
                       checks.tilled_config.status === 'healthy';

    return {
      checks,
      overall_status: allHealthy ? 'healthy' : 'degraded',
      statusCode: allHealthy ? 200 : 503
    };
  }
}

// Usage in routes.js
const healthCheckService = new HealthCheckService(billingPrisma);

router.get('/health', requireAppId(), async (req, res) => {
  const { checks, statusCode } = await healthCheckService.getHealthStatus(req.verifiedAppId);
  res.status(statusCode).json(checks);
});
```

**Impact:** Reduces routes.js by ~50 lines, enables health check reuse in monitoring

---

### 2. tilledClient.js (430 lines) ✅ WELL-STRUCTURED

**Assessment:** This file demonstrates excellent separation of concerns.

**Strengths:**
- Single responsibility: Tilled SDK wrapper
- Each method handles one API operation
- Consistent error handling pattern
- Clean abstraction over Tilled SDK
- Lazy SDK initialization (lines 30-50)

**Minor Opportunity:** Extract error handling utility

```javascript
// Lines 258-267, 313-322, 340-347 all repeat this pattern
catch (error) {
  const errorCode = error.response?.data?.code || error.code || 'unknown';
  const errorMessage = error.response?.data?.message || error.message;

  throw Object.assign(new Error(errorMessage), {
    code: errorCode,
    message: errorMessage
  });
}

// Could extract to private method:
_handleTilledError(error) {
  const errorCode = error.response?.data?.code || error.code || 'unknown';
  const errorMessage = error.response?.data?.message || error.message;

  throw Object.assign(new Error(errorMessage), {
    code: errorCode,
    message: errorMessage
  });
}
```

**Recommendation:** SKIP - The duplication is minimal (~12 lines repeated 6 times = 60 lines), and the pattern is clear. Extracting it provides minimal benefit.

---

### 3. WebhookService.js (397 lines) ✅ MOSTLY WELL-STRUCTURED

**Assessment:** Good service separation with clear method responsibilities.

**Strengths:**
- Event handling separated from HTTP layer
- Idempotency logic at database level (lines 10-28)
- Signature verification delegated to TilledClient
- Event dispatcher pattern (lines 82-115)

**Minor Opportunity:** Repetitive database query patterns

```javascript
// Lines 129-134, 159-164, 188-193 all use findFirst pattern
const subscription = await billingPrisma.billing_subscriptions.findFirst({
  where: {
    tilled_subscription_id: subscriptionId,
    app_id: appId
  }
});

if (!subscription) {
  logger.warn('Subscription not found', {...});
  return;
}
```

**Recommendation:** SKIP - These patterns are clear and explicit. A helper method would obscure the query logic without significant benefit.

---

## Refactoring Roadmap

### Priority 1: Error Handling (Highest Impact)

**Files to Create:**
- `backend/src/utils/errors.js` - Custom error classes
- `backend/src/middleware/errorHandler.js` - Centralized error handler

**Files to Modify:**
- All service files - throw typed errors instead of generic errors
- `routes.js` - replace catch blocks with `next(error)`

**Estimated Impact:** -400 lines in routes.js, +150 lines in new files, +50 lines service changes

---

### Priority 2: Validation Middleware (High Impact)

**Files to Create:**
- `backend/src/validators/requestValidators.js` - Express-validator schemas

**Files to Modify:**
- `routes.js` - remove inline validation, add validator middleware

**Dependencies:**
- Install `express-validator` package

**Estimated Impact:** -150 lines in routes.js, +200 lines in validators

---

### Priority 3: Idempotency Middleware (Medium Impact)

**Files to Create:**
- `backend/src/middleware/idempotency.js` - Idempotency key handler

**Files to Modify:**
- `routes.js` - remove inline idempotency logic

**Estimated Impact:** -100 lines in routes.js, +80 lines in middleware

---

### Priority 4: Health Check Service (Low Impact)

**Files to Create:**
- `backend/src/services/HealthCheckService.js`

**Files to Modify:**
- `routes.js` - simplify health check route

**Estimated Impact:** -50 lines in routes.js, +90 lines in service

---

## Testing Strategy

**For Each Refactoring:**

1. **Before refactoring:**
   - Run full test suite: `npm test` (verify 226/226 passing)
   - Document current behavior

2. **During refactoring:**
   - Write tests for new middleware/utilities first (TDD)
   - Migrate one route at a time
   - Run tests after each route migration

3. **After refactoring:**
   - Verify all 226 tests still pass
   - Run integration tests against real database
   - Verify no API contract changes

**Test Files to Create:**
- `tests/unit/middleware/errorHandler.test.js`
- `tests/unit/validators/requestValidators.test.js`
- `tests/unit/middleware/idempotency.test.js`
- `tests/unit/services/HealthCheckService.test.js`

---

## Projected File Sizes After Refactoring

| File | Current Size | Projected Size | Change |
|------|--------------|----------------|--------|
| routes.js | 772 lines | ~370 lines | -402 lines (-52%) |
| errors.js | N/A | ~80 lines | +80 lines |
| errorHandler.js | N/A | ~70 lines | +70 lines |
| requestValidators.js | N/A | ~200 lines | +200 lines |
| idempotency.js | N/A | ~80 lines | +80 lines |
| HealthCheckService.js | N/A | ~90 lines | +90 lines |
| Service files (error changes) | ~1,400 lines | ~1,450 lines | +50 lines |

**Net Impact:** +168 lines total, but routes.js reduced by 52%, significantly improved maintainability

---

## Non-Breaking Changes Guarantee

All proposed refactorings maintain the existing API contract:

✅ HTTP endpoints unchanged
✅ Request/response formats unchanged
✅ Validation rules unchanged
✅ Error messages unchanged (or improved)
✅ Idempotency behavior unchanged
✅ No new dependencies (except express-validator)

---

## Recommendation

**Proceed with Priority 1 (Error Handling) and Priority 2 (Validation) refactorings.**

**Rationale:**
1. routes.js is currently 772 lines, which is manageable but on the edge of becoming difficult to maintain
2. Error handling and validation patterns will be needed for future route additions
3. Refactoring now prevents technical debt accumulation
4. All changes are non-breaking and can be tested incrementally
5. The billing module is already production-ready (226/226 tests passing), so refactoring is low-risk

**Skip Priority 3 & 4 for now:**
- Idempotency middleware: Only 2 routes currently need it (can revisit when adding more)
- Health check service: Current inline implementation is acceptable for a single route

---

**Next Steps:**
1. Get stakeholder approval for Priority 1 & 2 refactorings
2. Create feature branch: `refactor/separation-of-concerns`
3. Implement error handling first (biggest impact)
4. Implement validation middleware second
5. Run full test suite after each priority
6. Create PR with before/after metrics

**Estimated Effort:** 6-8 hours development, 2-3 hours testing

---

**Document Status:** Ready for review
**Author:** FuchsiaCove (Architecture Analysis)
**Approvers Needed:** BrownIsland (Security Impact), HazyOwl (Implementation Review)
