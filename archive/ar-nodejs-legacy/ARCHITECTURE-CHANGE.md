# Architecture Change: Separate Database

## What Changed

The billing module now has **its own database**, completely separate from the main application.

### Before (Monolithic)
```
apps/backend/prisma/schema.prisma
├── gauge tables
├── customer tables
├── quoting tables
└── billing tables ❌ (mixed in with app)

Main app and billing share one database
```

### After (Separated)
```
apps/backend/prisma/schema.prisma
├── gauge tables
├── customer tables
└── quoting tables

packages/ar/prisma/schema.prisma
├── billing_customers
├── billing_subscriptions
└── billing_webhooks

Two separate databases with clear boundaries ✅
```

## Why This Matters

### ✅ Truly Generic & Reusable

**Before:** Billing tied to TrashTech ERP database
- Can't use in Apping without sharing databases
- Schema migrations affect entire app
- Tight coupling to app structure

**After:** Billing is completely standalone
- Can be used in TrashTech, Apping, or any app
- Each app sets `DATABASE_URL_BILLING` to their billing DB
- Same billing code, different instances
- No schema conflicts

### ✅ Clear Separation of Concerns

**Before:** Mixed responsibilities
```javascript
// Main app Prisma client handles everything
import { prisma } from '@fireproof/infrastructure';
await prisma.billing_customers.create(...);  // Billing in app DB
await prisma.customers.create(...);           // App data
```

**After:** Explicit boundaries
```javascript
// Main app uses main Prisma
import { prisma } from '@fireproof/infrastructure';
await prisma.customers.create(...);

// Billing uses billing Prisma
import { billingPrisma } from '@fireproof/ar';
await billingPrisma.billing_customers.create(...);
```

### ✅ Independent Scaling

**Before:** Billing data grows → entire app DB grows
**After:** Billing DB scales independently from app DB

### ✅ Flexible Deployment

```bash
# Dev: Same MySQL instance, different databases
DATABASE_URL="mysql://localhost:3306/trashtech_dev"
DATABASE_URL_BILLING="mysql://localhost:3306/billing_dev"

# Staging: Shared billing across apps
DATABASE_URL="mysql://staging:3306/trashtech_staging"
DATABASE_URL_BILLING="mysql://billing:3306/billing_staging"  # Shared

# Production: Fully separate
DATABASE_URL="mysql://trashtech-db:3306/production"
DATABASE_URL_BILLING="mysql://billing-db:3306/production"    # Dedicated
```

## Technical Changes

### 1. New Files Created

```
packages/ar/
├── prisma/
│   └── schema.prisma          ← Billing's own schema
└── backend/src/
    └── prisma.js              ← Billing's own Prisma client
```

### 2. Files Modified

**billingService.js:**
```javascript
// Before
const { prisma } = require('@fireproof/infrastructure/database/prisma');
await prisma.billing_customers.create(...);

// After
const { billingPrisma } = require('./prisma');
await billingPrisma.billing_customers.create(...);
```

**No longer uses BaseRepository** - Uses Prisma directly (cleaner for separate DB).

### 3. Files Removed From

**apps/backend/prisma/schema.prisma:**
- Removed `billing_customers` model
- Removed `billing_subscriptions` model
- Removed `billing_webhooks` model
- Removed billing enums

**packages/infrastructure/src/repositories/BaseRepository.js:**
- Removed billing tables from `ALLOWED_TABLES`

### 4. Dependencies Added

**packages/ar/package.json:**
```json
{
  "dependencies": {
    "@prisma/client": "^6.19.2"  ← Own Prisma client
  },
  "devDependencies": {
    "prisma": "^6.19.2"           ← Own Prisma CLI
  }
}
```

## Data Linking Pattern

Apps link to billing via `external_customer_id`:

```javascript
// 1. Create customer in YOUR app's database
const traschtechCustomer = await prisma.customers.create({
  data: { business_name: 'Acme Waste', email: 'acme@example.com' }
});

// 2. Create billing customer in BILLING database
const billingCustomer = await billingService.createCustomer(
  'trashtech',
  'acme@example.com',
  'Acme Waste',
  traschtechCustomer.id,  ← Link via external_customer_id
  { industry: 'waste' }
);

// 3. Optionally store billing_customer_id back in your app
await prisma.customers.update({
  where: { id: traschtechCustomer.id },
  data: { billing_customer_id: billingCustomer.id }  ← For quick lookups
});
```

**No database-level foreign keys** across databases (by design). Apps maintain referential integrity at application layer.

## Migration Guide

### For New Projects

1. Set `DATABASE_URL_BILLING` in `.env`
2. Run `npm install` in `packages/ar`
3. Run `npm run prisma:generate` in `packages/ar`
4. Run `npm run prisma:migrate` in `packages/ar`
5. Use billing normally

### For Existing Projects (If you had old billing tables)

1. **Backup data** from old billing tables
2. Drop old billing tables from main app DB
3. Set `DATABASE_URL_BILLING` in `.env`
4. Run billing migrations (creates new DB/tables)
5. Migrate data from backup to new billing DB
6. Update app code to use new separate billing

## Benefits Summary

| Aspect | Before (Monolithic) | After (Separated) |
|--------|---------------------|-------------------|
| **Reusability** | ❌ Tied to one app | ✅ Works with any app |
| **Schema conflicts** | ❌ Version lock-in | ✅ Independent versions |
| **Scaling** | ❌ Coupled | ✅ Independent |
| **Deployment** | ❌ All-or-nothing | ✅ Flexible |
| **Multi-app** | ❌ Separate instances | ✅ Shared or separate |
| **Complexity** | Lower (one DB) | Slightly higher (two DBs) |

## Trade-offs Accepted

**Slightly more complexity:**
- Two databases to manage (vs one)
- Two sets of migrations (vs one)
- Application-layer foreign keys (vs DB-level)

**Big wins:**
- True modularity
- Reusability across apps
- Independent scaling
- Clear boundaries

## Next Steps

See **SEPARATE-DATABASE-SETUP.md** for detailed setup instructions.

---

**You made the right call!** This architecture scales beautifully for multi-app SaaS. 🎯
