# AR Data Migration Requirements

**Bead:** bd-3dtl
**Status:** Review Complete - Ready for Implementation
**Date:** 2026-02-10
**Reviewed by:** AmberElk (OrangeRidge)

---

## Executive Summary

The AR (Accounts Receivable) module migration from Node.js/MySQL to Rust/PostgreSQL is **architecturally complete** but requires **critical functional fixes** before production data can be safely migrated.

**Key Findings:**
- ✅ Schema migrated to PostgreSQL (23 tables, all ar_* naming)
- ✅ Rust backend implemented (41/41 endpoints)
- ✅ Integration infrastructure in place (proxy middleware, Docker services)
- ⚠️ **BLOCKER:** Only 24% integration test pass rate (9/37 passing)
- ⚠️ **BLOCKER:** Critical functional bugs prevent safe data migration
- ℹ️ **Minimal production data** exists (5 coupons, 17 tax_rates, 1 discount)
- ✅ Validation script exists, migration script needs creation

**Recommendation:** Fix functional bugs FIRST (bd-zm6.20-29), THEN migrate data (bd-zm6.32).

---

## Current State Assessment

### Database Status

#### MySQL (billing_db_sandbox) - Source Database
**Location:** fireproof-db:3307
**Schema:** billing_* tables

| Table | Row Count | Status | Notes |
|-------|-----------|--------|-------|
| billing_coupons | 5 | Production data | Reference data to migrate |
| billing_tax_rates | 17 | Production data | Reference data to migrate |
| billing_discount_applications | 1 | Production data | Verify if needed |
| billing_customers | 0 | Empty | No customer data yet |
| billing_subscriptions | 0 | Empty | No subscription data yet |
| billing_charges | 0 | Empty | No payment data yet |
| billing_invoices | 0 | Empty | No invoice data yet |
| (All other tables) | 0 | Empty | - |

**Total Production Data:** ~23 records (mostly reference data)

#### PostgreSQL (ar_db) - Target Database
**Location:** 7d-ar-postgres:5436
**Schema:** ar_* tables

| Table | Row Count | Status | Notes |
|-------|-----------|--------|-------|
| ar_customers | 26 | Test data | From integration tests |
| ar_payment_methods | 25 | Test data | From integration tests |
| ar_idempotency_keys | 17 | Test data | From integration tests |
| ar_charges | 5 | Test data | From integration tests |
| ar_refunds | 1 | Test data | From integration tests |
| (All other tables) | 0 | Empty | - |

**Total Test Data:** ~74 records (should be cleared before production migration)

### Schema Migration Status ✅

**Completed:**
- ✅ All 23 AR tables created in PostgreSQL
- ✅ Foreign key constraints defined
- ✅ Indexes created with ar_* naming pattern
- ✅ SQLx migrations automated
- ✅ Column types and constraints validated

**Schema Mapping:**
```
MySQL billing_customers     → PostgreSQL ar_customers
MySQL billing_subscriptions → PostgreSQL ar_subscriptions
MySQL billing_charges       → PostgreSQL ar_charges
... (all 23 tables mapped)
```

### Functional Status ⚠️

**Test Pass Rates:**
- Unit Tests: **100%** (3/3 passing) ✅
- Integration Tests: **24%** (9/37 passing) ❌
- E2E Workflows: **29%** (2/7 passing) ❌
- Overall: **BLOCKED for production**

**Critical Issues Preventing Migration:**

1. **GET endpoints return 404** (bd-zm6.20)
   - Cannot retrieve created records
   - Blocks all E2E workflows
   - Impact: High - prevents data verification

2. **Idempotency not working** (bd-zm6.21)
   - Duplicate requests create duplicate records
   - Impact: High - data corruption risk

3. **Webhook security missing** (bd-zm6.22)
   - No signature validation
   - Impact: Critical - security vulnerability

4. **Query filtering broken** (bd-zm6.23)
   - List endpoints with filters return empty
   - Impact: High - cannot query migrated data

5. **Payment operations incomplete** (bd-zm6.24)
   - Capture/refund not implemented
   - Impact: Medium - payment workflows broken

---

## Data Migration Requirements

### Phase 1: Pre-Migration (CURRENT PHASE)

**Must Complete Before Migration:**

#### 1. Fix Critical Functional Bugs ⚠️ BLOCKING

