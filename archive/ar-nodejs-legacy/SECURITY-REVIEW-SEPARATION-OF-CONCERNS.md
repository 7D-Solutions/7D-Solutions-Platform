# Security Review: Separation of Concerns Refactoring

**Review Date:** 2026-01-31
**Reviewer:** BrownIsland (Security Audit)
**Document Reviewed:** SEPARATION-OF-CONCERNS-ANALYSIS.md
**Review Status:** ✅ APPROVED with security requirements

---

## Executive Summary

**Security Assessment:** The proposed refactorings are **SECURE** and **IMPROVE** the overall security posture of the billing module.

**Key Security Benefits:**
- ✅ Centralized validation reduces risk of missed validation checks
- ✅ Centralized error handling reduces information disclosure risk
- ✅ Typed error classes prevent error message leakage
- ✅ Middleware patterns maintain existing multi-tenant isolation
- ✅ No new attack surface introduced

**Approval Status:** ✅ APPROVED for Priority 1 & 2 (Error Handling + Validation)

**Critical Requirements:** See Section 3 (Security Requirements for Implementation)

---

## 1. Security Analysis by Component

### A. Validation Middleware (express-validator) ✅ SECURE

**Proposed Pattern:**
```javascript
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
```

**Security Assessment:**

✅ **Strengths:**
- express-validator is a well-audited, widely-used library (13M+ weekly downloads)
- Centralized validation ensures consistent enforcement across all routes
- Reduces risk of forgotten validation checks (current inline pattern requires manual duplication)
- Email validation uses express-validator's `.isEmail()` (RFC 5322 compliant, more robust than regex)
- Validation error messages are controlled and do not leak sensitive data

✅ **Multi-Tenant Isolation:**
- Validation middleware executes AFTER `requireAppId()` middleware
- Does not interact with app_id scoping (scoping happens in service layer)
- No cross-tenant data access risk

⚠️ **Security Requirements:**
1. **Middleware Order:** Validation MUST execute after `requireAppId()` and before business logic
2. **Error Messages:** Error messages MUST NOT expose sensitive data (current messages are safe)
3. **Sanitization:** Input sanitization should be considered for free-text fields (name, note, metadata)

**Recommended Additions:**
```javascript
body('name').trim().escape().notEmpty()  // Prevent XSS in logs/responses
body('note').optional().trim().escape()  // Sanitize optional text fields
```

---

### B. Idempotency Middleware ✅ SECURE with Requirements

**Proposed Pattern:**
```javascript
function requireIdempotency(billingService) {
  return async (req, res, next) => {
    const idempotencyKey = req.headers['idempotency-key'];
    if (!idempotencyKey) {
      return res.status(400).json({ error: 'Idempotency-Key header is required' });
    }

    const appId = req.verifiedAppId;  // ← CRITICAL: Uses verified app_id
    const requestHash = billingService.computeRequestHash(req.method, req.path, req.body);

    const cachedResponse = await billingService.getIdempotentResponse(appId, idempotencyKey, requestHash);
    if (cachedResponse) {
      return res.status(cachedResponse.statusCode).json(cachedResponse.body);
    }

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
```

**Security Assessment:**

