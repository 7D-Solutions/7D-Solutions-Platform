# TrashTech Pro - Billing Integration Guide

**Target:** Production deployment of `@fireproof/ar` in TrashTech Pro backend
**Critical:** Follow middleware order exactly to prevent webhook failures

---

## 1. Express Middleware Order (CRITICAL ⚠️)

**Incorrect order breaks webhook signature verification.**

### Correct Mount Pattern

```javascript
const express = require('express');
const { captureRawBody } = require('@fireproof/ar/backend/middleware');
const billingRoutes = require('@fireproof/ar/backend/routes');

const app = express();

// CRITICAL: Webhook route MUST capture raw body BEFORE JSON parsing
app.use(
  '/api/billing/webhooks',
  captureRawBody,           // ← FIRST: Capture raw body from stream
  express.json(),           // ← SECOND: Parse JSON
  billingRoutes             // ← THIRD: Route handler
);

// Regular billing routes (no raw body needed)
app.use(
  '/api/billing',
  express.json(),
  rejectSensitiveData,      // Optional but recommended
  billingRoutes
);
```

### Why This Order Matters

1. **captureRawBody** reads the stream and stores it in `req.rawBody`
2. **express.json()** parses the body into `req.body`
3. Signature verification uses `req.rawBody` (not parsed JSON)
4. If JSON parsing happens first, the stream is consumed and rawBody is empty

**Test It:**
```bash
curl -X POST http://localhost:3000/api/billing/webhooks/trashtech \
  -H "Content-Type: application/json" \
  -H "payments-signature: t=123,v1=abc" \
  -d '{"test":"event"}'

# Should return 401 (invalid signature), NOT 500 (missing rawBody)
```

---

## 2. Environment Variables (Required)

### Billing Database
```bash
DATABASE_URL_BILLING="mysql://billing_user:password@host:3306/billing_db"
```

**Production Notes:**
- Separate from main app database
- Use dedicated read/write user
- Enable SSL if supported by host

### Tilled Credentials (TrashTech)
```bash
TILLED_SECRET_KEY_TRASHTECH="sk_prod_xxxxx"          # API secret key
TILLED_ACCOUNT_ID_TRASHTECH="acct_xxxxx"             # Merchant account ID
TILLED_WEBHOOK_SECRET_TRASHTECH="whsec_xxxxx"        # Webhook signing secret
TILLED_SANDBOX="false"                                # MUST be explicit
```

**Where to Get These:**
1. Log into Tilled dashboard
2. Navigate to: Settings → API Keys
3. Note: Production keys are different from sandbox keys

### Verification Script
```bash
# Run this before deployment
node -e "
const required = [
  'DATABASE_URL_BILLING',
  'TILLED_SECRET_KEY_TRASHTECH',
  'TILLED_ACCOUNT_ID_TRASHTECH',
  'TILLED_WEBHOOK_SECRET_TRASHTECH',
  'TILLED_SANDBOX'
];

const missing = required.filter(key => !process.env[key]);
if (missing.length > 0) {
  console.error('❌ Missing env vars:', missing);
  process.exit(1);
}
console.log('✅ All billing env vars present');
"
```

---

## 3. Database Schema Verification

### Run Migration
```bash
cd packages/billing
npx prisma migrate deploy --schema=./prisma/schema.prisma
```

### Verify Critical Constraints
```sql
-- Unique constraints (prevent duplicates)
SELECT CONSTRAINT_NAME, CONSTRAINT_TYPE, TABLE_NAME
FROM INFORMATION_SCHEMA.TABLE_CONSTRAINTS
WHERE TABLE_SCHEMA = 'billing_db'
  AND TABLE_NAME IN ('billing_customers', 'billing_subscriptions', 'billing_webhooks')
  AND CONSTRAINT_TYPE = 'UNIQUE';

-- Expected results:
-- billing_customers.tilled_customer_id (UNIQUE)
-- billing_customers.(app_id, external_customer_id) (UNIQUE)
-- billing_subscriptions.tilled_subscription_id (UNIQUE)
-- billing_webhooks.event_id (UNIQUE)
```

