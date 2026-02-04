# Billing Module Integration Guide

## Installation

```bash
npm install tilled-node
```

## Express App Setup

```javascript
const express = require('express');
const { billingRoutes, middleware } = require('@fireproof/ar');

const app = express();

// Webhook route (MUST come before express.json())
app.use(
  '/api/billing/webhooks',
  middleware.captureRawBody,
  express.json(),
  billingRoutes
);

// Other billing routes (with optional app_id auth)
app.use(
  '/api/billing',
  express.json(),
  middleware.rejectSensitiveData,
  middleware.requireAppId({
    getAppIdFromAuth: (req) => req.user?.app_id  // Optional: from JWT/session
  }),
  billingRoutes
);

// IMPORTANT: Error handler MUST be mounted AFTER all routes
app.use(middleware.handleBillingError);
```

## Environment Variables

```bash
# TrashTech App
TILLED_SECRET_KEY_TRASHTECH=sk_test_xxx
TILLED_ACCOUNT_ID_TRASHTECH=acct_xxx
TILLED_WEBHOOK_SECRET_TRASHTECH=whsec_xxx

# Global
TILLED_SANDBOX=true
```

## Usage Example

```javascript
const { BillingService } = require('@fireproof/ar');
const billingService = new BillingService();

// Step 1: Create billing customer
const customer = await billingService.createCustomer(
  'trashtech',              // app_id
  'customer@example.com',
  'Acme Waste Inc',
  traschtechCustomerId,     // Link to your customer table
  { industry: 'waste' }     // Optional metadata
);

// Step 2: Frontend collects payment via Tilled.js
// User enters card/ACH in hosted fields → get payment_method_id

// Step 3: Create subscription (backend)
const subscription = await billingService.createSubscription(
  customer.id,
  paymentMethodId,          // From Tilled hosted fields
  'trashtech-pro-monthly',  // plan_id
  'TrashTech Pro Monthly',  // plan_name
  9900,                     // price_cents ($99.00)
  {
    intervalUnit: 'month',
    intervalCount: 1,
    metadata: { features: ['routes', 'analytics'] }
  }
);
```

## API Routes

- `POST /api/billing/customers` - Create customer
- `POST /api/billing/customers/:id/default-payment-method` - Set default payment method
- `POST /api/billing/subscriptions` - Create subscription
- `DELETE /api/billing/subscriptions/:id` - Cancel subscription
- `POST /api/billing/webhooks/:app_id` - Tilled webhook endpoint

## Frontend Payment Collection (PCI-Safe)

```html
<script src="https://js.tilled.com/v1"></script>
<script>
  const tilled = new Tilled('pk_PUBLIC_KEY', { accountId: 'acct_xxx' });

  // Create hosted card fields
  const cardFields = tilled.createCardFields({
    cardNumber: { element: '#card-number' },
    cardCvv: { element: '#card-cvv' },
    cardExpiry: { element: '#card-expiry' }
  });

  // Submit to get payment_method_id
  const { paymentMethod } = await cardFields.createPaymentMethod({
    billing_details: { name: 'John Doe' }
  });

  // Send ONLY the payment_method_id to your backend
  await fetch('/api/billing/subscriptions', {
    method: 'POST',
    body: JSON.stringify({
      billing_customer_id: customerId,
      payment_method_id: paymentMethod.id,  // Tilled token (safe)
      plan_id: 'trashtech-pro-monthly',
      plan_name: 'TrashTech Pro Monthly',
      price_cents: 9900
    })
  });
</script>
```

## Database Migration

Run Prisma migration to create tables:

```bash
cd apps/backend
npx prisma migrate dev --name add_billing_tables
```

## Webhook Configuration

In Tilled dashboard:
1. Add webhook URL: `https://yourdomain.com/api/billing/webhooks/trashtech`
2. Select events: `subscription.*`, `customer.*`
3. Copy webhook secret → `TILLED_WEBHOOK_SECRET_TRASHTECH`

## Testing

Use Tilled sandbox:
- Test card: 4242424242424242
- Test ACH: Routing 110000000, Account 000123456789
