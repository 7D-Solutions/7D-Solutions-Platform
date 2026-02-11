# Billing Package - Pre-Launch Operational Checklist

**Purpose:** Complete this checklist before deploying billing package to production.
**Based on:** Production reviews (BrownIsland + HazyOwl), PRODUCTION-OPS.md, LAUNCH-HARDENING-AUDIT.md
**Target:** Production-ready deployment with zero operational gaps

---

## Phase 1: Infrastructure Setup (15-30 minutes)

### Database Configuration

- [ ] **Separate billing database provisioned**
  - Hosted separately from main application database
  - See SEPARATE-DATABASE-SETUP.md for instructions

- [ ] **DATABASE_URL_BILLING environment variable set**
  ```bash
  DATABASE_URL_BILLING="mysql://user:pass@billing-db-host:3306/billing_db?connection_limit=10"
  ```
  - ⚠️ CRITICAL: Must use separate connection pool from main DB

- [ ] **Prisma schema path pinned in all commands**
  - All scripts use: `--schema=packages/billing/prisma/schema.prisma`
  - Verify package.json scripts include schema path
  - Never run `prisma generate` without `--schema` flag

### Backup Strategy

- [ ] **6-hour automated backups configured**
  - Cron job: `0 */6 * * * /path/to/backup-billing-db.sh`
  - Retention: 7 days point-in-time, 30 days daily, 365 days monthly
  - Test script: `./scripts/backup-billing-db.sh`

- [ ] **Initial production backup taken**
  - Before first migration
  - Before first customer data
  - Verified restorable: `./scripts/restore-billing-db.sh`

### Access Control

- [ ] **Billing database access restricted**
  - Only billing service has write access
  - Read-only credentials for analytics/reporting
  - No direct database access for developers in production

- [ ] **Tilled API credentials secured**
  - Production keys stored in secret management (not .env files in repo)
  - Sandbox keys separate from production
  - Webhook signing secret secured

---

## Phase 2: Tilled Integration (30-45 minutes)

### Account Configuration

- [ ] **Tilled production account activated**
  - Account ID: `acct_PROD_xxxxxx`
  - Merchant account connected
  - Payment methods enabled (card, ACH, EFT)

- [ ] **API keys generated**
  - Public key: `pk_PROD_xxxxxx` (frontend use)
  - Secret key: `sk_PROD_xxxxxx` (backend use - NEVER expose)
  - Webhook signing secret obtained

### Webhook Setup

- [ ] **Production webhook endpoint configured in Tilled**
  - URL: `https://your-domain.com/api/billing/webhooks`
  - Events subscribed:
    - `subscription.created`, `subscription.updated`, `subscription.canceled`, `subscription.deleted`
    - `charge.failed`, `invoice.payment_failed`, `payment_intent.payment_failed`
    - `refund.created`, `refund.updated`
    - `dispute.created`, `dispute.updated`

- [ ] **Webhook signature verification tested**
  - Send test webhook from Tilled dashboard
  - Verify signature validation passes
  - Verify webhook record created in `billing_webhooks` table
  - Verify duplicate webhook idempotency (send twice, only 1 processed)

### Test Transactions

- [ ] **Sandbox end-to-end flow tested**
  - Create customer
  - Add payment method (card)
  - Create subscription
  - Process payment
  - Receive webhook
  - Cancel subscription
  - Process refund
  - Verify all database records correct

---

## Phase 3: Application Integration (20-30 minutes)

### Environment Variables

- [ ] **All required environment variables set in production**
  ```bash
  # Database
  DATABASE_URL_BILLING=mysql://...

  # Tilled Production
  TILLED_SECRET_KEY=sk_PROD_xxxxxx
  TILLED_PUBLIC_KEY=pk_PROD_xxxxxx
  TILLED_WEBHOOK_SECRET=whsec_xxxxxx
  TILLED_ACCOUNT_ID=acct_PROD_xxxxxx

  # Application
  NODE_ENV=production
  ```

### Route Mounting

