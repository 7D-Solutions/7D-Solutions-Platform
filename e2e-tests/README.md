# End-to-End Tests

This directory contains cross-module end-to-end tests that verify the complete flow across multiple services.

## Tests Overview

- **`real_e2e.rs`** - Real NATS-based integration test with separate service processes ⭐ **RECOMMENDED**
- **`bill_run_e2e.rs`** - Legacy in-memory test with mock consumers (deprecated)

---

## Real NATS-Based E2E Test (Recommended)

The `real_e2e` test validates the complete bill run workflow using:
- **Real NATS** for event messaging (not InMemoryBus)
- **Real Postgres** databases (one per module, from docker-compose.infrastructure.yml)
- **Separate OS processes** for each service (no shared in-memory mocks)

### Quick Start

**From repo root:**
```bash
make e2e
```

This will:
1. Start infrastructure (NATS + Postgres via docker-compose)
2. Build all services in release mode
3. Run the E2E test with real services and real NATS
4. Leave infrastructure running for inspection

**To stop infrastructure:**
```bash
make e2e-infra-down
```

**To clean everything (including volumes):**
```bash
make e2e-clean
```

### Manual Run

```bash
# 1. Start infrastructure
docker compose -f docker-compose.infrastructure.yml up -d

# 2. Run the test
cd e2e-tests
cargo test --test real_e2e -- --ignored --test-threads=1 --nocapture

# 3. Stop infrastructure
docker compose -f docker-compose.infrastructure.yml down
```

### Test Flow

1. Infrastructure startup (NATS + Postgres)
2. Service builds and health checks (ar-rs, subscriptions-rs, payments-rs, notifications-rs)
3. Test data setup (customer, subscription plan, active subscription)
4. Trigger: `POST http://localhost:8087/api/bill-runs/execute`
5. Event propagation through real NATS
6. Database assertions across all modules

### Requirements

- Docker & Docker Compose
- Rust toolchain
- Available ports: 4222, 8222, 5434-5437, 8086-8089

---

## Legacy In-Memory Test (Deprecated)

The old `bill_run_e2e` test uses InMemoryBus and mock consumers. It's kept for reference but **the real_e2e test should be used instead**.

## Prerequisites (Legacy Test Only)

1. **All module databases must be running** with test schemas:
   - AR: `postgresql://postgres:postgres@localhost:5433/ar_test`
   - Subscriptions: `postgresql://postgres:postgres@localhost:5434/subscriptions_test`
   - Payments: `postgresql://postgres:postgres@localhost:5435/payments_test`
   - Notifications: `postgresql://postgres:postgres@localhost:5436/notifications_test`

2. **All modules must be running** with `BUS_TYPE=inmemory`:
   - AR: `http://localhost:8086`
   - Subscriptions: `http://localhost:8087`
   - Payments: `http://localhost:8088`
   - Notifications: `http://localhost:8089`

3. **Event consumers must be active** in each module to process cross-module events.

## Running the Tests

### With InMemoryBus (Default)

```bash
cd e2e-tests
cargo test --test bill_run_e2e
```

### With NATS (Optional)

1. Start NATS server:
   ```bash
   docker run -d --name nats -p 4222:4222 nats:2.10-alpine -js
   ```

2. Configure all modules to use NATS:
   ```bash
   export BUS_TYPE=nats
   export NATS_URL=nats://localhost:4222
   ```

3. Run tests:
   ```bash
   cargo test --test bill_run_e2e
   ```

## Test: Bill Run E2E Happy Path

**File:** `tests/bill_run_e2e.rs`

**Flow:**
1. Create AR customer and subscription
2. Trigger bill-run via Subscriptions API
3. Wait for `subscriptions.billrun.completed` event
4. Wait for `ar.payment.collection.requested` event
5. Wait for `payment.succeeded` event from Payments
6. Wait for `notification.delivery.succeeded` event
7. Assert final state:
   - AR invoice status = `paid`
   - Subscription `next_bill_date` updated
   - Payment record exists
   - Notification sent

**Expected Duration:** < 10 seconds with InMemoryBus

## Environment Variables

| Variable | Default | Description |
|----------|---------|-------------|
| `AR_DATABASE_URL` | `postgresql://postgres:postgres@localhost:5433/ar_test` | AR database connection |
| `SUBSCRIPTIONS_DATABASE_URL` | `postgresql://postgres:postgres@localhost:5434/subscriptions_test` | Subscriptions database |
| `PAYMENTS_DATABASE_URL` | `postgresql://postgres:postgres@localhost:5435/payments_test` | Payments database |
| `NOTIFICATIONS_DATABASE_URL` | `postgresql://postgres:postgres@localhost:5436/notifications_test` | Notifications database |
| `SUBSCRIPTIONS_BASE_URL` | `http://localhost:8087` | Subscriptions API base URL |
| `BUS_TYPE` | `inmemory` | Event bus type (`inmemory` or `nats`) |

## Troubleshooting

### Test times out waiting for events

- **Cause:** Event consumers not running or not subscribed to correct topics
- **Fix:** Ensure all modules are running with consumer tasks active

### "Connection refused" errors

- **Cause:** Module services not running
- **Fix:** Start all required modules before running tests

### "Database does not exist"

- **Cause:** Test databases not created
- **Fix:** Create test databases:
  ```bash
  createdb ar_test
  createdb subscriptions_test
  createdb payments_test
  createdb notifications_test
  ```

### Events not flowing between modules

- **Cause:** Mismatched event bus (some using NATS, some using InMemory)
- **Fix:** Ensure all modules use the same `BUS_TYPE` configuration

## Adding New E2E Tests

1. Create a new test file in `tests/`
2. Add it to `Cargo.toml`:
   ```toml
   [[test]]
   name = "your_test_name"
   path = "tests/your_test_name.rs"
   ```
3. Follow the pattern in `bill_run_e2e.rs`:
   - Set up all required databases
   - Create shared InMemoryBus
   - Subscribe to relevant events
   - Execute the workflow
   - Assert final state

## Design Principles

- **Shared InMemoryBus**: All modules in the test use the same bus instance for deterministic event flow
- **Event-driven assertions**: Wait for events rather than polling database state
- **Timeout safety**: All event waits have reasonable timeouts (10s default)
- **Cleanup**: Tests clean up created resources to avoid pollution
- **Idempotency**: Tests use unique IDs to allow parallel execution

## Future Enhancements

- [ ] Add E2E test for refund flow
- [ ] Add E2E test for failed payment retry
- [ ] Add E2E test for subscription cancellation
- [ ] Add E2E test for dispute handling
- [ ] Add performance benchmarks
- [ ] Add chaos testing (random failures)
