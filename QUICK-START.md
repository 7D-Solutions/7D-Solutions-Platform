# Billing Module - Quick Start (30 Minutes)

Follow these steps to get the billing module running in sandbox mode.

## Step 1: Install Dependencies (2 minutes)

```bash
cd packages/ar
npm install
```

This installs:
- `@prisma/client` - Database client
- `prisma` - Migration tool
- `tilled-node` - Payment processor SDK

## Step 2: Create Billing Database (2 minutes)

```bash
# Connect to MySQL
mysql -u root -p

# Create database
CREATE DATABASE billing_db;

# Verify
SHOW DATABASES;

# Exit
exit;
```

## Step 3: Set Environment Variables (3 minutes)

Copy the example file:
```bash
cp .env.example .env
```

Edit `.env` and fill in your values:

```bash
# 1. Database connection (use your MySQL credentials)
DATABASE_URL_BILLING="mysql://user:password@localhost:3306/billing_db"

# 2. Get Tilled sandbox credentials from https://sandbox.tilled.com
TILLED_SECRET_KEY_TRASHTECH=sk_test_xxx
TILLED_ACCOUNT_ID_TRASHTECH=acct_xxx
TILLED_WEBHOOK_SECRET_TRASHTECH=whsec_xxx

# 3. Use sandbox mode
TILLED_SANDBOX=true
```

**Where to get Tilled credentials:**
1. Sign up at https://sandbox.tilled.com
2. Go to Settings → API Keys
3. Copy your credentials

## Step 4: Generate Prisma Client (1 minute)

```bash
npm run prisma:generate
```

This creates the billing Prisma client at `node_modules/.prisma/ar/`

## Step 5: Run Migrations (1 minute)

```bash
npm run prisma:migrate
```

When prompted for migration name, enter: `init`

This creates:
- `billing_customers` table
- `billing_subscriptions` table
- `billing_webhooks` table

## Step 6: Verify Setup (1 minute)

```bash
npm run verify
```

**Expected output:**
```
✅ All checks passed! Billing module is ready.

Next steps:
  1. Mount routes in your app
  2. Run sandbox tests
  3. Deploy to production
```

**If you see errors:**
- ❌ Missing env var → Check your `.env` file
- ❌ Database connection failed → Verify MySQL is running and credentials are correct
- ❌ Tables missing → Run `npm run prisma:migrate`

## Step 7: Browse Database (Optional)

```bash
npm run prisma:studio
```

Opens Prisma Studio in your browser to view/edit tables.

## Step 8: Integrate into Your App (10 minutes)

### 8.1 Install in Backend

```bash
cd ../../apps/backend
npm install
```

The billing package is already linked via workspace.

### 8.2 Mount Routes

Edit `apps/backend/src/app.js`:

```javascript
const express = require('express');
const { billingRoutes, middleware } = require('@fireproof/ar');

const app = express();

// CRITICAL: Webhook route BEFORE express.json()
app.use(
  '/api/billing/webhooks',
  middleware.captureRawBody,
  express.json(),
  billingRoutes
);

// Normal routes AFTER express.json()
app.use(express.json());

app.use(
  '/api/billing',
  middleware.rejectSensitiveData,  // PCI safety
  billingRoutes
);

// Your other routes...
app.use('/api/customers', customerRoutes);
// etc...

app.listen(3000, () => {
  console.log('Server running on http://localhost:3000');
});
```

### 8.3 Copy Environment Variables

Copy billing environment variables from `packages/billing/.env` to `apps/backend/.env`:

```bash
# Add to apps/backend/.env
DATABASE_URL_BILLING="mysql://user:password@localhost:3306/billing_db"
TILLED_SECRET_KEY_TRASHTECH=sk_test_xxx
TILLED_ACCOUNT_ID_TRASHTECH=acct_xxx
TILLED_WEBHOOK_SECRET_TRASHTECH=whsec_xxx
TILLED_SANDBOX=true
```