- [ ] **Billing routes mounted in main Express app**
  ```javascript
  const { billingRoutes } = require('@fireproof/ar/backend');

  // CRITICAL: Mount webhook BEFORE express.json()
  app.use('/api/billing/webhooks', captureRawBody, billingRoutes);
  app.use(express.json());
  app.use('/api/billing', billingRoutes);
  ```
  - ⚠️ Webhook route MUST be before `express.json()` middleware
  - captureRawBody required for signature verification

### Middleware Chain

- [ ] **requireAppId middleware active**
  - All routes except webhooks require app_id
  - Validates app_id from params/body/query

- [ ] **rejectSensitiveData middleware active**
  - Blocks card_number, cvv, account_number fields
  - Returns 400 error on PCI violation attempts

- [ ] **Rate limiting configured**
  - Login attempts: 5 per 15 minutes per IP
  - API endpoints: 100 per 15 minutes per app_id
  - Webhook endpoint: 1000 per hour (Tilled bursts)

---

## Phase 4: Monitoring & Observability (30-45 minutes)

### Logging Configuration

- [ ] **Structured logging enabled**
  - All billing operations log with: app_id, billing_customer_id, tilled IDs
  - Sensitive data filtered (card numbers, CVV)
  - Log rotation configured (7 days retention minimum)

### Metrics Collection

- [ ] **Key metrics tracked**
  - Subscription creation success rate
  - Payment processing latency (p50, p95, p99)
  - Webhook processing time
  - Tilled API error rates
  - Database query performance (slow query log)

### Alerting Rules

- [ ] **Critical alerts configured**
  - Webhook signature validation failures (>5 in 5 min)
  - Payment processing failures (>10% failure rate)
  - Tilled sync failures (divergence_risk=true)
  - Database connection errors
  - Refund processing failures
  - Dispute notifications (immediate alert)

### Dashboards

- [ ] **Operational dashboard created**
  - Active subscriptions by status
  - Payment volume (last 24h, 7d, 30d)
  - Top error types
  - Webhook processing lag
  - Failed Tilled API calls

---

## Phase 5: Data Migration (If Applicable)

### Existing Customer Data

- [ ] **Migration plan documented**
  - Mapping: old system → billing package schema
  - Data validation rules
  - Rollback procedure

- [ ] **Test migration completed in staging**
  - Sample dataset migrated
  - All relationships preserved
  - Tilled customers synced

- [ ] **Production migration scheduled**
  - Maintenance window planned
  - Backup taken before migration
  - Migration script tested
  - Verification queries prepared

---

## Phase 6: Security Verification (15-20 minutes)

### Multi-Tenant Isolation

- [ ] **App ID scoping verified**
  - All database queries include app_id in WHERE clause
  - Composite unique constraints enforced:
    - `billing_webhooks`: (event_id, app_id)
    - `billing_disputes`: (tilled_dispute_id, app_id)
    - `billing_refunds`: (tilled_refund_id, app_id)
  - Reference: APP_ID_SCOPING_AUDIT.md

### PCI DSS Compliance

- [ ] **No card data stored**
  - Only masked last4, brand, expiry stored
  - Tilled hosted fields used for collection
  - rejectSensitiveData middleware active
  - Reference: PCI-DSS-COMPLIANCE.md

### Vulnerability Scan

- [ ] **Security fixes verified deployed**
  - Commit 84e63c5: Multi-tenant security vulnerabilities fixed
  - TOCTOU race condition fix (PaymentMethodService line 191)
  - WebhookService app_id scoping (lines 37-76)

---

## Phase 7: Operational Readiness (20-30 minutes)

### Team Training

- [ ] **Development team trained**
  - How to add new payment methods
  - How to investigate payment failures
  - How to handle refunds/disputes
  - When to check Tilled dashboard vs database

- [ ] **Support team trained**
  - Customer subscription lookup procedure
  - Payment failure troubleshooting guide
  - Refund request process
  - Escalation paths for disputes

### Runbooks Created

- [ ] **Payment failure investigation runbook**
  - Check `billing_webhooks` for payment_failed events
  - Check `billing_subscriptions` status
  - Check Tilled dashboard for failure codes
  - Customer communication templates

- [ ] **Subscription cancellation runbook**
  - Immediate vs end-of-period cancellation
  - Refund policy decision tree
  - Data retention requirements

- [ ] **Tilled sync failure runbook**
  - Query for divergence_risk=true logs
  - Manual reconciliation procedure
  - When to contact Tilled support

