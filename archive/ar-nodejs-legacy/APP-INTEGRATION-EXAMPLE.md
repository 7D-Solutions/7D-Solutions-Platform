# Backend App Integration Example

## Correct Middleware Mounting Order

**CRITICAL:** Webhook route MUST be mounted BEFORE `express.json()` to preserve raw body.

### In `apps/backend/src/app.js`:

```javascript
const express = require('express');
const { billingRoutes, middleware } = require('@fireproof/ar');

const app = express();

// ============================================================================
// BILLING WEBHOOKS - MUST COME FIRST (before express.json())
// ============================================================================
app.use(
  '/api/billing/webhooks',
  middleware.captureRawBody,    // Captures raw body
  express.json(),                // Then parse JSON
  billingRoutes                  // Webhook handler (NO auth middleware)
);

// ============================================================================
// GLOBAL MIDDLEWARE (applies to all other routes)
// ============================================================================
app.use(express.json());         // JSON parser for all other routes
app.use(express.urlencoded({ extended: true }));

// Your auth middleware (if any)
app.use(authenticateUser);       // Sets req.user from JWT/session

// ============================================================================
// BILLING API ROUTES (after JSON parser, with auth)
// ============================================================================
app.use(
  '/api/billing',
  middleware.rejectSensitiveData,  // PCI safety check
  middleware.requireAppId({
    getAppIdFromAuth: (req) => req.user?.app_id  // Optional: validate app_id
  }),
  billingRoutes
);

// ============================================================================
// OTHER APP ROUTES
// ============================================================================
app.use('/api/customers', customerRoutes);
app.use('/api/routes', routeRoutes);
// ... etc

// ============================================================================
// ERROR HANDLER (MUST BE LAST)
// ============================================================================
app.use(middleware.handleBillingError);  // Centralized error handling
```

## Why This Order Matters

### ❌ WRONG (will break webhook signature verification):
```javascript
app.use(express.json());  // Parses body too early
app.use('/api/billing/webhooks', billingRoutes);  // Raw body lost!
```

### ✅ CORRECT:
```javascript
app.use('/api/billing/webhooks', captureRawBody, express.json(), billingRoutes);
app.use(express.json());  // For all other routes
app.use('/api/billing', billingRoutes);
app.use(middleware.handleBillingError);  // Error handler MUST be last
```

## Webhook URL Structure

For Tilled dashboard configuration:
- **TrashTech:** `https://yourdomain.com/api/billing/webhooks/trashtech`
- **Apping:** `https://yourdomain.com/api/billing/webhooks/apping`
- **Testing:** `http://localhost:3000/api/billing/webhooks/trashtech`

## Environment Variables Check

```bash
# apps/backend/.env

# Tilled - TrashTech
TILLED_SECRET_KEY_TRASHTECH=sk_test_xxxxxxxxxxxx
TILLED_ACCOUNT_ID_TRASHTECH=acct_xxxxxxxxxxxx
TILLED_WEBHOOK_SECRET_TRASHTECH=whsec_xxxxxxxxxxxx

# Global
TILLED_SANDBOX=true

# Database
DATABASE_URL=mysql://user:pass@localhost:3306/dbname
```

## Testing the Integration

### 1. Start server
```bash
cd apps/backend
npm run dev
```

### 2. Test webhook endpoint
```bash
# Should return 401 (missing signature) - proves endpoint is reachable
curl -X POST http://localhost:3000/api/billing/webhooks/trashtech \
  -H "Content-Type: application/json" \
  -d '{"id": "evt_test", "type": "test"}'
```

### 3. Test customer creation
```bash
curl -X POST http://localhost:3000/api/billing/customers \
  -H "Content-Type: application/json" \
  -d '{
    "app_id": "trashtech",
    "email": "test@example.com",
    "name": "Test Customer"
  }'
```

## Common Issues

### Issue: Webhook signature always fails
**Cause:** `express.json()` parsed body before `captureRawBody`
**Fix:** Mount webhook route BEFORE global `express.json()`

### Issue: "Missing rawBody" error
**Cause:** `captureRawBody` middleware not applied
**Fix:** Ensure webhook route includes `middleware.captureRawBody`

### Issue: Routes return 404
**Cause:** Conflicting route paths
**Fix:** Ensure `/api/billing/webhooks` comes before `/api/billing`

## Production Checklist

- [ ] Webhook route mounted BEFORE `express.json()`
- [ ] `captureRawBody` middleware applied to webhook route
- [ ] Environment variables set (test in sandbox first)
- [ ] Webhook URL configured in Tilled dashboard
- [ ] Test webhook delivery from Tilled dashboard
- [ ] Verify webhook records appear in `billing_webhooks` table
- [ ] Test customer creation
- [ ] Test subscription creation (with Tilled.js on frontend)
- [ ] Test subscription cancellation
- [ ] Monitor first 5-10 real transactions

## Next: Frontend Integration

See [INTEGRATION.md](./INTEGRATION.md) for Tilled.js hosted fields setup.