### 8.4 Start Backend

```bash
cd apps/backend
npm run dev
```

## Step 9: Test API Endpoint (2 minutes)

Test that billing routes are accessible:

```bash
# Should return 400 (missing fields) - proves endpoint works
curl -X POST http://localhost:3000/api/billing/customers \
  -H "Content-Type: application/json" \
  -d '{}'

# Should return 401 (missing signature) - proves webhook endpoint works
curl -X POST http://localhost:3000/api/billing/webhooks/trashtech \
  -H "Content-Type: application/json" \
  -d '{"id": "evt_test", "type": "test"}'
```

**Expected responses:**
- `/customers` → 400 "Missing required fields"
- `/webhooks` → 401 "Missing webhook signature"

Both mean the routes are working!

## Step 10: Create First Customer (5 minutes)

```bash
curl -X POST http://localhost:3000/api/billing/customers \
  -H "Content-Type: application/json" \
  -d '{
    "app_id": "trashtech",
    "email": "test@acmewaste.com",
    "name": "Acme Waste Inc",
    "external_customer_id": "1",
    "metadata": {"industry": "waste"}
  }'
```

**Expected response:**
```json
{
  "id": 1,
  "app_id": "trashtech",
  "email": "test@acmewaste.com",
  "name": "Acme Waste Inc",
  "tilled_customer_id": "cus_xxxxxxxxxx",
  "created_at": "2026-01-22T..."
}
```

**Verify in database:**
```bash
cd packages/ar
npm run prisma:studio
```

Check `billing_customers` table - you should see your customer!

**Verify in Tilled:**
- Go to https://sandbox.tilled.com
- Navigate to Customers
- Find "Acme Waste Inc"

## ✅ You're Ready!

If you got here successfully:
- ✅ Database is set up
- ✅ Migrations are run
- ✅ Routes are mounted
- ✅ API is working
- ✅ First customer created

### Next Steps

**Option A: Run Full Sandbox Tests (2-3 hours)**
Follow [SANDBOX-TEST-CHECKLIST.md](./SANDBOX-TEST-CHECKLIST.md) to test:
- Payment method collection
- Subscription creation (card)
- Subscription creation (ACH)
- Webhook processing
- Cancellation
- Error handling

**Option B: Build Frontend Form (4-6 hours)**
See [INTEGRATION.md](./INTEGRATION.md) for Tilled.js hosted fields setup.

**Option C: Deploy to Production**
See [PRODUCTION-OPS.md](./PRODUCTION-OPS.md) for deployment guide.

## Troubleshooting

### "Can't connect to database"
```bash
# Check MySQL is running
mysql -u root -p

# Verify database exists
SHOW DATABASES;

# Check DATABASE_URL_BILLING in .env
```

### "Prisma client not found"
```bash
cd packages/ar
npm run prisma:generate
```

### "Tables don't exist"
```bash
cd packages/ar
npm run prisma:migrate
```

### "Routes return 404"
Ensure webhook route is mounted BEFORE `express.json()` in app.js

### "Tilled API errors"
- Verify credentials in .env
- Check TILLED_SANDBOX=true
- Ensure account is active at sandbox.tilled.com

## Quick Reference

```bash
# Verify setup
npm run verify

# Generate Prisma client
npm run prisma:generate

# Run migrations
npm run prisma:migrate

# Check migration status
npm run prisma:status

# Open database browser
npm run prisma:studio

# Deploy to production
npm run prisma:deploy
```

## Next: Full Testing

Once quick start is complete, proceed to:
- **[SANDBOX-TEST-CHECKLIST.md](./SANDBOX-TEST-CHECKLIST.md)** - 12 comprehensive tests
- **[INTEGRATION.md](./INTEGRATION.md)** - Frontend payment form
- **[PRODUCTION-OPS.md](./PRODUCTION-OPS.md)** - Production deployment

---

**Questions?** See [START-HERE.md](./START-HERE.md) for documentation navigator.