### Verify Indexes (Performance)
```sql
-- Indexes for hot query paths
SELECT TABLE_NAME, INDEX_NAME, COLUMN_NAME
FROM INFORMATION_SCHEMA.STATISTICS
WHERE TABLE_SCHEMA = 'billing_db'
  AND TABLE_NAME IN ('billing_customers', 'billing_subscriptions')
ORDER BY TABLE_NAME, INDEX_NAME, SEQ_IN_INDEX;

-- Expected indexes:
-- billing_customers: idx_app_id, idx_email
-- billing_subscriptions: idx_app_id, idx_billing_customer_id, idx_status, idx_plan_id
```

---

## 4. Tilled Webhook Configuration

### Setup in Tilled Dashboard

1. Navigate to: **Settings → Webhooks**
2. Create new endpoint: `https://your-domain.com/api/billing/webhooks/trashtech`
3. Select events:
   - `subscription.created`
   - `subscription.updated`
   - `subscription.canceled`
   - `subscription.deleted`
   - `charge.failed`
   - `invoice.payment_failed`
   - `payment_intent.payment_failed`

4. Copy webhook signing secret → `TILLED_WEBHOOK_SECRET_TRASHTECH`

### Test Webhook
```bash
# From Tilled dashboard, use "Send test webhook" button
# Your app should return:
# 200 OK with {"received": true, "duplicate": false}
```

### Webhook Reliability Rules

**DO:**
- Return 200 immediately (< 1 second)
- Log failures but don't block response
- Use database idempotency (insert-first pattern)

**DON'T:**
- Call external APIs in webhook handler
- Retry failed operations synchronously
- Return 500 for duplicate events (return 200 + duplicate: true)

---

## 5. Health Check Integration

### Add to Deployment Pipeline
```bash
# After deployment, verify health
curl https://your-domain.com/api/billing/health?app_id=trashtech

# Expected 200 response:
{
  "timestamp": "2026-01-23T...",
  "app_id": "trashtech",
  "database": { "status": "healthy", "error": null },
  "tilled_config": {
    "status": "healthy",
    "error": null,
    "sandbox_mode": false
  },
  "overall_status": "healthy"
}
```

### Add to Monitoring
- **URL:** `GET /api/billing/health?app_id=trashtech`
- **Success:** 200 status + `overall_status: "healthy"`
- **Alert on:** 503 status or any component showing `"unhealthy"`
- **Frequency:** Every 5 minutes

---

## 6. Logging Configuration

### Required Log Fields (for filtering)

Every billing log should include:
```javascript
{
  service: 'billing',
  app_id: 'trashtech',
  // ... event-specific fields
}
```

### Critical Logs to Monitor

**High Priority (page on-call):**
```
"Failed to sync customer update to Tilled" + divergence_risk: "high"
```
→ Email changes failed - needs manual Tilled update

**Medium Priority (review daily):**
```
"Payment failure detected"
```
→ Customer subscription may move to past_due

**Low Priority (weekly review):**
```
"Unhandled webhook event"
```
→ New Tilled event type, may need handler

### Log Aggregation Setup

**If using Datadog/NewRelic/etc:**
```javascript
// Tag all billing logs
logger.info('message', {
  service: 'billing',
  app_id: 'trashtech',
  environment: process.env.NODE_ENV
});
```

**If using stdout (Docker/Railway):**
```javascript
// Use JSON format for parsing
console.log(JSON.stringify({
  level: 'info',
  service: 'billing',
  message: 'Event processed',
  ...details
}));
```

---

## 7. TrashTech UI Integration Patterns

### Check Active Subscription
```javascript
// Frontend → Backend API → Billing module
const response = await fetch(
  `/api/billing/subscriptions?app_id=trashtech&billing_customer_id=${customerId}`
);
const subscriptions = await response.json();
const active = subscriptions.find(sub => sub.status === 'active');

if (active) {
  // Show "Manage Subscription" button
} else {
  // Show "Subscribe to Pro" button
}
```