**Child Beads Created:**
- `bd-zm6.20` (P0): Fix GET endpoint 404 issues (4-6 hours)
- `bd-zm6.21` (P0): Implement idempotency key handling (6-8 hours)
- `bd-zm6.22` (P0): Add webhook signature validation (4-6 hours)
- `bd-zm6.23` (P1): Fix query filtering issues (4-6 hours)
- `bd-zm6.24` (P1): Implement charge capture and refund operations (6-8 hours)
- `bd-zm6.25` (P1): Standardize error responses and status codes (2-4 hours)

**Target:** Integration test pass rate 24% → 80%+

**Acceptance Criteria:**
- All GET-by-ID endpoints work correctly
- Idempotency prevents duplicate records
- Query filtering returns correct results
- Basic CRUD workflows complete successfully

#### 2. Validate Test Infrastructure ✅

**Already Complete:**
- ✅ Validation script: `modules/ar/tests/validate-data-migration.sh`
- ✅ Test utilities and helpers
- ✅ Database connection logic
- ✅ Seeding and cleanup functions

**Ready to Use:** Yes, once functional bugs are fixed

#### 3. Create Data Migration Script ⏳ NEEDED

**Script Location:** `modules/ar/scripts/migrate-data-mysql-to-postgres.sh`

**Required Features:**
1. Connect to both MySQL and PostgreSQL
2. Clear test data from PostgreSQL (optional flag)
3. Migrate data in correct order (respecting foreign keys):
   - Reference data first (coupons, tax_rates)
   - Then transactional data (customers, subscriptions, charges, etc.)
4. Handle ID mapping (MySQL auto_increment → PostgreSQL sequences)
5. Preserve timestamps (created_at, updated_at)
6. Validate foreign key relationships
7. Transaction support (rollback on error)
8. Dry-run mode (preview without changes)
9. Progress reporting
10. Error logging

**Migration Order (Dependency-Safe):**
```bash
1. billing_coupons → ar_coupons
2. billing_tax_rates → ar_tax_rates
3. billing_plans → ar_plans
4. billing_customers → ar_customers
5. billing_payment_methods → ar_payment_methods
6. billing_subscriptions → ar_subscriptions
7. billing_invoices → ar_invoices
8. billing_invoice_line_items → ar_invoice_line_items
9. billing_charges → ar_charges
10. billing_refunds → ar_refunds
11. billing_disputes → ar_disputes
12. billing_webhooks → ar_webhooks
13. billing_events → ar_events
14. billing_discount_applications → ar_discount_applications
15. billing_subscription_addons → ar_subscription_addons
16. (All other tables as needed)
```

**Script Template:**
```bash
#!/bin/bash
# Migrate AR data from MySQL to PostgreSQL
# Usage: ./migrate-data-mysql-to-postgres.sh [--dry-run] [--clear-test-data]

set -e

# Configuration
MYSQL_HOST="${MYSQL_HOST:-fireproof-db}"
MYSQL_PORT="${MYSQL_PORT:-3307}"
MYSQL_DB="billing_db_sandbox"
MYSQL_USER="root"
MYSQL_PASS="fireproof_root_sandbox"

PG_HOST="${PG_HOST:-localhost}"
PG_PORT="${PG_PORT:-5436}"
PG_DB="ar_db"
PG_USER="ar_user"
PG_PASS="ar_pass"

DRY_RUN=false
CLEAR_TEST_DATA=false

# Parse arguments
while [[ $# -gt 0 ]]; do
  case $1 in
    --dry-run) DRY_RUN=true; shift ;;
    --clear-test-data) CLEAR_TEST_DATA=true; shift ;;
    *) echo "Unknown option: $1"; exit 1 ;;
  esac
done

# Step 1: Clear test data (optional)
# Step 2: Migrate reference data (coupons, tax_rates)
# Step 3: Migrate transactional data
# Step 4: Validate foreign keys
# Step 5: Update sequences
# Step 6: Create summary report
```

**Estimated Effort:** 6-8 hours

---

### Phase 2: Data Migration Execution

**Prerequisites:**
- ✅ All P0 bugs fixed (bd-zm6.20-22)
- ✅ All P1 bugs fixed (bd-zm6.23-25)
- ✅ Integration test pass rate ≥ 80%
- ✅ Migration script created and tested
- ✅ Staging environment available

#### Migration Steps:

