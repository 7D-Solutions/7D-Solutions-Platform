# AR Migration Validation Guide

## Overview

This document outlines the comprehensive end-to-end validation strategy for the AR (Accounts Receivable) service migration from Node.js/MySQL to Rust/PostgreSQL.

## Migration Scope

### What Was Migrated

1. **Backend Implementation**
   - **From:** Node.js/Express (`packages/ar/backend/`)
   - **To:** Rust/Axum (`packages/ar-rs/`)
   - **Lines of Code:** ~5,000 LOC migrated

2. **Database**
   - **From:** MySQL (`fireproof` database)
   - **To:** PostgreSQL (`ar_service` database)
   - **Tables Migrated:**
     - `billing_customers`
     - `billing_subscriptions`
     - `billing_charges`
     - `billing_refunds`
     - `billing_invoices`
     - `billing_payment_methods`
     - `billing_disputes`
     - `billing_webhooks`
     - `billing_events`
     - `billing_idempotency_keys`

3. **API Endpoints** (All RESTful endpoints migrated)

   **Customer Management:**
   - `POST /api/ar/customers` - Create customer
   - `GET /api/ar/customers` - List customers
   - `GET /api/ar/customers/:id` - Get customer
   - `PUT /api/ar/customers/:id` - Update customer
   - `DELETE /api/ar/customers/:id` - Delete customer

   **Subscription Management:**
   - `POST /api/ar/subscriptions` - Create subscription
   - `GET /api/ar/subscriptions` - List subscriptions
   - `GET /api/ar/subscriptions/:id` - Get subscription
   - `PUT /api/ar/subscriptions/:id` - Update subscription
   - `POST /api/ar/subscriptions/:id/cancel` - Cancel subscription

   **Payment Processing:**
   - `POST /api/ar/charges` - Create charge
   - `GET /api/ar/charges` - List charges
   - `GET /api/ar/charges/:id` - Get charge
   - `POST /api/ar/charges/:id/capture` - Capture authorized charge

   **Refund Management:**
   - `POST /api/ar/refunds` - Create refund
   - `GET /api/ar/refunds` - List refunds
   - `GET /api/ar/refunds/:id` - Get refund

   **Invoice Management:**
   - `POST /api/ar/invoices` - Create invoice
   - `GET /api/ar/invoices` - List invoices
   - `GET /api/ar/invoices/:id` - Get invoice
   - `POST /api/ar/invoices/:id/finalize` - Finalize invoice

   **Dispute Management:**
   - `GET /api/ar/disputes` - List disputes
   - `GET /api/ar/disputes/:id` - Get dispute
   - `POST /api/ar/disputes/:id/evidence` - Submit dispute evidence

   **Webhook Handling:**
   - `POST /api/ar/webhooks/tilled` - Receive Tilled webhook
   - `GET /api/ar/webhooks` - List webhooks
   - `GET /api/ar/webhooks/:id` - Get webhook
   - `POST /api/ar/webhooks/:id/replay` - Replay webhook

   **Event Logging:**
   - `GET /api/ar/events` - List events
   - `GET /api/ar/events/:id` - Get event

4. **Tilled Integration**
   - Payment processing API calls
   - Webhook event handling
   - Idempotency key management
   - Error handling and retries

## Validation Strategy

### 1. Unit Tests

**Location:** `packages/ar-rs/tests/`

**Coverage:**
- ✅ Customer operations (`customer_tests.rs`)
- ✅ Subscription lifecycle (`subscription_tests.rs`)
- ✅ Payment processing (`payment_tests.rs`)
- ✅ Webhook handling (`webhook_tests.rs`)
- ✅ Idempotency (`idempotency_test.rs`)

**Run Unit Tests:**
```bash
cd packages/ar-rs
cargo test --lib
```

**Expected Results:**
- All unit tests should pass
- Coverage should be >80% for business logic
- No database connection leaks

### 2. Integration Tests