### Display Current Plan
```javascript
const subscription = /* from above */;
return (
  <div>
    <h3>{subscription.plan_name}</h3>
    <p>${(subscription.price_cents / 100).toFixed(2)} / {subscription.interval_unit}</p>
    <p>Next billing: {new Date(subscription.current_period_end).toLocaleDateString()}</p>
  </div>
);
```

### Handle Plan Changes

**Monthly → Annual (requires cancel + create):**
```javascript
// 1. Cancel existing subscription
await fetch(`/api/billing/subscriptions/${currentSub.id}`, {
  method: 'DELETE'
});

// 2. Create new subscription with annual billing
await fetch('/api/billing/subscriptions', {
  method: 'POST',
  body: JSON.stringify({
    billing_customer_id: customerId,
    payment_method_id: paymentMethodId,
    plan_id: 'pro-annual',
    plan_name: 'Pro Annual',
    price_cents: 99900,  // $999/year
    interval_unit: 'year',
    interval_count: 1
  })
});
```

**Price Change (same billing cycle):**
```javascript
// Update price for next cycle
await fetch(`/api/billing/subscriptions/${subscription.id}`, {
  method: 'PUT',
  body: JSON.stringify({
    app_id: 'trashtech',
    price_cents: 12900  // $129/month (was $99)
  })
});

// NOTE: Takes effect on next billing cycle, not immediately
```

### Status Display Logic
```javascript
function getSubscriptionStatus(subscription) {
  switch (subscription.status) {
    case 'active':
      return { text: 'Active', color: 'green' };
    case 'past_due':
      return { text: 'Payment Failed', color: 'red', action: 'Update Payment Method' };
    case 'canceled':
      return { text: 'Canceled', color: 'gray', action: 'Resubscribe' };
    case 'trialing':
      return { text: `Trial (ends ${formatDate(subscription.current_period_end)})`, color: 'blue' };
    default:
      return { text: subscription.status, color: 'gray' };
  }
}
```

---

## 8. Production Deployment Checklist

### Pre-Deployment
- [ ] All environment variables set (verify with script)
- [ ] Database migration applied
- [ ] Tilled webhook endpoint configured
- [ ] Middleware mount order correct (webhook rawBody first)

### Deployment
- [ ] Deploy application
- [ ] Run health check: `curl /api/billing/health?app_id=trashtech`
- [ ] Verify database connectivity
- [ ] Verify Tilled credentials loaded

### Post-Deployment
- [ ] Send test webhook from Tilled dashboard (verify 200 response)
- [ ] Create test subscription in staging/sandbox
- [ ] Monitor logs for 15 minutes (watch for errors)
- [ ] Verify billing logs are aggregated correctly

### Week 1 Monitoring
- [ ] Daily review of "Payment failure detected" logs
- [ ] Daily review of "Failed to sync" logs with high divergence_risk
- [ ] Weekly review of unhandled webhook events
- [ ] Compare subscription counts: local DB vs Tilled dashboard

---

## 9. Troubleshooting Guide

### Issue: Webhook returns 500 "Missing rawBody"

**Cause:** Middleware order wrong - JSON parsing happened before rawBody capture

**Fix:**
```javascript
// WRONG ORDER:
app.use('/api/billing/webhooks', express.json(), billingRoutes);

// CORRECT ORDER:
app.use('/api/billing/webhooks', captureRawBody, express.json(), billingRoutes);
```

---

### Issue: Webhook returns 401 "Invalid signature"

**Possible Causes:**

1. **Wrong webhook secret:**
   - Verify `TILLED_WEBHOOK_SECRET_TRASHTECH` matches Tilled dashboard
   - Check for trailing spaces in env var

2. **Webhook endpoint mismatch:**
   - Tilled configured: `/webhooks/trashtech`
   - App expects: `/api/billing/webhooks/trashtech`
   - Must match exactly

3. **Signature tolerance expired:**
   - Default 5 minute window
   - Check server time sync (NTP)

