# Separate Database Setup Guide

The `@fireproof/ar` package has its own database, completely separate from your main application database.

## Why Separate Database?

✅ **Truly generic** - Can be used in any app without schema conflicts
✅ **Independent scaling** - Billing can grow separately
✅ **Clear boundaries** - No accidental coupling to app data
✅ **Flexible deployment** - Same DB or different DB per environment
✅ **Multi-app ready** - TrashTech, Apping, etc. share same billing infrastructure

## Database Setup Options

### Option 1: Same MySQL Instance, Different Database (Recommended for MVP)

```bash
# Create billing database
mysql -u root -p
CREATE DATABASE billing_db;
GRANT ALL ON billing_db.* TO 'youruser'@'localhost';
```

**.env:**
```bash
# Main app database
DATABASE_URL="mysql://user:pass@localhost:3306/trashtech_db"

# Billing database (separate)
DATABASE_URL_BILLING="mysql://user:pass@localhost:3306/billing_db"
```

**Benefits:**
- Simple setup
- One MySQL instance to manage
- Easy local development
- Can migrate to separate instance later

### Option 2: Completely Separate MySQL Instance

```bash
# Separate MySQL server for billing
DATABASE_URL="mysql://user:pass@localhost:3306/trashtech_db"
DATABASE_URL_BILLING="mysql://user:pass@billing-server:3306/billing_db"
```

**Benefits:**
- True isolation
- Independent scaling
- Can use different DB technology later (Postgres, etc.)

### Option 3: Shared Database (For Quick Testing Only)

```bash
# NOT RECOMMENDED for production
DATABASE_URL="mysql://user:pass@localhost:3306/main_db"
DATABASE_URL_BILLING="mysql://user:pass@localhost:3306/main_db"
```

Same database, different tables. Loses separation benefits but works for quick prototyping.

## Installation Steps

### 1. Install Dependencies

```bash
cd packages/ar
npm install
```

This installs:
- `@prisma/client` - Prisma client for billing database
- `prisma` - Prisma CLI for migrations
- `tilled-node` - Tilled payment SDK

### 2. Set Environment Variable

Add to your app's `.env`:

```bash
# Main app
DATABASE_URL="mysql://user:pass@localhost:3306/trashtech_db"

# Billing (separate database)
DATABASE_URL_BILLING="mysql://user:pass@localhost:3306/billing_db"

# Tilled credentials
TILLED_SECRET_KEY_TRASHTECH=sk_test_xxx
TILLED_ACCOUNT_ID_TRASHTECH=acct_xxx
TILLED_WEBHOOK_SECRET_TRASHTECH=whsec_xxx
TILLED_SANDBOX=true
```

### 3. Generate Prisma Client

```bash
cd packages/ar
npm run prisma:generate
```

This creates the billing Prisma client at `node_modules/.prisma/ar/`

### 4. Run Migrations

```bash
cd packages/ar
npm run prisma:migrate
```

Or manually:
```bash
npx prisma migrate dev --schema=./prisma/schema.prisma --name init
```

This creates:
- `billing_customers` table
- `billing_subscriptions` table
- `billing_webhooks` table

### 5. Verify Setup

```bash
cd packages/ar
npm run prisma:studio
```

Opens Prisma Studio to browse billing database tables.

## Integration in Your App

### apps/backend/src/app.js

```javascript
const express = require('express');
const { billingRoutes, middleware } = require('@fireproof/ar');

const app = express();

// Webhook route (BEFORE express.json())
app.use(
  '/api/billing/webhooks',
  middleware.captureRawBody,
  express.json(),
  billingRoutes
);

// Other billing routes
app.use(
  '/api/billing',
  express.json(),
  middleware.rejectSensitiveData,
  billingRoutes
);

app.listen(3000);
```

### No schema changes to main app!

Your main `apps/backend/prisma/schema.prisma` stays **unchanged**.
Billing tables live in their own database with their own schema.

## Data Linking (How Apps Reference Billing Data)

Apps link to billing data via `external_customer_id`:

```javascript
// In your TrashTech customer signup
const traschtechCustomer = await prisma.customers.create({
  data: { business_name: 'Acme Waste', email: 'acme@example.com' }
});

// Create corresponding billing customer (in separate DB)
const { BillingService } = require('@fireproof/ar');
const billingService = new BillingService();

const billingCustomer = await billingService.createCustomer(
  'trashtech',                    // app_id
  'acme@example.com',
  'Acme Waste Inc',
  traschtechCustomer.id,          // external_customer_id (your DB's FK)
  { industry: 'waste' }
);

// Store billing_customer.id in your app if needed
await prisma.customers.update({
  where: { id: traschtechCustomer.id },
  data: { billing_customer_id: billingCustomer.id }
});
```

**Key insight:** Apps store `billing_customer_id` if they need quick lookups, but it's just an integer reference across databases.

## Migration Commands Reference

**🚨 CRITICAL: Always specify `--schema` path to prevent migrating wrong database!**

```bash
# Generate Prisma client
npm run prisma:generate
# Or: npx prisma generate --schema=./prisma/schema.prisma

# Create new migration (development)
npm run prisma:migrate
# Or: npx prisma migrate dev --schema=./prisma/schema.prisma --name your_change

# Apply migrations (production)
npx prisma migrate deploy --schema=./prisma/schema.prisma

# Check migration status
npx prisma migrate status --schema=./prisma/schema.prisma

# Reset database (dev only - WARNING: deletes all data)
npx prisma migrate reset --schema=./prisma/schema.prisma

# Open Prisma Studio
npm run prisma:studio
# Or: npx prisma studio --schema=./prisma/schema.prisma
```

**Golden Rule:** If you forget `--schema`, Prisma will look for schema.prisma in the current directory or default location, potentially migrating the WRONG database. Always pin the path!

## Production Deployment

### Environment Variables

```bash
# Production .env
DATABASE_URL="mysql://user:pass@prod-db:3306/trashtech_prod"
DATABASE_URL_BILLING="mysql://user:pass@billing-db:3306/billing_prod"

TILLED_SECRET_KEY_TRASHTECH=sk_live_xxx
TILLED_ACCOUNT_ID_TRASHTECH=acct_live_xxx
TILLED_WEBHOOK_SECRET_TRASHTECH=whsec_live_xxx
TILLED_SANDBOX=false
```

### Deploy Steps

1. Create production billing database
2. Run migrations: `npx prisma migrate deploy --schema=packages/billing/prisma/schema.prisma`
3. Set `DATABASE_URL_BILLING` in production environment
4. Deploy app

## Troubleshooting

### "Environment variable DATABASE_URL_BILLING not found"

**Solution:** Set `DATABASE_URL_BILLING` in your `.env` file.

### "Can't reach database server"

**Solution:** Ensure billing database exists and credentials are correct.

### "Table billing_customers doesn't exist"

**Solution:** Run migrations: `npm run prisma:migrate`

### "Prisma Client not generated"

**Solution:** Run `npm run prisma:generate` in packages/billing

### Development: Use same database for both

```bash
DATABASE_URL="mysql://localhost:3306/dev_db"
DATABASE_URL_BILLING="mysql://localhost:3306/dev_db"
```

This works for development but loses isolation benefits.

## Next Steps

1. ✅ Database created
2. ✅ Migrations run
3. ✅ Environment variables set
4. ✅ Routes mounted in app
5. → Run sandbox tests (see SANDBOX-TEST-CHECKLIST.md)
6. → Build frontend payment form
7. → Launch!

## Benefits Summary

**With separate database:**
- ✅ Billing module can be used in TrashTech, Apping, or any app
- ✅ No schema conflicts between apps
- ✅ Billing scales independently
- ✅ Clear separation of concerns
- ✅ Easy to extract to separate service later

**vs monolithic (shared database):**
- ❌ Tied to one app's database
- ❌ Schema conflicts on version upgrades
- ❌ Hard to reuse across apps
- ❌ Coupling increases over time

You made the right call! 🎯