- [ ] **Disaster recovery runbook**
  - Database restore procedure (tested)
  - Webhook replay from Tilled
  - Reference: PRODUCTION-OPS.md lines 225-319

---

## Phase 8: Final Pre-Launch Tests (30-45 minutes)

### Automated Test Suite

- [ ] **All tests passing**
  ```bash
  cd packages/billing
  npm test  # Should show: 226/226 tests passing
  ```
  - 138 unit tests passing
  - 88 integration tests passing
  - Reference: TEST_FAILURES_ANALYSIS.md

### Production Smoke Tests

- [ ] **Create test customer in production**
  - Use test app_id
  - Verify Tilled customer created
  - Verify database record created

- [ ] **Add test payment method**
  - Use Tilled test card (if available in production account)
  - Verify payment method stored
  - Verify default payment method logic

- [ ] **Process test subscription (if safe)**
  - Create subscription for $0.01
  - Verify charge succeeds
  - Verify webhook received and processed
  - Cancel subscription
  - Verify cancellation webhook received

- [ ] **Test idempotency**
  - Send duplicate API request with same Idempotency-Key
  - Verify second request returns cached response
  - Verify only one database record created

### Load Testing

- [ ] **Webhook concurrency tested**
  - Send 10 duplicate webhooks concurrently
  - Verify only 1 processed, 9 duplicates detected
  - Reference: routes.test.js:346-413

- [ ] **API rate limits tested**
  - Verify rate limiting triggers correctly
  - Verify graceful degradation

---

## Phase 9: Launch Coordination (10-15 minutes)

### Communication

- [ ] **Stakeholders notified**
  - Engineering team: deployment time
  - Product team: feature availability
  - Support team: new billing system live
  - Finance team: revenue tracking changes

- [ ] **Launch announcement prepared**
  - Feature documentation
  - API changes (if any)
  - Known limitations

### Rollback Plan

- [ ] **Rollback procedure documented**
  - Database backup location
  - Code rollback command (git revert)
  - Tilled webhook disable procedure
  - Customer impact assessment

- [ ] **Rollback decision criteria defined**
  - Payment failure rate threshold (>15%)
  - Data integrity issues
  - Tilled API unavailability
  - Critical bug discovered

---

## Phase 10: Post-Launch Monitoring (First 48 Hours)

### Hour 1: Critical Watch

- [ ] **Monitor error logs**
  - No unexpected errors
  - All webhooks processing successfully
  - No Tilled API failures

- [ ] **Verify first customer transactions**
  - Subscription created successfully
  - Payment processed
  - Webhook received and processed

### Hour 24: System Health

- [ ] **Review metrics**
  - Payment success rate: >95%
  - Webhook processing time: <2 seconds p95
  - API response time: <500ms p95
  - No critical alerts triggered

### Hour 48: Operational Review

- [ ] **Team retrospective**
  - Issues encountered
  - Resolution times
  - Improvements identified

- [ ] **Documentation updates**
  - Update runbooks with learnings
  - Document common issues
  - Refine alerting thresholds

---

## Success Criteria

**All of the following must be true before considering launch complete:**

✅ All Phase 1-9 checklist items completed
✅ 226/226 automated tests passing
✅ Zero security vulnerabilities (verified by APP_ID_SCOPING_AUDIT.md)
✅ Production webhook endpoint receiving and processing events
✅ Team trained on operational procedures
✅ Monitoring and alerting configured
✅ Rollback plan tested and ready
✅ First 24 hours of production traffic successful (>95% success rate)

---

## References

- **Security:** APP_ID_SCOPING_AUDIT.md
- **Operations:** PRODUCTION-OPS.md
- **Testing:** TEST_FAILURES_ANALYSIS.md, TESTING-STRATEGY.md
- **Integration:** INTEGRATION.md, APP-INTEGRATION-EXAMPLE.md
- **Compliance:** PCI-DSS-COMPLIANCE.md
- **Architecture:** ARCHITECTURE-CHANGE.md

---

**Last Updated:** 2026-01-31
**Review Status:** Ready for production deployment
**Approvers:** BrownIsland (Security Audit), HazyOwl (Implementation & Testing)