✅ **Strengths:**
- Uses `req.verifiedAppId` (set by `requireAppId()` middleware) for app_id scoping
- Idempotent cache is scoped by `appId` (multi-tenant isolation preserved)
- Request hash computation includes method + path + body (prevents cache collision)
- Response interception preserves original response behavior
- Async error handling for cache storage (doesn't block response)

✅ **Multi-Tenant Isolation:**
- `billingService.getIdempotentResponse(appId, ...)` scopes cache by app_id
- No risk of cross-tenant cache pollution
- Verified in IdempotencyService.js (uses `app_id` in WHERE clause)

⚠️ **Security Requirements:**
1. **Middleware Order:** MUST execute AFTER `requireAppId()` (requires `req.verifiedAppId`)
2. **Cache Scoping:** `billingService.storeIdempotentResponse()` MUST include app_id in database query
3. **Request Hash:** MUST use `req.path` (not hardcoded route) to prevent cache collision across routes
4. **Error Handling:** Cache storage errors MUST NOT fail the request (current implementation correct)

**Verification Needed:**
- Confirm `IdempotencyService.storeIdempotentResponse()` uses app_id in INSERT query ✅ (verified in routes.js:574-581)

---

### C. Error Handler Middleware ✅ SECURE

**Proposed Pattern:**
```javascript
class NotFoundError extends BillingError {
  constructor(message) { super(message, 404); }
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

  // Tilled API errors
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
```

**Security Assessment:**

✅ **Strengths:**
- Centralized error handling reduces risk of inconsistent error messages
- Typed error classes (NotFoundError, ValidationError) prevent error message leakage
- Prisma error codes are mapped to generic messages (prevents internal detail exposure)
- Generic "Internal server error" for unknown errors (safe fallback)
- Error logging includes full error details (for debugging) but response sanitized

✅ **Information Disclosure Prevention:**
- Prisma errors mapped to generic messages ("Duplicate record", "Record not found")
- No stack traces exposed in responses
- Internal error codes (P2002, P2025) not exposed to client
- Tilled API errors include `error.code` (safe - Tilled codes are designed for client visibility)

⚠️ **Security Requirements:**
1. **Production Mode:** In production, `err.message` for 500 errors should be replaced with generic message
2. **Stack Traces:** MUST NOT include stack traces in responses (current implementation correct)
3. **Service Layer:** Services MUST throw typed errors (NotFoundError, ValidationError) instead of generic errors
4. **Tilled Errors:** Verify Tilled error codes do not leak sensitive information (acceptable - Tilled codes are client-safe)

**Recommended Enhancement:**
```javascript
// Default error (production-safe)
const message = process.env.NODE_ENV === 'production'
  ? 'An unexpected error occurred'
  : err.message;

res.status(500).json({ error: 'Internal server error', message });
```

---

### D. Health Check Service ✅ SECURE

**Proposed Pattern:**
```javascript
class HealthCheckService {
  checkTilledConfig(appId) {
    const prefix = appId.toUpperCase();
    const secretKey = process.env[`TILLED_SECRET_KEY_${prefix}`];
    const accountId = process.env[`TILLED_ACCOUNT_ID_${prefix}`];
    const webhookSecret = process.env[`TILLED_WEBHOOK_SECRET_${prefix}`];

    const missing = [];
    if (!secretKey) missing.push('TILLED_SECRET_KEY');
    if (!accountId) missing.push('TILLED_ACCOUNT_ID');
    if (!webhookSecret) missing.push('TILLED_WEBHOOK_SECRET');

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
}
```

**Security Assessment:**

✅ **Strengths:**
- Does NOT expose actual environment variable values (only checks presence)
- Uses `appId` parameter to scope credential checks (multi-tenant aware)
- Returns only status ('healthy'/'unhealthy') and generic error messages
- Sandbox mode flag is safe to expose (not sensitive)

✅ **Information Disclosure:**
- Does not leak credential values
- Only indicates WHICH credentials are missing (acceptable for health checks)
- No internal paths, database connection strings, or sensitive config exposed

⚠️ **Security Requirements:**
1. **Route Protection:** Health check route MUST use `requireAppId()` middleware (verified in SEPARATION-OF-CONCERNS-ANALYSIS.md line 369)
2. **Credential Values:** MUST NOT return actual credential values (current implementation correct)
3. **Error Details:** Missing credential names are acceptable (helps debugging without exposing values)

**No Changes Needed:** Current implementation is secure.

---

## 2. Multi-Tenant Isolation Verification

**Critical Requirement:** All refactorings MUST preserve existing multi-tenant isolation.

### Middleware Chain Order (CRITICAL)

**Current Order (SECURE):**
```javascript
router.post('/customers',
  requireAppId(),           // 1. Authenticate app, set req.verifiedAppId
  rejectSensitiveData,      // 2. Block PCI-sensitive fields
  createCustomerValidator,  // 3. Validate input (NEW)
  async (req, res) => {     // 4. Business logic
    // Uses req.verifiedAppId for app_id scoping
  }
);
```

**For Idempotent Routes:**
```javascript
router.post('/charges/one-time',
  requireAppId(),             // 1. Authenticate app, set req.verifiedAppId
  rejectSensitiveData,        // 2. Block PCI-sensitive fields
  requireIdempotency(service),// 3. Check/cache idempotent response (NEW)
  chargeValidator,            // 4. Validate input (NEW)
  async (req, res) => {       // 5. Business logic
    // Idempotency and validation already handled
  }
);
```

**Error Handling (applies to ALL routes):**
```javascript
// In main app.js, AFTER all routes
app.use('/api/billing', billingRoutes);
app.use(handleBillingError);  // MUST be last
```

✅ **Verification:**
- `requireAppId()` MUST be first (sets req.verifiedAppId)
- `rejectSensitiveData` MUST be before business logic (PCI compliance)
- `requireIdempotency()` MUST be after `requireAppId()` (uses req.verifiedAppId)
- Validation middleware MUST be before business logic
- Error handler MUST be last in middleware chain

---

### App ID Scoping Preservation

**Current Implementation (SECURE):**
- All database queries include `app_id` in WHERE clause (verified in APP_ID_SCOPING_AUDIT.md)
- `requireAppId()` middleware sets `req.verifiedAppId`
- Services use `req.verifiedAppId` for all database operations

**Refactoring Impact:**
- ✅ Validation middleware: Does NOT interact with database (no app_id scoping needed)
- ✅ Idempotency middleware: Uses `req.verifiedAppId` for cache scoping (secure)
- ✅ Error handler: Does NOT interact with database (no app_id scoping needed)
- ✅ Health check: Uses `appId` parameter (secure)

**No New Risks:** All refactorings maintain existing app_id scoping.

---

## 3. Security Requirements for Implementation

### Phase 1: Error Handling Refactoring

**REQUIRED Security Measures:**

1. **Service Layer Error Types**
   - CustomerService MUST throw `NotFoundError` for missing customers
   - SubscriptionService MUST throw `ValidationError` for invalid input
   - PaymentMethodService MUST throw `ConflictError` for duplicate operations

2. **Error Message Sanitization**
   - Production mode MUST use generic messages for 500 errors
   - MUST NOT expose stack traces in responses
   - Prisma error codes MUST be mapped to generic messages

3. **Testing Requirements**
   - Test error handler with Prisma P2002 (duplicate) → 409 Conflict
   - Test error handler with Prisma P2025 (not found) → 404 Not Found
   - Test error handler with typed errors (NotFoundError, ValidationError)
   - Test error handler with unknown errors → 500 with generic message

**Implementation Checklist:**
- [ ] Create `utils/errors.js` with BillingError base class
- [ ] Create `middleware/errorHandler.js` with handleBillingError function
- [ ] Modify all services to throw typed errors
- [ ] Update routes.js to use `next(error)` instead of inline catch blocks
- [ ] Add production mode check for 500 error messages
- [ ] Write unit tests for error handler
- [ ] Verify all 226 tests still pass

---

### Phase 2: Validation Middleware Refactoring

**REQUIRED Security Measures:**

1. **Input Sanitization**
   - Email: Use `.isEmail()` (NOT regex - more robust)
   - Text fields (name, note): Use `.trim().escape()` to prevent XSS
   - Numeric fields (amount_cents): Use `.isInt({ min: 1 })` to prevent negative amounts
   - Optional fields: Use `.optional()` to allow missing fields

2. **Middleware Order**
   - Validation MUST execute AFTER `requireAppId()`
   - Validation MUST execute BEFORE business logic
   - Idempotency (if present) SHOULD execute BEFORE validation (cache check before expensive validation)

3. **Error Messages**
   - Use `.withMessage()` to provide consistent error messages
   - MUST NOT expose internal field names or database structure
   - Use generic messages: "Invalid email format" (NOT "billing_customers.email is invalid")

**Implementation Checklist:**
- [ ] Install `express-validator` package
- [ ] Create `validators/requestValidators.js` with all route validators
- [ ] Add input sanitization (`.trim()`, `.escape()`) for text fields
- [ ] Update routes.js to use validator middleware
- [ ] Verify middleware order: requireAppId → rejectSensitiveData → validators → business logic
- [ ] Write unit tests for validators
- [ ] Verify all 226 tests still pass

---

### Phase 3: Idempotency Middleware (Optional)

**REQUIRED Security Measures:**

1. **Cache Scoping**
   - `getIdempotentResponse()` MUST include `app_id` in WHERE clause
   - `storeIdempotentResponse()` MUST include `app_id` in INSERT
   - Request hash MUST include `req.path` to prevent cross-route collision

2. **Response Interception**
   - MUST preserve original response behavior (status code, headers, body)
   - MUST handle async cache storage errors gracefully
   - MUST NOT expose cache storage errors to client

3. **Middleware Order**
   - MUST execute AFTER `requireAppId()` (requires req.verifiedAppId)
   - SHOULD execute BEFORE validation (cache check before expensive validation)

**Implementation Checklist:**
- [ ] Create `middleware/idempotency.js` with requireIdempotency function
- [ ] Verify IdempotencyService uses app_id in all queries
- [ ] Update routes.js to use idempotency middleware (2 routes currently)
- [ ] Verify middleware order: requireAppId → requireIdempotency → validators → business logic
- [ ] Write unit tests for idempotency middleware
- [ ] Test concurrent duplicate requests (should only process once)
- [ ] Verify all 226 tests still pass

---

## 4. Security Testing Requirements

### Unit Tests (NEW)

**Required Test Files:**
- `tests/unit/middleware/errorHandler.test.js` (40+ test cases)
- `tests/unit/validators/requestValidators.test.js` (60+ test cases)
- `tests/unit/middleware/idempotency.test.js` (30+ test cases)

**Critical Test Cases:**

**Error Handler:**
- ✅ NotFoundError → 404 with correct message
- ✅ ValidationError → 400 with correct message
- ✅ ConflictError → 409 with correct message
- ✅ Prisma P2002 → 409 Conflict
- ✅ Prisma P2025 → 404 Not Found
- ✅ Tilled API error → 502 with error code
- ✅ Unknown error → 500 with generic message (production mode)
- ✅ No stack traces in responses

**Validation Middleware:**
- ✅ Valid email → passes
- ✅ Invalid email → 400 error
- ✅ Missing required field → 400 error
- ✅ XSS attempt in name field → sanitized
- ✅ Negative amount_cents → 400 error
- ✅ Optional field missing → passes
- ✅ Optional field present and valid → passes

**Idempotency Middleware:**
- ✅ Missing Idempotency-Key → 400 error
- ✅ First request → processes and caches
- ✅ Duplicate request (same key + hash) → returns cached response
- ✅ Same key, different hash → processes as new request
- ✅ Cache storage error → request still succeeds
- ✅ Multi-tenant: app1 cannot access app2's cached responses

---

### Integration Tests (EXISTING)

**Verification Required:**
- ✅ All 226 existing tests MUST still pass after refactoring
- ✅ Multi-tenant isolation tests MUST pass (app_id scoping)
- ✅ Idempotency concurrency tests MUST pass (10 duplicates → 1 processed)
- ✅ Webhook processing tests MUST pass (signature verification)

**New Integration Tests (if needed):**
- Test validation middleware with real database (e.g., email uniqueness)
- Test error handler with real Prisma errors
- Test idempotency middleware with real cache storage

---

## 5. Risk Assessment

### Refactoring Risks

| Risk | Likelihood | Impact | Mitigation |
|------|-----------|--------|------------|
| Middleware order incorrect | Medium | High | Clear documentation, integration tests |
| Validation bypass | Low | High | Unit tests, code review |
| Error message leakage | Low | Medium | Production mode check, code review |
| Cache scoping failure | Low | High | Unit tests with multi-tenant data |
| Breaking API contract | Low | Medium | Integration tests, no API changes |

### Security Benefits

✅ **Reduced Attack Surface:**
- Centralized validation prevents forgotten checks
- Centralized error handling prevents information leakage
- Middleware patterns enforce security controls consistently

✅ **Improved Maintainability:**
- Security controls in one place (easier to audit)
- Changes to validation/error handling require fewer file modifications
- Reduced code duplication reduces risk of inconsistencies

✅ **Defense in Depth:**
- Validation middleware adds second layer of validation (first layer: services)
- Error handler ensures no unexpected error responses
- Idempotency middleware prevents duplicate charge attacks

---

## 6. Approval Decision

### ✅ APPROVED: Priority 1 (Error Handling)

**Rationale:**
- Centralized error handling reduces information disclosure risk
- Typed error classes improve security posture
- All proposed patterns are secure
- No new attack surface introduced

**Conditions:**
- MUST implement production mode check for 500 error messages
- MUST verify no stack traces in responses
- MUST write comprehensive unit tests (40+ test cases)
- MUST verify all 226 integration tests still pass

---

### ✅ APPROVED: Priority 2 (Validation Middleware)

**Rationale:**
- express-validator is well-audited, industry-standard library
- Centralized validation reduces risk of missed validation
- Input sanitization improves XSS protection
- All proposed patterns are secure

**Conditions:**
- MUST add input sanitization (`.trim()`, `.escape()`) for text fields
- MUST verify middleware order (requireAppId → validators → business logic)
- MUST write comprehensive unit tests (60+ test cases)
- MUST verify all 226 integration tests still pass

---

### ⏸️ DEFERRED: Priority 3 (Idempotency Middleware)

**Rationale:**
- Only 2 routes currently need idempotency (low ROI)
- Current inline implementation is secure
- Can be implemented later when more routes need it

**Recommendation:** Revisit when 4+ routes need idempotency

---

### ⏸️ DEFERRED: Priority 4 (Health Check Service)

**Rationale:**
- Current inline implementation is secure
- Only 1 route uses health check (low ROI)
- No security improvement from refactoring

**Recommendation:** SKIP unless health checks are needed elsewhere

---

## 7. Implementation Roadmap

### Phase 1: Error Handling (Days 1-2)

**Day 1:**
- [ ] Create `utils/errors.js` with error classes
- [ ] Create `middleware/errorHandler.js`
- [ ] Write unit tests for error handler (40+ test cases)
- [ ] Verify tests pass

**Day 2:**
- [ ] Modify all service files to throw typed errors
- [ ] Update routes.js to use `next(error)` pattern
- [ ] Run full test suite (verify 226/226 passing)
- [ ] Git commit: "Refactor: Centralized error handling"

---

### Phase 2: Validation Middleware (Days 3-4)

**Day 3:**
- [ ] Install `express-validator` package
- [ ] Create `validators/requestValidators.js` with all validators
- [ ] Add input sanitization for text fields
- [ ] Write unit tests for validators (60+ test cases)
- [ ] Verify tests pass

**Day 4:**
- [ ] Update routes.js to use validator middleware
- [ ] Verify middleware order (requireAppId → validators → business logic)
- [ ] Run full test suite (verify 226/226 passing)
- [ ] Git commit: "Refactor: Centralized validation middleware"

---

### Phase 3: Security Review & Sign-off (Day 5)

- [ ] Security review of all changes (BrownIsland)
- [ ] Code review of implementation (HazyOwl)
- [ ] Final test suite run (226/226 passing)
- [ ] Update documentation (this file + SEPARATION-OF-CONCERNS-ANALYSIS.md)
- [ ] Create PR with before/after metrics
- [ ] Merge to main

---

## 8. Conclusion

**Security Verdict:** ✅ APPROVED with requirements

The proposed refactorings in SEPARATION-OF-CONCERNS-ANALYSIS.md are **secure** and **improve** the security posture of the billing module. All patterns maintain existing multi-tenant isolation and introduce no new attack surface.

**Key Security Benefits:**
1. Centralized validation reduces risk of forgotten validation checks
2. Centralized error handling reduces information disclosure risk
3. Typed error classes prevent error message leakage
4. Middleware patterns enforce security controls consistently

**Critical Requirements:**
1. Implement production mode check for 500 error messages
2. Add input sanitization for text fields (`.trim()`, `.escape()`)
3. Verify middleware chain order (requireAppId → validators → business logic)
4. Write comprehensive unit tests (100+ test cases total)
5. Verify all 226 integration tests still pass after refactoring

**Recommendation:** Proceed with Priority 1 (Error Handling) and Priority 2 (Validation) refactorings.

---

**Reviewer:** BrownIsland (Security Audit)
**Review Date:** 2026-01-31
**Next Reviewer:** HazyOwl (Implementation Review)
**Status:** ✅ Security Approved - Ready for Implementation
