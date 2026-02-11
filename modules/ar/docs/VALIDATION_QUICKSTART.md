# AR Migration Validation - Quick Start Guide

## Overview

This guide provides quick commands to validate the AR migration. For complete details, see [AR_MIGRATION_VALIDATION.md](./AR_MIGRATION_VALIDATION.md).

## Prerequisites

```bash
# Ensure you're in the ar-rs directory
cd modules/ar

# Install dependencies
cargo build

# Set up PostgreSQL database
export DATABASE_URL_AR="postgresql://postgres:postgres@localhost:5434/ar_service"
```

## Quick Validation Commands

### 1. Run All Automated Tests (Recommended First Step)

```bash
./tests/run-all-validation.sh
```

This runs:
- ✅ Unit tests
- ✅ Integration tests
- ✅ E2E workflow tests
- ⊘ Data validation (skipped - needs manual setup)
- ⊘ Comparison tests (skipped - needs both services)
- ⊘ Load tests (skipped - needs Artillery)

**Expected:** 3-4 stages pass, 2-3 stages skipped (manual setup required)

### 2. Run Individual Test Suites

**Unit Tests Only:**
```bash
cargo test --lib
```

**Integration Tests:**
```bash
cargo test --test customer_tests
cargo test --test subscription_tests
cargo test --test payment_tests
cargo test --test webhook_tests
cargo test --test idempotency_test
```

**E2E Workflow Tests:**
```bash
cargo test --test e2e_workflows -- --test-threads=1
```

### 3. Manual Validation Steps

**Data Migration Validation:**
```bash
# Requires: MySQL and PostgreSQL both accessible
./tests/validate-data-migration.sh
```

**Node.js vs Rust Comparison:**
```bash
# Terminal 1: Start Node.js service
cd modules/ar/backend
npm start  # Port 3001

# Terminal 2: Start Rust service
cd modules/ar
cargo run --release  # Port 8086

# Terminal 3: Run comparison
cd modules/ar
./tests/compare-implementations.sh
```

**Load Testing:**
```bash
# Install Artillery (if not installed)
npm install -g artillery

# Start Rust service
cargo run --release

# Run load test (in another terminal)
artillery run tests/load/ar-load-test.yml
```

## Quick Health Check

**Verify Rust Service:**
```bash
# Start service
cargo run

# In another terminal, test health endpoint
curl http://localhost:8086/api/health
# Expected: {"status":"healthy","service":"ar-rs"}

# Test customer creation
curl -X POST http://localhost:8086/api/ar/customers \
  -H "Content-Type: application/json" \
  -d '{
    "email": "test@example.com",
    "name": "Test Customer",
    "external_customer_id": "ext-123"
  }'
# Expected: 201 Created with customer JSON
```

## Test Results Location

All test results are saved to:
```
modules/ar/tests/load/validation-results/
```

Files include:
- `master-validation-report-{timestamp}.md` - Overall validation summary
- `unit-tests-{timestamp}.log` - Unit test output
- `integration-tests-{timestamp}.log` - Integration test output
- `e2e-tests-{timestamp}.log` - E2E test output
- `data-validation-{timestamp}.md` - Data comparison report
- `comparison-report-{timestamp}.md` - Node.js vs Rust comparison
- `load-test-{timestamp}.html` - Artillery load test report

## Common Issues

### Database Connection Errors

```bash
# Check PostgreSQL is running
pg_isready -h localhost -p 5434

# Verify DATABASE_URL_AR is set
echo $DATABASE_URL_AR

# Run migrations
sqlx migrate run --database-url $DATABASE_URL_AR
```

### Port Already in Use

```bash
# Check what's using port 8086
lsof -i :8086

# Kill the process if needed
kill -9 <PID>
```

### Test Failures

```bash
# Run tests with verbose output
cargo test -- --nocapture

# Run specific test
cargo test test_create_customer_success -- --nocapture

# Check test database
psql $DATABASE_URL_AR -c "SELECT COUNT(*) FROM billing_customers;"
```

## Success Criteria

Before production deployment, ensure:

- ✅ All unit tests pass (100%)
- ✅ All integration tests pass (100%)
- ✅ E2E workflow tests pass (>90%)
- ✅ Data validation shows 100% integrity
- ✅ Comparison tests show zero regressions
- ✅ Load tests meet performance targets:
  - P95 < 500ms
  - P99 < 1000ms
  - Error rate < 1%
  - Throughput > 50 req/sec

## Next Steps

1. Run automated validation: `./tests/run-all-validation.sh`
2. Review the master report in `tests/load/validation-results/`
3. Run manual validation steps (data, comparison, load)
4. Fix any failing tests
5. Re-run validation until all tests pass
6. Deploy to staging and monitor for 24 hours
7. Deploy to production with gradual rollout

## Getting Help

- **Documentation:** See `docs/AR_MIGRATION_VALIDATION.md`
- **Idempotency:** See `docs/IDEMPOTENCY_AND_EVENTS.md`
- **Test Code:** Check `tests/` directory for examples
- **API Routes:** See `src/routes.rs` for all endpoints

---

**Quick Reference:**
- Master validation: `./tests/run-all-validation.sh`
- Unit tests: `cargo test --lib`
- Integration tests: `cargo test --test '*'`
- Health check: `curl http://localhost:8086/api/health`
