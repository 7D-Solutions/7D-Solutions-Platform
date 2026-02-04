# Billing Module Testing Strategy

## Overview

The billing module uses a **layered testing approach** aligned with Fireproof ERP's testing standards:

- **Unit tests** - Fast, isolated tests with mocked dependencies
- **Integration tests** - Real database, mocked external APIs
- **Real API tests** - Optional tests against Tilled sandbox (expensive)

## Test Structure

```
packages/billing/tests/
├── setup.js                    # Global test configuration
├── fixtures/                   # Centralized test data
│   └── test-fixtures.js
├── helpers/                    # Reusable test utilities
│   └── index.js
├── unit/                       # Unit tests (fast, mocked)
│   ├── tilledClient.test.js
│   ├── billingService.test.js
│   └── middleware.test.js
├── integration/                # Integration tests (real DB)
│   ├── database-cleanup.js
│   ├── billingService.real.test.js
│   └── routes.test.js
└── real/                       # Real API tests (optional)
    └── tilled-sandbox.test.js
```

## Test Categories

### Unit Tests (`tests/unit/`)

**Purpose:** Test individual functions/classes in isolation

**Dependencies:**
- All external dependencies mocked (Tilled SDK, database)
- No network calls
- No database operations

**Speed:** Very fast (~5ms per test)

**When to run:** Every code change, in watch mode during development

**Example:**
```javascript
// Mock all dependencies
jest.mock('../../backend/src/tilledClient');
jest.mock('../../backend/src/prisma');

describe('BillingService', () => {
  it('should create customer', async () => {
    mockTilledClient.createCustomer.mockResolvedValue({ id: 'cus_123' });
    mockPrisma.create.mockResolvedValue({ id: 1 });

    const result = await service.createCustomer(...);
    expect(result.id).toBe(1);
  });
});
```

**Run:**
```bash
npm run test:unit
```

### Integration Tests (`tests/integration/`)

**Purpose:** Test full stack with real database

**Dependencies:**
- **Real database** (separate test database required)
- **Mocked Tilled API** (no real charges)
- Real Express routes and middleware

**Speed:** Medium (~50-200ms per test)

**When to run:** Before commits, in CI pipeline

**Database Strategy:**
- Use dedicated test database (`billing_test`)
- `beforeEach` truncates all tables (clean slate)
- `afterAll` disconnects Prisma client

**Example:**
```javascript
describe('BillingService Integration', () => {
  beforeEach(async () => {
    await cleanDatabase(); // Truncate all tables
  });

  it('should enforce unique constraints', async () => {
    await service.createCustomer('trashtech', 'test@example.com', ...);

    // Duplicate should fail
    await expect(
      service.createCustomer('trashtech', 'test@example.com', ...)
    ).rejects.toThrow();
  });
});
```

**Run:**
```bash
npm run test:integration
```

### Real API Tests (`tests/real/`)

**Purpose:** Test against actual Tilled sandbox API

**Dependencies:**
- Real Tilled sandbox account
- Real network calls
- Real database

**Speed:** Very slow (~1-5 seconds per test)

**Cost:** Uses real API quota, potential charges

**When to run:**
- Manually before production deployment
- Nightly CI runs (optional)
- Never in PR builds

**Example:**
```javascript
describe('Tilled Sandbox Integration', () => {
  it('should create real customer in sandbox', async () => {
    // This hits real Tilled API
    const customer = await realTilledClient.createCustomer(...);
    expect(customer.id).toMatch(/^cus_/);
  });
});
```

**Run:**
```bash
npm run test:real  # (not implemented - add manually if needed)
```

## Database Cleanup Strategy

### Problem
Parallel tests + shared database = flaky tests

### Solution
**Dedicated test database + truncate between tests**

#### Setup
1. Create separate test database:
```sql
CREATE DATABASE billing_test;
```

2. Set environment variable:
```bash
DATABASE_URL_BILLING="mysql://user:pass@localhost:3306/billing_test"
```

3. Run migrations:
```bash
npx prisma migrate deploy --schema=packages/billing/prisma/schema.prisma
```

#### Cleanup Pattern
```javascript
// tests/integration/database-cleanup.js
async function cleanDatabase() {
  // Delete in reverse dependency order
  await billingPrisma.billing_webhooks.deleteMany({});
  await billingPrisma.billing_subscriptions.deleteMany({});
  await billingPrisma.billing_customers.deleteMany({});
}

// In each test file
beforeEach(async () => {
  await cleanDatabase(); // Clean slate for each test
});
```

#### Safety Check
Tests verify you're using test database:
```javascript
if (!dbUrl.includes('_test')) {
  throw new Error('Must use test database');
}
```

## Running Tests

### Quick Reference

```bash
# All tests
npm test

# Unit tests only (fast - use during development)
npm run test:unit

# Integration tests only (medium speed)
npm run test:integration

# Watch mode (re-run on file changes)
npm run test:watch

# Coverage report
npm run test:coverage
```

### Development Workflow

1. **Write code + unit test simultaneously**
   ```bash
   npm run test:watch
   ```

2. **Before committing - run integration tests**
   ```bash
   npm run test:integration
   ```

3. **Full test suite before pushing**
   ```bash
   npm test
   ```

## Test Time Budgets

**Target times:**
- Unit tests: **< 5 seconds** for full suite
- Integration tests: **< 30 seconds** for full suite
- Combined: **< 35 seconds** total

If exceeded:
- Refactor slow tests
- Split into smaller focused tests
- Check for unnecessary database operations

## Test Data Management

### Centralized Fixtures
All test data lives in `tests/fixtures/test-fixtures.js`:

```javascript
const TEST_CUSTOMERS = {
  standard: {
    app_id: 'trashtech',
    email: 'test@acmewaste.com',
    name: 'Acme Waste Inc'
  }
};

const TILLED_CUSTOMER_RESPONSE = {
  id: 'cus_test_123456',
  email: 'test@acmewaste.com'
};
```

**Benefits:**
- Single source of truth
- Easy to update
- Consistent across tests
- Type-safe with JSDoc

### Test Helpers
Reusable utilities in `tests/helpers/index.js`:

```javascript
async function createTestCustomer(billingPrisma, data) {
  return billingPrisma.billing_customers.create({ data });
}
```

## CI/CD Integration

### GitHub Actions Example
```yaml
test:
  runs-on: ubuntu-latest
  services:
    mysql:
      image: mysql:8.0
      env:
        MYSQL_ROOT_PASSWORD: password
        MYSQL_DATABASE: billing_test
  steps:
    - name: Run unit tests
      run: npm run test:unit

    - name: Run integration tests
      run: npm run test:integration
      env:
        DATABASE_URL_BILLING: mysql://root:password@localhost:3306/billing_test
```

## Best Practices

### ✅ Do This

1. **Use fixtures for test data**
   ```javascript
   import { TEST_CUSTOMERS } from '../fixtures/test-fixtures';
   await service.createCustomer(...TEST_CUSTOMERS.standard);
   ```

2. **Clean database between tests**
   ```javascript
   beforeEach(async () => {
     await cleanDatabase();
   });
   ```

3. **Mock external APIs in integration tests**
   ```javascript
   jest.mock('../../backend/src/tilledClient');
   mockTilledClient.createCustomer.mockResolvedValue(...);
   ```

4. **Test error paths, not just happy path**
   ```javascript
   it('should throw error if customer not found', async () => {
     await expect(service.cancelSubscription(999))
       .rejects.toThrow('Subscription 999 not found');
   });
   ```

5. **Test database constraints**
   ```javascript
   it('should enforce unique constraint', async () => {
     await createCustomer('trashtech', 'test@example.com');
     await expect(
       createCustomer('trashtech', 'test@example.com')
     ).rejects.toThrow();
   });
   ```

### ❌ Don't Do This

1. **Don't hit real Tilled API in regular tests**
   ```javascript
   // ❌ Bad - costs money, slow, flaky
   const customer = await realTilledClient.createCustomer(...);

   // ✅ Good - fast, free, reliable
   mockTilledClient.createCustomer.mockResolvedValue({ id: 'cus_123' });
   ```

2. **Don't skip database cleanup**
   ```javascript
   // ❌ Bad - tests pollute each other
   it('test 1', async () => {
     await createCustomer(...);
   });

   // ✅ Good - clean slate each time
   beforeEach(async () => await cleanDatabase());
   ```

3. **Don't hardcode test data inline**
   ```javascript
   // ❌ Bad - duplicated, hard to maintain
   await service.createCustomer('trashtech', 'test@example.com', ...);

   // ✅ Good - centralized, reusable
   await service.createCustomer(...TEST_CUSTOMERS.standard);
   ```

4. **Don't test implementation details**
   ```javascript
   // ❌ Bad - brittle, coupled to implementation
   expect(service.tilledClients.has('trashtech')).toBe(true);

   // ✅ Good - test behavior, not internals
   expect(customer.tilled_customer_id).toMatch(/^cus_/);
   ```

5. **Don't use production database for tests**
   ```javascript
   // ❌ NEVER EVER DO THIS
   DATABASE_URL_BILLING="mysql://root:pass@prod-server/billing"

   // ✅ Always use test database
   DATABASE_URL_BILLING="mysql://root:pass@localhost:3306/billing_test"
   ```

## Coverage Goals

**Target coverage:**
- **Unit tests:** 90%+ (easy to achieve with mocks)
- **Integration tests:** Focus on critical paths (subscriptions, webhooks, constraints)
- **Lines:** 85%+
- **Branches:** 80%+

**Check coverage:**
```bash
npm run test:coverage
```

## Troubleshooting

### Tests timing out
- Check database connection (is MySQL running?)
- Verify test database exists
- Increase timeout in jest.config.js

### Database constraint violations
- Forgot to run `cleanDatabase()` in `beforeEach`
- Check migration status: `npm run prisma:status`

### Mocks not working
- Ensure mock is defined before importing module
- Use `jest.clearAllMocks()` in `afterEach`

### Slow tests
- Profile with `npm test -- --verbose`
- Check for accidental real API calls (should all be mocked)
- Verify not running migrations on every test

## Example Test File

```javascript
const BillingService = require('../../backend/src/billingService');
const { TEST_CUSTOMERS, TILLED_CUSTOMER_RESPONSE } = require('../fixtures/test-fixtures');
const { cleanDatabase } = require('./database-cleanup');

jest.mock('../../backend/src/tilledClient');

describe('BillingService Integration', () => {
  let service;

  beforeEach(async () => {
    await cleanDatabase();
    service = new BillingService();
  });

  describe('createCustomer', () => {
    it('should persist customer to database', async () => {
      mockTilledClient.createCustomer.mockResolvedValue(TILLED_CUSTOMER_RESPONSE);

      const customer = await service.createCustomer(...TEST_CUSTOMERS.standard);

      expect(customer.email).toBe(TEST_CUSTOMERS.standard.email);

      // Verify in database
      const dbCustomer = await billingPrisma.billing_customers.findUnique({
        where: { id: customer.id }
      });
      expect(dbCustomer).toBeTruthy();
    });
  });
});
```

---

**Questions?** See main [START-HERE.md](../START-HERE.md) for billing module documentation.