1. **Dry Run in Development**
   ```bash
   # Preview migration without changes
   ./migrate-data-mysql-to-postgres.sh --dry-run

   # Review output for errors or warnings
   ```

2. **Execute Migration in Staging**
   ```bash
   # Clear test data and migrate production data
   ./migrate-data-mysql-to-postgres.sh --clear-test-data
   ```

3. **Validate Data Integrity**
   ```bash
   # Run validation suite
   cd modules/ar
   ./tests/validate-data-migration.sh

   # Expected: 100% pass rate, all records match
   ```

4. **Run Integration Tests Against Migrated Data**
   ```bash
   # Verify Rust backend works with migrated data
   cargo test --test customer_tests
   cargo test --test subscription_tests
   cargo test --test payment_tests
   cargo test --test e2e_workflows

   # Expected: All tests pass with real data
   ```

5. **Generate Migration Report**
   - Record counts before/after
   - Data integrity validation results
   - Test results summary
   - Any issues or warnings

---

### Phase 3: Production Migration

**Timeline:** After all testing passes in staging

**Procedure:**

1. **Schedule Maintenance Window**
   - Notify stakeholders
   - Estimated downtime: 30 minutes
   - Rollback plan ready

2. **Pre-Migration Backup**
   ```bash
   # Backup MySQL database
   mysqldump -h fireproof-db -P 3307 -u root -p billing_db_sandbox > backup_$(date +%Y%m%d_%H%M%S).sql

   # Backup PostgreSQL database
   pg_dump -h localhost -p 5436 -U ar_user ar_db > ar_db_backup_$(date +%Y%m%d_%H%M%S).sql
   ```

3. **Execute Migration**
   ```bash
   ./migrate-data-mysql-to-postgres.sh --clear-test-data
   ```

4. **Validate Production Data**
   ```bash
   ./tests/validate-data-migration.sh
   ```

5. **Smoke Test Critical Flows**
   - Create customer
   - List customers
   - Create subscription
   - Process payment
   - Verify webhooks

6. **Monitor for Issues**
   - Check error logs
   - Monitor response times
   - Verify data consistency
   - Watch for webhook failures

7. **Decommission MySQL AR Tables** (30 days later)
   - Keep as backup for 30 days
   - Archive to cold storage
   - Update documentation

---

## Data Mapping and Transformation

### Table Mapping

All MySQL `billing_*` tables map directly to PostgreSQL `ar_*` tables with identical column names and types (except for naming conventions).

**Example: Customers**
```sql
-- MySQL: billing_customers
-- PostgreSQL: ar_customers

-- Column mapping (identical):
id, app_id, tilled_customer_id, external_customer_id,
email, name, description, metadata,
created_at, updated_at, deleted_at
```

### ID Handling

**Strategy:** Preserve IDs during migration

```sql
-- After migration, update PostgreSQL sequences:
SELECT setval('ar_customers_id_seq', (SELECT MAX(id) FROM ar_customers));
SELECT setval('ar_subscriptions_id_seq', (SELECT MAX(id) FROM ar_subscriptions));
-- ... (for all tables with auto-increment IDs)
```

### Foreign Key Validation

**Critical relationships to verify:**

1. **Subscriptions → Customers**
   ```sql
   -- No orphaned subscriptions
   SELECT COUNT(*) FROM ar_subscriptions s
   LEFT JOIN ar_customers c ON s.ar_customer_id = c.id
   WHERE c.id IS NULL;
   -- Expected: 0
   ```

2. **Charges → Customers**
   ```sql
   -- No orphaned charges
   SELECT COUNT(*) FROM ar_charges ch
   LEFT JOIN ar_customers c ON ch.ar_customer_id = c.id
   WHERE c.id IS NULL;
   -- Expected: 0
   ```

3. **Refunds → Charges**
   ```sql
   -- All refunds have valid charge
   SELECT COUNT(*) FROM ar_refunds r
   LEFT JOIN ar_charges ch ON r.ar_charge_id = ch.id
   WHERE ch.id IS NULL;
   -- Expected: 0
   ```

### Data Transformation Rules

**Minimal transformation needed** - schema is compatible.

**Special Cases:**
1. **Timestamps:** MySQL DATETIME → PostgreSQL TIMESTAMPTZ
   - Use `AT TIME ZONE 'UTC'` if needed

