# Period Close HTTP Boundary E2E Tests

## Status: ⏳ Awaiting bd-3gr (HTTP Handlers)

These tests are currently **disabled** (`#[ignore]`) until the HTTP handlers from bd-3gr are implemented.

## Test File

`boundary_e2e_http_period_close.rs` - 7 comprehensive E2E tests covering the period close workflow.

## Coverage

### ✅ Test Suite (bd-37m Acceptance Criteria)

1. **Successful validate on open period**
   - POST `/api/gl/periods/{id}/validate-close`
   - Verifies validation passes for balanced entries
   - Returns structured validation report

2. **Successful close operation**
   - POST `/api/gl/periods/{id}/close`
   - Atomically closes period with snapshot + hash
   - Verifies close_hash is generated and persisted

3. **Idempotent close repeat**
   - Multiple POST requests to `/close` endpoint
   - Verifies identical close_hash on repeated calls
   - No duplicate snapshots created

4. **Close-status reflects sealed snapshot/hash**
   - GET `/api/gl/periods/{id}/close-status`
   - Returns CloseStatus::Closed with hash
   - Hash matches database value

5. **Validate fails on already-closed period**
   - POST `/validate-close` on closed period
   - Returns `can_close=false`
   - Validation report contains `PERIOD_ALREADY_CLOSED` error

6. **Close fails on already-closed period**
   - POST `/close` on already-closed period
   - Returns existing close status (idempotent)
   - Preserves original `closed_by` and `close_hash`

7. **Performance guard (< 1s per suite)**
   - Full workflow: validate → close → status
   - Ensures response times meet Phase 12 standards

## Dependencies

### Blocking beads:
- **bd-3gr** (HTTP Handlers) - OPEN, assigned to GoldValley
  - Requires: bd-2jp (✓ CLOSED), bd-3sl (IN_PROGRESS), bd-1zp (OPEN)
- **bd-3sl** (Pre-Close Validation Engine) - IN_PROGRESS by GoldValley
- **bd-1zp** (Atomic Close Command) - OPEN, assigned to EmeraldBear

## How to Enable Tests

Once bd-3gr is merged:

1. Remove `#[ignore]` attributes from all tests in `boundary_e2e_http_period_close.rs`
2. Run the test suite:

```bash
# Run all period close E2E tests
cargo test --test boundary_e2e_http_period_close -- --test-threads=1

# Run specific test
cargo test --test boundary_e2e_http_period_close test_boundary_http_validate_close_success -- --test-threads=1 --nocapture
```

3. Verify all 7 tests pass
4. Add to CI workflow (see below)

## Running the Tests

### Prerequisites

```bash
# Start infrastructure
docker compose up -d

# Verify GL service is running
curl http://localhost:8090/health

# Verify Postgres is ready
docker compose ps | grep gl-db
```

### Environment Variables

```bash
export DATABASE_URL="postgres://gl_user:gl_pass@localhost:5438/gl_db"
export GL_SERVICE_URL="http://localhost:8090"
export DB_MAX_CONNECTIONS=2
export DB_MIN_CONNECTIONS=0
```

### Run Tests

```bash
# All period close E2E tests (serial execution)
cargo test --test boundary_e2e_http_period_close -- --test-threads=1

# Individual test
cargo test --test boundary_e2e_http_period_close test_boundary_http_close_period_success -- --test-threads=1 --nocapture

# All boundary E2E tests (includes period close when enabled)
cargo test boundary_e2e_http -- --test-threads=1
```

### Expected Output

```
running 7 tests
test test_boundary_http_validate_close_success ... ok
test test_boundary_http_close_period_success ... ok
test test_boundary_http_close_period_idempotent ... ok
test test_boundary_http_close_status_reflects_snapshot ... ok
test test_boundary_http_validate_close_fails_on_closed_period ... ok
test test_boundary_http_close_fails_on_closed_period ... ok
test test_boundary_http_period_close_performance_guard ... ok

test result: ok. 7 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
```

## CI Integration

Once tests are enabled, add to `.github/workflows/gl-e2e.yml`:

```yaml
- name: Run Period Close Boundary E2E Tests
  working-directory: modules/gl
  env:
    DATABASE_URL: postgres://gl_user:gl_pass@localhost:5438/gl_db
    GL_SERVICE_URL: http://localhost:8090
    DB_MAX_CONNECTIONS: 2
    DB_MIN_CONNECTIONS: 0
  run: |
    cargo test --test boundary_e2e_http_period_close -- --test-threads=1 --nocapture
```

## Test Infrastructure

### Singleton Pool Pattern

Tests use the Phase 12 singleton DB pool pattern to prevent resource exhaustion:

```rust
use common::get_test_pool;

#[tokio::test]
#[serial]
async fn my_test() {
    let pool = get_test_pool().await;
    // ...
}
```

### Serial Execution

All tests use `#[serial]` attribute to ensure:
- No concurrent writes to test data
- Predictable cleanup between tests
- Consistent DB connection usage

### Cleanup

Each test:
1. Creates isolated test data (unique tenant_id)
2. Performs operations
3. Cleans up test data (regardless of pass/fail)

## Troubleshooting

### Tests fail with "connection refused"

```bash
# Check GL service is running
docker compose ps gl-rs

# Check logs
docker compose logs gl-rs

# Restart if needed
docker compose restart gl-rs
```

### Tests fail with "table does not exist"

```bash
# Run migrations
cd modules/gl
sqlx migrate run --database-url postgres://gl_user:gl_pass@localhost:5438/gl_db
```

### Tests timeout or hang

```bash
# Ensure serial execution
cargo test --test boundary_e2e_http_period_close -- --test-threads=1

# Check for leaked connections
docker compose exec gl-db psql -U gl_user -d gl_db -c "SELECT count(*) FROM pg_stat_activity;"
```

## Performance Expectations

- Individual test: < 500ms
- Full suite (7 tests): < 5s
- Performance guard test: < 1s (validate + close + status)

## Architecture Notes

### Why HTTP Boundary?

Per ChatGPT guidance: "E2E for microservices means crossing the ACTUAL ingress boundary."

For the GL service:
- **Write path ingress** = NATS events (tested in `boundary_e2e_nats_posting.rs`)
- **Read path ingress** = HTTP GET endpoints (tested in trial balance, account activity, etc.)
- **Period close ingress** = HTTP POST/GET endpoints (tested here)

### Test Authenticity

These tests:
- Make real HTTP requests via `reqwest`
- Hit actual Axum router handlers
- Validate full serialization/deserialization chain
- Exercise tenant-scoped auth logic
- Verify error propagation through HTTP layer

### Idempotency Validation

The idempotency test is critical for Phase 13:
- Ensures `closed_at` field is the idempotency source of truth
- Verifies repeated close calls return identical `close_hash`
- Confirms no duplicate snapshot rows are created

## Related Documentation

- [Phase 13 Planning](../../../docs/phases/phase-13.md)
- [Period Close Contracts](../src/contracts/period_close_v1.rs)
- [Common Test Utilities](./common/mod.rs)
- [CI Workflow](.github/workflows/gl-e2e.yml)

## Contact

Questions or issues with these tests? Contact:
- **Bead**: bd-37m
- **Agent**: FuchsiaGrove
- **Coordinator**: PearlOwl
