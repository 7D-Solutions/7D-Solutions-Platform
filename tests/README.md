# Billing Module Tests

Comprehensive test suite for `@fireproof/ar` with unit, integration, and optional real API tests.

## Quick Start

```bash
# Install dependencies (from packages/ar)
npm install

# Setup test database
mysql -u root -p
CREATE DATABASE billing_test;
exit;

# Set environment variable
export DATABASE_URL_BILLING="mysql://root:password@localhost:3306/billing_test"

# Run migrations
npm run prisma:migrate

# Run all tests
npm test
```

## Test Categories

### Unit Tests (Fast)
```bash
npm run test:unit
```
- Mocked dependencies
- No database or network calls
- ~5ms per test
- Run during development in watch mode

### Integration Tests (Medium)
```bash
npm run test:integration
```
- Real database (billing_test)
- Mocked Tilled API
- ~50-200ms per test
- Run before commits

### Watch Mode
```bash
npm run test:watch
```
Automatically re-run tests when files change

### Coverage
```bash
npm run test:coverage
```
Generate coverage report in `coverage/` directory

## Project Structure

```
tests/
├── setup.js                    # Global config
├── fixtures/                   # Test data
│   └── test-fixtures.js
├── helpers/                    # Utilities
│   └── index.js
├── unit/                       # Unit tests
│   ├── tilledClient.test.js
│   ├── billingService.test.js
│   └── middleware.test.js
└── integration/                # Integration tests
    ├── database-cleanup.js
    ├── billingService.real.test.js
    └── routes.test.js
```

## Test Database Setup

**CRITICAL: Use separate test database**

### 1. Create Database
```sql
CREATE DATABASE billing_test;
```

### 2. Configure Environment
```bash
# In packages/billing/.env or root .env
DATABASE_URL_BILLING="mysql://user:pass@localhost:3306/billing_test"
```

### 3. Run Migrations
```bash
npm run prisma:migrate
```

### Safety Check
Integration tests verify you're using a test database (name must contain `_test`).

## Running Specific Tests

```bash
# Single test file
npm test -- tests/unit/tilledClient.test.js

# Tests matching pattern
npm test -- --testNamePattern="should create customer"

# With coverage
npm test -- --coverage tests/unit/
```

## Writing New Tests

### Unit Test Template
```javascript
const MyModule = require('../../backend/src/myModule');

// Mock dependencies
jest.mock('../../backend/src/dependency');

describe('MyModule', () => {
  let instance;

  beforeEach(() => {
    instance = new MyModule();
    jest.clearAllMocks();
  });

  it('should do something', () => {
    // Arrange
    const input = 'test';

    // Act
    const result = instance.doSomething(input);

    // Assert
    expect(result).toBe('expected');
  });
});
```

### Integration Test Template
```javascript
const { billingPrisma } = require('../../backend/src/prisma');
const { cleanDatabase, setupIntegrationTests, teardownIntegrationTests } = require('./database-cleanup');
const { TEST_CUSTOMERS } = require('../fixtures/test-fixtures');

jest.mock('../../backend/src/tilledClient');

describe('MyModule Integration', () => {
  beforeAll(async () => {
    await setupIntegrationTests();
  });

  beforeEach(async () => {
    await cleanDatabase();
  });

  afterAll(async () => {
    await teardownIntegrationTests();
  });

  it('should persist to database', async () => {
    // Test with real database
    const result = await myService.create(TEST_CUSTOMERS.standard);

    // Verify in database
    const dbRecord = await billingPrisma.billing_customers.findUnique({
      where: { id: result.id }
    });
    expect(dbRecord).toBeTruthy();
  });
});
```

## Test Data

All test data centralized in `fixtures/test-fixtures.js`:

```javascript
const { TEST_CUSTOMERS, TILLED_CUSTOMER_RESPONSE } = require('../fixtures/test-fixtures');

it('should use fixture data', async () => {
  mockTilledClient.createCustomer.mockResolvedValue(TILLED_CUSTOMER_RESPONSE);

  const customer = await service.createCustomer(
    TEST_CUSTOMERS.standard.app_id,
    TEST_CUSTOMERS.standard.email,
    TEST_CUSTOMERS.standard.name
  );

  expect(customer.email).toBe(TEST_CUSTOMERS.standard.email);
});
```

## Troubleshooting

### Tests Timeout
- Check MySQL is running: `mysql -u root -p`
- Verify database exists: `SHOW DATABASES;`
- Check connection string in `.env`

### "Database doesn't exist" Error
```bash
mysql -u root -p
CREATE DATABASE billing_test;
npm run prisma:migrate
```

### "Must use test database" Error
Your `DATABASE_URL_BILLING` doesn't include `_test` in the name. This safety check prevents accidentally running tests against production.

### Mocks Not Working
```javascript
// Ensure mock is BEFORE import
jest.mock('../../backend/src/tilledClient');
const BillingService = require('../../backend/src/billingService');

// Clear mocks between tests
afterEach(() => {
  jest.clearAllMocks();
});
```

### Slow Tests
- Run unit tests only: `npm run test:unit`
- Profile with: `npm test -- --verbose`
- Check for accidental real API calls (all should be mocked)

## CI/CD

Tests are configured for CI with:
- Separate test database per worker
- Parallel execution for unit tests
- Sequential for integration tests (database access)

Example GitHub Actions:
```yaml
- name: Setup MySQL
  run: |
    mysql -e "CREATE DATABASE billing_test;"

- name: Run tests
  run: npm test
  env:
    DATABASE_URL_BILLING: mysql://root:root@localhost:3306/billing_test
```

## Coverage Targets

- **Lines:** 85%+
- **Branches:** 80%+
- **Functions:** 85%+

Check with:
```bash
npm run test:coverage
```

## Best Practices

✅ **Do:**
- Use fixtures for test data
- Clean database before each test
- Mock external APIs (Tilled)
- Test error paths
- Run unit tests in watch mode during development

❌ **Don't:**
- Hit real Tilled API in regular tests
- Use production database
- Skip database cleanup
- Hardcode test data inline
- Test implementation details

## Time Budgets

- Unit tests: < 5 seconds
- Integration tests: < 30 seconds
- Total: < 35 seconds

If tests exceed budget, refactor or split into smaller tests.

---

For detailed testing strategy, see [TESTING-STRATEGY.md](./TESTING-STRATEGY.md)