**Debug:**
```javascript
// Add temporary logging in processWebhook
logger.info('Webhook signature debug', {
  received_signature: signature,
  expected_secret: process.env.TILLED_WEBHOOK_SECRET_TRASHTECH?.slice(0, 10) + '...',
  raw_body_length: rawBody?.length,
  timestamp_diff: Math.abs(Date.now() / 1000 - extractedTimestamp)
});
```

---

### Issue: Health check shows "unhealthy" database

**Cause:** Wrong `DATABASE_URL_BILLING` or network issue

**Debug:**
```bash
# Test direct connection
mysql -h host -u user -p billing_db -e "SELECT 1;"

# Check URL format
echo $DATABASE_URL_BILLING
# Should be: mysql://user:pass@host:port/database
```

---

### Issue: Subscription status stuck (not updating)

**Cause:** Webhooks not reaching app or being rejected

**Check:**
1. Tilled webhook logs (dashboard → Webhooks → View logs)
2. Your app logs (search for `event_id` from Tilled)
3. Network firewall rules (allow Tilled IPs)

**Manually trigger status sync:**
```sql
-- Find subscription in Tilled dashboard, note current status
-- Update local DB to match:
UPDATE billing_subscriptions
SET status = 'past_due', updated_at = NOW()
WHERE tilled_subscription_id = 'sub_xxxxx';
```

---

## 10. Security Checklist

### PCI Compliance
- [ ] Never log full card numbers
- [ ] Never store CVV
- [ ] Never pass card data through your server
- [ ] Use Tilled.js for client-side tokenization
- [ ] `rejectSensitiveData` middleware active on all PUT/POST routes

### Access Control
- [ ] Health check endpoint is admin-only (add auth)
- [ ] Webhook endpoint has no auth (signature verification only)
- [ ] All other endpoints require authentication
- [ ] All endpoints enforce `app_id` scoping

### Secrets Management
- [ ] Tilled keys stored in secrets manager (not .env files)
- [ ] Rotate keys quarterly
- [ ] Different keys for sandbox vs production
- [ ] Webhook secrets never logged

---

## 11. Performance Baselines

### Expected Response Times
- GET /customers/:id: < 50ms
- GET /subscriptions (list): < 100ms
- POST /subscriptions: < 500ms (calls Tilled)
- POST /webhooks: < 100ms

### Database Query Counts
- Single customer fetch: 1 query
- Subscription list (with customer): 1 query (with join)
- Subscription create: 3 queries (customer fetch, insert, Tilled call)

### Monitor These Metrics
- Webhook processing time (p95 < 200ms)
- Tilled API error rate (< 0.1%)
- Database connection pool utilization
- Failed payment rate

---

## 12. Rollback Plan

### If Production Issues Occur

**Option 1: Disable billing module (temporary)**
```javascript
// In main app, comment out billing routes
// app.use('/api/billing', billingRoutes);

// Add maintenance endpoint
app.get('/api/billing/*', (req, res) => {
  res.status(503).json({ error: 'Billing temporarily unavailable' });
});
```

**Option 2: Switch to sandbox mode (testing only)**
```bash
TILLED_SANDBOX=true  # Points to sandbox.tilled.com
# Use test credentials
```

**Option 3: Full rollback**
```bash
# Revert to previous deployment
# Existing subscriptions in Tilled are unaffected
# Local DB state remains consistent (idempotent webhooks)
```

### Data Integrity After Rollback
- Billing DB is append-only (no destructive operations)
- Webhooks will replay missed events when back online
- Tilled remains source of truth for subscription status

---

## Contact & Support

**Tilled Support:**
- Dashboard: https://dashboard.tilled.com
- Docs: https://docs.tilled.com
- Support: support@tilled.com

**Internal Escalation:**
- Database issues → DevOps team
- Tilled API errors → Check Tilled status page
- Payment failures → Customer success team

---

**Last Updated:** 2026-01-23
**Module Version:** 1.0.0
**Status:** Production Ready
