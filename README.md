# @fireproof/ar (Accounts Receivable)

**Accounts Receivable module with separate database.** Customer billing, subscriptions, payments with Tilled integration.

## Features

- ✅ **Separate database** - Not tied to any app's schema
- ✅ **Truly reusable** - Works with TrashTech, Apping, or any app
- ✅ PCI-safe (client-side payment collection only)
- ✅ Multi-app support (via `app_id`)
- ✅ Card + ACH recurring payments
- ✅ Webhook processing with idempotency
- ✅ Signature verification (HMAC SHA256 + timestamp tolerance)
- ✅ Complete Tilled API integration

## File Structure

```
packages/ar/
├── package.json
├── README.md
├── INTEGRATION.md
├── SEPARATE-DATABASE-SETUP.md    ← Setup guide
├── prisma/
│   └── schema.prisma              ← Own Prisma schema
└── backend/src/
    ├── index.js           (25 lines)
    ├── prisma.js          (12 lines) ← Own Prisma client
    ├── tilledClient.js    (130 lines)
    ├── billingService.js  (230 lines)
    ├── routes.js          (136 lines)
    └── middleware.js      (65 lines)
```

**Total: ~598 lines of clean, production-ready code**

## Database Schema

3 tables:
- `billing_customers` - Customer records with optional default payment method
- `billing_subscriptions` - Active/canceled subscriptions
- `billing_webhooks` - Webhook events with status tracking

All tables include proper indexes and Prisma enums.

## Quick Start

See [INTEGRATION.md](./INTEGRATION.md) for full setup guide.

```javascript
const { BillingService } = require('@fireproof/ar');
const billingService = new BillingService();

// Create customer
const customer = await billingService.createCustomer(
  'trashtech', 'customer@example.com', 'Acme Waste Inc'
);

// Create subscription (after payment method collection via Tilled.js)
const subscription = await billingService.createSubscription(
  customer.id,
  paymentMethodId,
  'pro-monthly',
  'Pro Monthly',
  9900  // $99.00 in cents
);
```

## Environment Variables

```bash
# Billing database (separate from main app)
DATABASE_URL_BILLING="mysql://user:pass@localhost:3306/billing_db"

# Tilled credentials (per app)
TILLED_SECRET_KEY_TRASHTECH=sk_xxx
TILLED_ACCOUNT_ID_TRASHTECH=acct_xxx
TILLED_WEBHOOK_SECRET_TRASHTECH=whsec_xxx
TILLED_SANDBOX=true
```

**Key:** Billing has its own database, configured via `DATABASE_URL_BILLING`.

## Implementation Highlights

### Subscription Flow
1. Frontend: Collect payment via Tilled.js → get `payment_method_id`
2. Backend: Attach payment method to customer
3. Backend: Create subscription in Tilled
4. Backend: Save to database

### Webhook Processing
1. Try insert event record (idempotency via unique constraint)
2. If duplicate → return 200 immediately
3. Verify signature (timestamp + HMAC)
4. Process event + update subscription
5. Update webhook status (processed/failed)

### Security
- Raw body capture for webhook signature verification
- Timestamp tolerance (±5 min) prevents replay attacks
- Length check before `timingSafeEqual`
- PCI middleware rejects raw card data
- App isolation via `app_id` scoping

## Authentication Strategy

This billing module uses **app-scoped API key authentication** to securely isolate multi-tenant data.

### How It Works

1. **App ID Middleware** (`requireAppId()`)
   - Validates incoming requests using the `X-App-Id` header and API key
   - Extracts and verifies the authenticated app (e.g., `trashtech`, `apping`)
   - Attaches `req.verifiedAppId` for downstream use

2. **Multi-Tenant Isolation**
   - All database queries include `app_id` in WHERE clauses
   - Prevents cross-tenant data access at the query level
   - Each app has separate Tilled credentials via environment variables:
     ```bash
     TILLED_SECRET_KEY_TRASHTECH=sk_xxx
     TILLED_ACCOUNT_ID_TRASHTECH=acct_xxx
     TILLED_WEBHOOK_SECRET_TRASHTECH=whsec_xxx
     ```

3. **Webhook Authentication**
   - Webhooks use **signature verification only** (no app-level auth)
   - HMAC SHA256 signature + timestamp tolerance (±5 min)
   - Prevents replay attacks and unauthorized webhook injection

### Security Model

- **App-level authorization**: Platform apps must authenticate before accessing billing APIs
- **Customer-level isolation**: Apps can only access their own customers/subscriptions via `app_id` scoping
- **PCI compliance**: Middleware rejects raw card data; only payment tokens are accepted
- **Idempotency**: Duplicate webhook events are detected via unique constraints on `(event_id, app_id)`

### Integration Example

```javascript
// Frontend: Call billing API with app credentials
const response = await fetch('/api/billing/customers', {
  method: 'POST',
  headers: {
    'X-App-Id': 'trashtech',
    'X-API-Key': process.env.TRASHTECH_API_KEY,
    'Content-Type': 'application/json'
  },
  body: JSON.stringify({ email, name })
});
```

For full integration details, see [INTEGRATION.md](./INTEGRATION.md).

## Dependencies

- `tilled-node` - Official Tilled SDK
- `@fireproof/infrastructure` - BaseRepository, BaseService, logger

## License

MIT