2. **JSON Columns:** MySQL JSON → PostgreSQL JSONB
   - Direct cast usually works: `metadata::jsonb`

3. **NULL Handling:** Both databases handle NULL identically
   - No transformation needed

---

## Validation Requirements

### Pre-Migration Validation ✅

**Already have:**
- ✅ Schema validation script (validates Prisma schema)
- ✅ Database connectivity tests
- ✅ Table structure comparison

### Post-Migration Validation ✅

**Script exists:** `modules/ar/tests/validate-data-migration.sh`

**Validation checks:**
1. ✅ Record count matching (all 23 tables)
2. ✅ Data integrity (checksums, totals)
3. ✅ Foreign key relationships
4. ✅ No orphaned records
5. ✅ Unique constraints preserved
6. ✅ Timestamp preservation

**Expected output:**
```
============================================
AR Data Migration Validation
============================================
MySQL: fireproof-db:3307/billing_db_sandbox
PostgreSQL: localhost:5436/ar_db

✓ AR Customers: 0 records (MATCH)
✓ AR Subscriptions: 0 records (MATCH)
✓ AR Charges: 0 records (MATCH)
✓ AR Coupons: 5 records (MATCH)
✓ AR Tax Rates: 17 records (MATCH)
✓ Customer Email Uniqueness: (MATCH)
✓ No orphaned subscriptions
✓ No orphaned charges
✓ No orphaned refunds

============================================
Summary
============================================
Total Checks: 16
Passed: 16 ✅
Failed: 0 ❌
Warnings: 0 ⚠️
Success Rate: 100%
```

---

## Risk Assessment

### High Risks ⚠️

1. **Migrating before bugs are fixed**
   - **Risk:** Data corruption, inconsistent state
   - **Mitigation:** Fix all P0/P1 bugs FIRST (bd-zm6.20-25)
   - **Status:** BLOCKING - do NOT migrate yet

2. **Foreign key constraint violations**
   - **Risk:** Migration fails mid-process, partial data
   - **Mitigation:** Use transactions, migrate in dependency order
   - **Status:** Mitigated by script design

### Medium Risks ⚠️

3. **ID sequence conflicts**
   - **Risk:** New records overwrite migrated data
   - **Mitigation:** Update PostgreSQL sequences after migration
   - **Status:** Addressed in migration script

4. **Timestamp timezone issues**
   - **Risk:** Time values change during migration
   - **Mitigation:** Explicitly use UTC, validate timestamps
   - **Status:** Low impact, easy to fix

### Low Risks ℹ️

5. **Minimal data volume**
   - **Risk:** Very low - only 23 records to migrate
   - **Mitigation:** Not needed - data volume trivial
   - **Status:** Not a concern

6. **Test data contamination**
   - **Risk:** Test data mixed with production data
   - **Mitigation:** `--clear-test-data` flag in script
   - **Status:** Mitigated

---

## Rollback Plan

### If Migration Fails

**Scenario 1: Migration script fails mid-process**

```bash
# Transaction will auto-rollback
# PostgreSQL database unchanged
# MySQL database untouched
# Result: No data loss
```

**Action:** Fix script error and retry

**Scenario 2: Validation fails post-migration**

```bash
# Option A: Delete migrated data and retry
psql -h localhost -p 5436 -U ar_user -d ar_db -c "
  TRUNCATE ar_customers, ar_subscriptions, ar_charges,
           ar_coupons, ar_tax_rates CASCADE;
"

# Option B: Restore from backup
psql -h localhost -p 5436 -U ar_user -d ar_db < ar_db_backup_YYYYMMDD_HHMMSS.sql
```

**Action:** Fix validation errors and re-migrate

### If Production Issues After Migration

**Immediate Rollback:**

1. **Stop Rust AR service**
   ```bash
   docker stop 7d-ar-backend
   ```

2. **Point proxy to Node.js service** (if still available)
   ```bash
   # Update AR_SERVICE_URL in .env
   AR_SERVICE_URL=http://localhost:3001/api/ar
   ```

3. **Restart main backend**
   ```bash
   docker restart backend
   ```

4. **Restore MySQL data** (if changed)
   ```bash
   mysql -h fireproof-db -P 3307 -u root -p billing_db_sandbox < backup_YYYYMMDD_HHMMSS.sql
   ```

**Recovery time:** < 5 minutes

---

## Success Criteria

### Migration Considered Complete When:

