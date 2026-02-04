# Phase 1 Integration Tests Setup

## Prerequisites

The Phase 1 integration tests require the test database to have the Phase 1 schema migrations applied.

## Setup Steps

### 1. Apply Migration to Test Database

Run the following command to apply the Phase 1 schema migration to the test database:

```bash
cd packages/billing
DATABASE_URL_BILLING="mysql://billing_test:testpass@localhost:3309/billing_test" \
  npx prisma migrate deploy --schema=./prisma/schema.prisma
```

**Note:** If you get a "database schema is not empty" error, you need to reset the test database first:

```bash
# WARNING: This will DELETE ALL DATA in the billing_test database
DATABASE_URL_BILLING="mysql://billing_test:testpass@localhost:3309/billing_test" \
  npx prisma migrate reset --force --skip-seed --schema=./prisma/schema.prisma
```

### 2. Run the Tests

Once the migration is applied, run the Phase 1 integration tests:

```bash
npm test tests/integration/phase1-routes.test.js
```

## What the Tests Cover

The Phase 1 integration tests cover:

1. **GET /api/billing/state** - Billing snapshot endpoint
   - Returns composed state with customer, subscription, payment method, access, and entitlements
   - Handles missing customers with 404

2. **Payment Methods CRUD**
   - GET /api/billing/payment-methods - List payment methods
   - POST /api/billing/payment-methods - Add payment method
   - PUT /api/billing/payment-methods/:id/default - Set default
   - DELETE /api/billing/payment-methods/:id - Soft delete

3. **Subscription Lifecycle**
   - DELETE /api/billing/subscriptions/:id with at_period_end=true - Cancel at period end
   - DELETE /api/billing/subscriptions/:id with at_period_end=false - Immediate cancel
   - POST /api/billing/subscriptions/change-cycle - Change billing cycle

## Database Safety

The test database is configured in `tests/setup.js` to use:
- Database: `billing_test` (not `billing_db_sandbox`)
- Port: 3309 (trashtech-mysql container)
- User: `billing_test`
- Password: `testpass`

This ensures tests never accidentally run against production data.
