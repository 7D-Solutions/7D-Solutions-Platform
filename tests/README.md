# AR Rust Backend Integration Tests

This directory contains integration tests for the AR (Accounts Receivable) Rust backend.

## Test Structure

```
tests/
├── common/
│   └── mod.rs          # Shared test utilities and helpers
├── customer_tests.rs   # Customer CRUD operations (8 tests)
├── subscription_tests.rs # Subscription lifecycle (8 tests)
├── payment_tests.rs    # Charges and refunds (11 tests)
├── webhook_tests.rs    # Webhook handling and idempotency (10 tests)
└── README.md           # This file
```

## Test Coverage

### Customer Tests (8 tests)
1. ✓ Create customer with valid data
2. ✓ Create customer with duplicate email → Error
3. ✓ Create customer with missing email → Error
4. ✓ Get customer by ID
5. ✓ Get customer with invalid ID → 404
6. ✓ List customers with pagination
7. ✓ Update customer
8. ✓ List customers by external_customer_id

### Subscription Tests (8 tests)
1. ✓ Create subscription for customer
2. ✓ Create subscription with invalid customer ID → Error
3. ✓ Get subscription by ID
4. ✓ Cancel subscription with cancel_at_period_end=true
5. ✓ Cancel subscription immediately (cancel_at_period_end=false)
6. ✓ List subscriptions by customer
7. ✓ List subscriptions by status
8. ✓ Update subscription price

### Payment Tests (11 tests)

#### Charge Tests (5 tests)
1. ✓ Create charge for customer
2. ✓ Create charge with invalid amount (negative) → Error
3. ✓ Create charge with zero amount → Error
4. ✓ Get charge by ID
5. ✓ List charges by customer

#### Refund Tests (6 tests)
6. ✓ Create refund for full charge amount
7. ✓ Create refund for partial charge amount
8. ✓ Create refund exceeding charge amount → Error
9. ✓ Get refund by ID
10. ✓ List refunds by customer
11. ✓ List refunds by charge ID

### Webhook Tests (10 tests)
1. ✓ Receive valid webhook with correct signature
2. ✓ Reject webhook with invalid signature
3. ✓ Handle duplicate event_id (idempotency)
4. ✓ List webhooks by event type
5. ✓ List webhooks by status
6. ✓ Get webhook by ID
7. ✓ Replay failed webhook
8. ✓ Replay processed webhook without force flag → Error
9. ✓ Replay processed webhook with force flag
10. ✓ Process out-of-order webhook events

## Test Helpers (common/mod.rs)

### Database Setup
- `setup_pool()` - Creates test database connection pool with migrations
- `teardown_pool()` - Closes pool and releases connections

### Test Data Generation
- `unique_email()` - Generate unique test email
- `unique_external_id()` - Generate unique external customer ID
- `unique_plan_id()` - Generate unique plan ID
- `unique_reference_id()` - Generate unique reference ID

### Test Data Seeding
- `seed_customer()` - Create test customer
- `seed_subscription()` - Create test subscription
- `seed_charge()` - Create test charge
- `seed_webhook()` - Create test webhook

### Test Data Cleanup
- `cleanup_customers()` - Delete test customers and related records
- `cleanup_webhooks()` - Delete test webhooks

### Utilities
- `app()` - Build test router with database state
- `body_json()` - Parse response body as JSON

## Running Tests

### Run all tests
```bash
cargo test --tests
```

### Run specific test file
```bash
cargo test --test customer_tests
cargo test --test subscription_tests
cargo test --test payment_tests
cargo test --test webhook_tests
```

### Run single test
```bash
cargo test test_create_customer_success -- --exact
```

### Run with output
```bash
cargo test --test customer_tests -- --nocapture
```

## Test Isolation

All tests use `#[serial]` attribute from `serial_test` crate to ensure:
- Tests run sequentially (not in parallel)
- No database conflicts between tests
- Consistent test results

## Database Configuration

Tests require `DATABASE_URL_AR` environment variable:
```
DATABASE_URL_AR="postgresql://ar_user:ar_pass@localhost:5436/ar_db"
```

Create `.env` file in `packages/ar-rs/` directory with this variable.

## Test Patterns

### Standard Test Structure
```rust
#[tokio::test]
#[serial]
async fn test_operation_scenario() {
    let pool = common::setup_pool().await;
    let app = common::app(&pool);

    // Setup test data
    let (customer_id, _, _) = common::seed_customer(&pool, APP_ID).await;

    // Make API request
    let response = app.oneshot(request).await.unwrap();

    // Assert response
    assert_eq!(response.status(), StatusCode::OK);

    // Cleanup
    common::cleanup_customers(&pool, &[customer_id]).await;
    common::teardown_pool(pool).await;
}
```

## Mock Strategy

- **Tilled API**: Tests do NOT hit real Tilled payment processor
- **Database**: Tests use real PostgreSQL database (isolated test database)
- **Webhooks**: Tests generate valid HMAC signatures for webhook verification

## Current Status

**Total Tests**: 37 integration tests across 4 test files

**Test Status**:
- ✅ All tests compile successfully
- ⚠️  Some tests fail due to incomplete route implementations
- 🔧 Tests are ready for route implementation work

**Known Issues**:
1. Some routes return 404 (not yet implemented)
2. App ID mismatch (routes use "default_app", tests use "test-app")
3. Some validation logic not yet implemented

These tests follow the **Test-Driven Development (TDD)** pattern - tests are written first, then implementations follow to make tests pass.