**Location:** `packages/ar-rs/tests/`

**What's Tested:**
- Real database connections
- Full API request/response cycle
- Error handling and validation
- Database transactions and rollbacks

**Run Integration Tests:**
```bash
cd packages/ar-rs
DATABASE_URL_AR="postgresql://postgres:postgres@localhost:5434/ar_service" \
cargo test --test '*'
```

**Expected Results:**
- All integration tests pass
- Database state cleaned up after each test
- No hanging connections

### 3. End-to-End Workflow Tests

**Location:** `packages/ar-rs/tests/e2e_workflows.rs`

**Scenarios Tested:**
1. **Customer Lifecycle**: Create → Update → List → Get
2. **Subscription Flow**: Create customer → Create subscription → Update → Cancel
3. **Payment Flow**: Create customer → Create charge → Capture → Refund
4. **Invoice Flow**: Create customer → Create invoice → Finalize → Pay
5. **Webhook Flow**: Receive webhook → Process → Log event → Replay
6. **Error Recovery**: Idempotency keys and retry logic
7. **Multi-tenant Isolation**: Verify app_id separation

**Run E2E Tests:**
```bash
cd packages/ar-rs
cargo test --test e2e_workflows -- --test-threads=1
```

**Expected Results:**
- All workflows complete successfully
- Complex multi-step operations work correctly
- Error handling is graceful

### 4. Load Testing

**Location:** `packages/ar-rs/tests/load/ar-load-test.yml`

**Test Configuration:**
- **Warm-up:** 5 req/sec for 60s
- **Ramp-up:** 5→25 req/sec over 120s
- **Sustained:** 25 req/sec for 300s (5 min)
- **Peak:** 25→50 req/sec over 120s
- **Cool-down:** 50→5 req/sec over 60s

**Traffic Mix:**
- 40% Customer operations
- 30% Subscription lifecycle
- 20% Payment processing
- 10% Invoice operations
- 50% Read operations (health, lists)

**Prerequisites:**
```bash
# Install Artillery
npm install -g artillery

# Start Rust AR service
cd packages/ar-rs
cargo run --release
```

**Run Load Test:**
```bash
cd packages/ar-rs
artillery run tests/load/ar-load-test.yml
```

**Performance Targets:**
- **Error Rate:** <1%
- **P95 Response Time:** <500ms
- **P99 Response Time:** <1000ms
- **Throughput:** 50+ req/sec sustained

### 5. Comparison Testing (Node.js vs Rust)

**Location:** `packages/ar-rs/tests/compare-implementations.sh`

**Prerequisites:**
```bash
# Start Node.js AR service
cd packages/ar/backend
npm start  # Runs on port 3001

# Start Rust AR service (in another terminal)
cd packages/ar-rs
cargo run --release  # Runs on port 8086
```

**Run Comparison Test:**
```bash
cd packages/ar-rs
./tests/compare-implementations.sh
```

**What's Compared:**
- HTTP status codes (should match exactly)
- Response structure (keys should match)
- Response timing (Rust should be faster)

**Expected Results:**
- All API responses structurally identical
- Rust implementation 30-50% faster
- Zero functional regressions

### 6. Data Migration Validation

**Location:** `packages/ar-rs/tests/validate-data-migration.sh`

**Prerequisites:**
- MySQL database accessible (`fireproof-db:3307`)
- PostgreSQL database accessible (`localhost:5434`)
- Data migration completed (`packages/ar-rs/migrations/`)

**Run Data Validation:**
```bash
cd packages/ar-rs
./tests/validate-data-migration.sh
```

**Validation Checks:**

**Record Counts:**
- ✅ Billing customers
- ✅ Subscriptions
- ✅ Charges
- ✅ Refunds
- ✅ Invoices
- ✅ Payment methods
- ✅ Disputes
- ✅ Webhooks
- ✅ Events
- ✅ Idempotency keys

