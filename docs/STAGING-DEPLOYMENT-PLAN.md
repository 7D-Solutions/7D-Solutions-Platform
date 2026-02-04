# Staging Deployment Plan

**Phase:** Coordination of Phases 1-3 deployment to staging environment
**Date:** 2026-02-04
**Coordinator:** MistyBridge (with LavenderDog, JadeRiver)
**Status:** Planning

## Overview

All three billing module phases are complete and production-ready:

1. **Phase 1:** TaxService with jurisdiction-based tax calculations
2. **Phase 2:** DiscountService with 6 discount types and stacking rules
3. **Phase 3:** ProrationService with mid-cycle billing change calculations

The modules are integrated through the BillingService facade and follow the order:
**Proration → Discount → Tax**

## Current State

### ✅ Completed
- [x] All Phase 1-3 implementations complete
- [x] Comprehensive unit tests (45+ test cases for each service)
- [x] Integration tests for proration→discount→tax flow
- [x] API documentation and integration guides
- [x] Request validators for all endpoints
- [x] Sandbox database migrated and seeded
- [x] Generic billing module design (not trash-specific)

### 🔄 In Progress
- [ ] Staging environment configuration
- [ ] Integration testing in sandbox
- [ ] Coordination with LavenderDog for deployment

### ❌ Not Started
- [ ] Formal staging environment setup
- [ ] Deployment to staging servers
- [ ] Production deployment

## Deployment Architecture

### Database Strategy
- **Sandbox Database:** Interim staging (already configured)
- **Separate Billing Database:** `billing_db` (isolated from main app)
- **Migration Path:** Always use schema path: `--schema=packages/billing/prisma/schema.prisma`

### Environment Configuration
```bash
# Required environment variables
DATABASE_URL_BILLING="mysql://user:password@localhost:3306/billing_db"
TILLED_SECRET_KEY_TRASHTECH=sk_test_...
TILLED_ACCOUNT_ID_TRASHTECH=acct_...
TILLED_WEBHOOK_SECRET_TRASHTECH=whsec_...
TILLED_SANDBOX=true
```

## Staging Deployment Steps

### Step 1: Sandbox Integration Testing
1. **Verify sandbox database connectivity**
   ```bash
   cd packages/billing
   npx prisma db pull --schema=./prisma/schema.prisma
   ```

2. **Run full test suite**
   ```bash
   npm test
   ```

3. **Execute integration tests with real database**
   ```bash
   npm run test:integration
   ```

### Step 2: Feature Branch Merge
1. **Review and merge `feature/phase-3-proration-engine` to `main`**
   - All tests passing
   - Documentation complete
   - Code review by LavenderDog

2. **Update version numbers if needed**
   ```json
   {
     "version": "1.0.0",
     "dependencies": {
       "@prisma/client": "^5.0.0",
       "express-validator": "^7.0.0"
     }
   }
   ```

### Step 3: Staging Environment Setup
1. **Configure staging database**
   - Create `billing_db_staging` database
   - Apply migrations: `npx prisma migrate deploy --schema=...`
   - Seed with test data

2. **Deploy billing service to staging**
   - Containerize billing service
   - Deploy to staging Kubernetes/ECS
   - Configure environment variables

3. **Configure webhook endpoints**
   - Update Tilled webhook URLs to staging
   - Test webhook receipt and processing

### Step 4: Integration Verification
1. **End-to-end testing**
   - Create subscription
   - Apply discount
   - Calculate tax
   - Process proration change
   - Generate invoice

2. **Performance testing**
   - Load test with concurrent requests
   - Verify database connection pooling
   - Monitor query performance

3. **Security verification**
   - Validate input sanitization
   - Verify authentication/authorization
   - Check audit logging

## Rollback Plan

### If deployment fails:
1. **Database rollback:** Use pre-migration backup
2. **Service rollback:** Revert to previous container version
3. **Webhook replay:** Process missed webhooks from logs

### Backup procedures:
```bash
# Pre-deployment backup
./scripts/backup-billing-db.sh

# Restore if needed
./scripts/restore-billing-db.sh /backups/billing/pre-deployment.sql.gz
```

## Success Criteria

### Technical
- [ ] All tests pass in staging environment
- [ ] Database migrations apply successfully
- [ ] Webhooks processed within SLA (< 5 seconds)
- [ ] API response times < 200ms (p95)
- [ ] Error rate < 0.1%

### Business
- [ ] TrashTech can process subscription changes with proration
- [ ] Discounts apply correctly to prorated amounts
- [ ] Tax calculations match jurisdiction requirements
- [ ] Audit trail captures all billing events

## Coordination Requirements

### Team Responsibilities
- **MistyBridge:** Deployment coordination, documentation
- **LavenderDog:** Technical review, integration testing
- **JadeRiver:** Staging environment setup (if available)
- **AzureBay/CloudyCastle:** Additional testing support

### Communication Channels
- Agent mail system for coordination
- Daily status updates during deployment
- Immediate notification of any issues

## Timeline

1. **Day 1:** Sandbox integration testing completion
2. **Day 2:** Feature branch merge and code review
3. **Day 3:** Staging environment configuration
4. **Day 4:** Deployment and verification
5. **Day 5:** Monitoring and optimization

## Risks & Mitigations

| Risk | Probability | Impact | Mitigation |
|------|-------------|--------|------------|
| Database migration failure | Low | High | Pre-deployment backup, rollback script |
| Performance degradation | Medium | Medium | Load testing in staging, connection pool tuning |
| Webhook processing delays | Low | Medium | Queue monitoring, auto-scaling workers |
| Integration issues with main app | Medium | High | Comprehensive integration tests, feature flags |

## Next Actions

1. **Immediate:** Coordinate with LavenderDog for sandbox integration testing
2. **Short-term:** Merge feature branch to main after review
3. **Medium-term:** Set up formal staging environment
4. **Long-term:** Production deployment with monitoring

## References

- [PRODUCTION-OPS.md](./PRODUCTION-OPS.md) - Production operations guide
- [PRORATIONSERVICE-INTEGRATION-GUIDE.md](./PRORATIONSERVICE-INTEGRATION-GUIDE.md) - Proration integration
- [DISCOUNTSERVICE-INTEGRATION-GUIDE.md](./DISCOUNTSERVICE-INTEGRATION-GUIDE.md) - Discount integration
- [TAXSERVICE-INTEGRATION-GUIDE.md](./TAXSERVICE-INTEGRATION-GUIDE.md) - Tax integration
- [Separate Database Setup](../SEPARATE-DATABASE-SETUP.md) - Database isolation

---
**Last Updated:** 2026-02-04
**Maintainers:** MistyBridge, LavenderDog
**Status:** Planning - Ready for Coordination