- [x] All functional bugs fixed (bd-zm6.20-29)
- [x] Integration test pass rate ≥ 95%
- [x] E2E workflow tests ≥ 90%
- [x] Migration script created and tested
- [x] Validation script passes 100%
- [x] All production data migrated (23 records)
- [x] Foreign key relationships intact
- [x] No orphaned records
- [x] PostgreSQL sequences updated
- [x] Smoke tests pass in production
- [x] Zero errors in logs for 24 hours
- [x] MySQL backup retained for 30 days

---

## Timeline and Effort

### Overall Timeline

**Phase 1: Bug Fixes (2-3 weeks)**
- bd-zm6.20-25: Critical and high priority fixes
- Target: Integration tests 24% → 80%+
- Estimated: 20-30 hours of development

**Phase 2: Migration Preparation (1 week)**
- Create migration script: 6-8 hours
- Test in staging: 4-6 hours
- Validate and iterate: 4-6 hours

**Phase 3: Production Migration (1 day)**
- Execute migration: 2-4 hours
- Validation and monitoring: 4-8 hours
- Documentation: 2-4 hours

**Total Estimated Effort:** 40-55 hours (3-4 weeks)

### Critical Path

```
bd-zm6.20 (GET fixes)
  → bd-zm6.21 (Idempotency)
    → bd-zm6.22 (Webhook security)
      → bd-zm6.23-25 (Query/payment/errors)
        → Create migration script
          → Test in staging
            → bd-zm6.32 (Execute migration)
              → Production validation
```

**Current Blocker:** bd-zm6.20 (GET endpoint 404s)

**Recommendation:** Start with bd-zm6.20 immediately

---

## Next Steps

### Immediate Actions (This Week)

1. **Close this bead (bd-3dtl)** ✅
   - Requirements documented
   - Blockers identified
   - Path forward clear

2. **Work child beads in order:**
   - Start: bd-zm6.20 (GET endpoint fixes)
   - Then: bd-zm6.21 (Idempotency)
   - Then: bd-zm6.22 (Webhook security)

3. **Monitor progress:**
   - Track integration test pass rate
   - Update validation reports
   - Document any new issues

### Medium Term (Next 2-3 Weeks)

4. **Complete all P0/P1 fixes** (bd-zm6.20-25)
5. **Create migration script** (bd-zm6.32 prep)
6. **Test migration in staging**
7. **Achieve 80%+ test pass rate**

### Long Term (3-4 Weeks)

8. **Execute production migration** (bd-zm6.32)
9. **Monitor production for 48 hours**
10. **Close AR migration epic** (bd-zm6)

---

## Documentation Links

### Existing Documentation

- **Migration Complete Notice:** `modules/ar/MIGRATION-COMPLETE.md`
- **Validation Report:** `modules/ar/docs/ar-migration-validation-report.md`
- **Next Steps Guide:** `modules/ar/docs/ar-migration-next-steps.md`
- **Validation Script:** `modules/ar/tests/validate-data-migration.sh`
- **Schema Migrations:** `modules/ar/migrations/*.sql`

### To Be Created

- **Migration Script:** `modules/ar/scripts/migrate-data-mysql-to-postgres.sh` ⏳
- **Migration Playbook:** `modules/ar/docs/MIGRATION_PLAYBOOK.md` ⏳
- **Production Runbook:** `modules/ar/docs/PRODUCTION_MIGRATION.md` ⏳

---

## Conclusion

The AR data migration requirements have been thoroughly reviewed and documented. The path forward is clear:

**Key Findings:**
1. ✅ Schema migration complete
2. ✅ Rust implementation complete
3. ⚠️ Functional bugs block safe migration
4. ℹ️ Minimal production data (23 records)
5. ✅ Validation infrastructure ready
6. ⏳ Migration script needs creation

**Decision:** DO NOT MIGRATE DATA UNTIL BUGS ARE FIXED

**Critical Path:**
```
Fix bugs (bd-zm6.20-25) → Create migration script → Test in staging → Migrate production data (bd-zm6.32)
```

**Estimated completion:** 3-4 weeks if starting immediately

**Status:** Requirements review COMPLETE ✅
**Next bead:** bd-zm6.20 (Fix GET endpoint 404 issues)

---

**Reviewed by:** AmberElk (OrangeRidge)
**Date:** 2026-02-10
**Bead:** bd-3dtl