**Data Integrity:**
- ✅ Email uniqueness preserved
- ✅ Total charge amounts match
- ✅ Total refund amounts match
- ✅ Active subscription counts match

**Foreign Key Validation:**
- ✅ No orphaned subscriptions
- ✅ No orphaned charges
- ✅ No orphaned refunds
- ✅ All relationships intact

**Expected Results:**
- 100% record count match
- All data integrity checks pass
- Zero orphaned records

## Validation Checklist

Use this checklist to validate the complete migration:

### Pre-Migration
- [ ] Node.js AR service running and stable
- [ ] MySQL database backed up
- [ ] All existing tests passing

### Migration
- [ ] PostgreSQL database created
- [ ] Schema migrations applied
- [ ] Data migrated from MySQL to PostgreSQL
- [ ] Rust AR service builds successfully
- [ ] Environment variables configured

### Testing
- [ ] Unit tests passing (all modules)
- [ ] Integration tests passing (all endpoints)
- [ ] E2E workflow tests passing
- [ ] Load tests meet performance targets
- [ ] Comparison tests show no regressions
- [ ] Data validation shows 100% integrity

### Production Readiness
- [ ] Health checks responding
- [ ] Metrics/observability configured
- [ ] Error handling comprehensive
- [ ] Connection pooling stable
- [ ] Tilled integration working
- [ ] Idempotency keys functioning
- [ ] Webhook processing reliable

### Performance Validation
- [ ] Response times meet SLA (<500ms P95)
- [ ] Throughput adequate (50+ req/sec)
- [ ] Memory usage stable under load
- [ ] No connection leaks
- [ ] Database query performance optimized

### Security & Compliance
- [ ] API authentication working
- [ ] Authorization checks in place
- [ ] PCI compliance maintained
- [ ] Audit logging enabled
- [ ] Sensitive data encrypted

## Known Issues & Limitations

### Current Limitations
1. **Tilled Sandbox Only**: Production Tilled integration not yet tested
2. **Single App Context**: Multi-tenant app_id filtering partially implemented
3. **Webhook Signatures**: Webhook signature verification pending

### Future Enhancements
1. **Rate Limiting**: Implement per-customer rate limits
2. **Caching**: Add Redis caching for frequently accessed data
3. **Batch Operations**: Support bulk customer/subscription creation
4. **Reporting**: Add analytics and reporting endpoints

## Rollback Plan

If critical issues are discovered:

1. **Immediate Rollback:**
   ```bash
   # Stop Rust service
   pkill ar-rs

   # Restart Node.js service
   cd packages/ar/backend
   npm start
   ```

2. **Data Rollback (if needed):**
   ```bash
   # Restore MySQL backup
   mysql -h fireproof-db -P 3307 -u root -p fireproof < backup.sql
   ```

3. **Proxy Rollback:**
   ```javascript
   // In apps/backend/src/middleware/arProxy.js
   // Change proxyTarget to Node.js service
   const proxyTarget = 'http://localhost:3001';
   ```

## Success Criteria

Migration is considered successful when:

1. ✅ All automated tests pass (unit, integration, E2E)
2. ✅ Load tests meet performance targets
3. ✅ Comparison tests show zero regressions
4. ✅ Data validation shows 100% integrity
5. ✅ Production monitoring shows stable operation for 24 hours
6. ✅ Zero customer-reported issues for 48 hours

## Test Reports

Test reports are saved to:
- **Comparison Reports:** `tests/load/comparison-results/comparison-report-*.md`
- **Data Validation:** `tests/load/validation-results/data-validation-*.md`
- **Load Test Results:** Console output and Artillery HTML reports

## Monitoring

Post-migration monitoring dashboard should track:
- Request rate (req/sec)
- Response times (P50, P95, P99)
- Error rate (%)
- Database connection pool usage
- Memory usage
- Tilled API call success rate
- Webhook processing latency

## Contact

**Migration Owner:** SapphireBrook Agent
**Bead:** bd-zm6.13
**Date:** 2026-02-10
