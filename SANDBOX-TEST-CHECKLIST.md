# Sandbox Testing Checklist

Run these tests BEFORE processing real customer payments.

## Prerequisites

- [ ] Migration run: `npx prisma migrate dev --name add_billing_tables`
- [ ] Environment variables set (sandbox credentials)
- [ ] Backend server running
- [ ] Tilled sandbox account created
- [ ] Webhook URL configured in Tilled dashboard

## Test 1: Create Billing Customer

```bash
curl -X POST http://localhost:3000/api/billing/customers \
  -H "Content-Type: application/json" \
  -d '{
    "app_id": "trashtech",
    "email": "test@acmewaste.com",
    "name": "Acme Waste Inc",
    "external_customer_id": "123",
    "metadata": {"industry": "waste"}
  }'
```

**Expected:**
- [ ] Returns 201 status
- [ ] Response includes `id`, `tilled_customer_id`
- [ ] Record appears in `billing_customers` table
- [ ] Customer appears in Tilled sandbox dashboard

## Test 2: Collect Payment Method (Card)

### Frontend (Tilled.js)
```html
<script src="https://js.tilled.com/v1"></script>
<div id="card-number"></div>
<div id="card-cvv"></div>
<div id="card-expiry"></div>
<button id="submit">Subscribe</button>

<script>
  const tilled = new Tilled('pk_SANDBOX_KEY', {
    sandbox: true,
    accountId: 'acct_SANDBOX'
  });

  const cardFields = tilled.createCardFields({
    cardNumber: { element: '#card-number' },
    cardCvv: { element: '#card-cvv' },
    cardExpiry: { element: '#card-expiry' }
  });

  document.getElementById('submit').onclick = async () => {
    // Use test card: 4242424242424242
    const { paymentMethod, error } = await cardFields.createPaymentMethod({
      billing_details: { name: 'Test User' }
    });

    if (error) {
      console.error(error);
      return;
    }

    console.log('Payment Method ID:', paymentMethod.id);
    // Save this ID for next test
  };
</script>
```

**Expected:**
- [ ] Hosted fields render correctly
- [ ] Test card (4242424242424242) is accepted
- [ ] Returns `payment_method_id` (starts with `pm_`)
- [ ] Payment method appears in Tilled dashboard

**Test Cards (Sandbox):**
- Success: `4242424242424242`
- Decline: `4000000000000002`
- Insufficient funds: `4000000000009995`

## Test 3: Set Default Payment Method

```bash
curl -X POST http://localhost:3000/api/billing/customers/{CUSTOMER_ID}/default-payment-method \
  -H "Content-Type: application/json" \
  -d '{
    "payment_method_id": "pm_xxxxxxxx",
    "payment_method_type": "card"
  }'
```

**Expected:**
- [ ] Returns 200 status
- [ ] `billing_customers` record updated with `default_payment_method_id`
- [ ] `payment_method_type` set to "card"

## Test 4: Create Subscription (Card)

```bash
curl -X POST http://localhost:3000/api/billing/subscriptions \
  -H "Content-Type: application/json" \
  -d '{
    "billing_customer_id": 1,
    "payment_method_id": "pm_xxxxxxxx",
    "plan_id": "trashtech-pro-monthly",
    "plan_name": "TrashTech Pro Monthly",
    "price_cents": 9900,
    "interval_unit": "month",
    "interval_count": 1,
    "metadata": {"features": ["routes", "analytics"]}
  }'
```

**Expected:**
- [ ] Returns 201 status
- [ ] Response includes `id`, `tilled_subscription_id`, `status: "active"`
- [ ] Record appears in `billing_subscriptions` table
- [ ] Subscription appears in Tilled dashboard
- [ ] Initial charge succeeds (check Tilled dashboard)

## Test 5: Collect Payment Method (ACH)

### Frontend (Tilled.js)
```javascript
const achFields = tilled.createAchDebitFields({
  accountNumber: { element: '#account-number' },
  routingNumber: { element: '#routing-number' }
});

const { paymentMethod } = await achFields.createPaymentMethod({
  billing_details: { name: 'Test User' },
  type: 'ach_debit'
});

// Test ACH (Sandbox):
// Routing: 110000000
// Account: 000123456789
```

**Expected:**
- [ ] ACH fields render correctly
- [ ] Test ACH details accepted
- [ ] Returns `payment_method_id` with `type: "ach_debit"`

## Test 6: Create Subscription (ACH)

```bash
curl -X POST http://localhost:3000/api/billing/subscriptions \
  -H "Content-Type: application/json" \
  -d '{
    "billing_customer_id": 1,
    "payment_method_id": "pm_ach_xxxxxxxx",
    "plan_id": "trashtech-pro-monthly",
    "plan_name": "TrashTech Pro Monthly",
    "price_cents": 9900,
    "interval_unit": "month",
    "metadata": {"payment_type": "ach"}
  }'
```

**Expected:**
- [ ] ACH subscription created
- [ ] `payment_method_type` set to "ach_debit"
- [ ] Status transitions to "active" (ACH may start as "incomplete")

## Test 7: Webhook Processing

### Trigger Test Webhook
In Tilled dashboard:
1. Go to Webhooks → Events
2. Click "Send test webhook"
3. Select event type: `subscription.created`
4. Send to your webhook URL

**Expected:**
- [ ] Webhook received (check logs)
- [ ] Signature verified successfully
- [ ] Record created in `billing_webhooks` with `status: "processed"`
- [ ] No duplicate processing on retry

### Test Webhook Idempotency
Resend same webhook from Tilled dashboard.

**Expected:**
- [ ] Returns 200 status
- [ ] Response: `{ "received": true, "duplicate": true }`
- [ ] No duplicate record in database
- [ ] Original webhook record unchanged

### Test Invalid Signature
```bash
curl -X POST http://localhost:3000/api/billing/webhooks/trashtech \
  -H "Content-Type: application/json" \
  -H "payments-signature: t=123,v1=invalid" \
  -d '{"id": "evt_test", "type": "test"}'
```

**Expected:**
- [ ] Returns 401 status
- [ ] Webhook record created with `status: "failed"`
- [ ] Error field: "Invalid signature"

## Test 8: Webhook Event Types

Trigger these events from Tilled dashboard or by performing actions:

### subscription.updated
**Trigger:** Update subscription in Tilled dashboard
**Expected:**
- [ ] Webhook processed
- [ ] `billing_subscriptions` record updated
- [ ] Status/dates synced

### subscription.canceled
**Trigger:** Cancel subscription (see Test 9)
**Expected:**
- [ ] Webhook processed
- [ ] Status updated to "canceled"
- [ ] `canceled_at` timestamp set

### payment_intent.succeeded
**Expected:**
- [ ] Webhook received
- [ ] Logged (no action needed for v1)

### payment_intent.payment_failed
**Trigger:** Use decline card (4000000000000002)
**Expected:**
- [ ] Webhook received
- [ ] Subscription status may change to "past_due"

## Test 9: Cancel Subscription

```bash
curl -X DELETE http://localhost:3000/api/billing/subscriptions/1
```

**Expected:**
- [ ] Returns 200 status
- [ ] Subscription status updated to "canceled"
- [ ] `canceled_at` timestamp set
- [ ] Subscription marked as canceled in Tilled dashboard
- [ ] Webhook received (`subscription.canceled`)

## Test 10: Error Handling

### Missing payment method
```bash
curl -X POST http://localhost:3000/api/billing/subscriptions \
  -H "Content-Type: application/json" \
  -d '{
    "billing_customer_id": 1,
    "payment_method_id": "pm_invalid",
    "plan_id": "test",
    "plan_name": "Test",
    "price_cents": 1000
  }'
```

**Expected:**
- [ ] Returns 500 status
- [ ] Error message indicates payment method not found

### PCI violation attempt
```bash
curl -X POST http://localhost:3000/api/billing/subscriptions \
  -H "Content-Type: application/json" \
  -d '{
    "billing_customer_id": 1,
    "card_number": "4242424242424242",
    "plan_id": "test",
    "plan_name": "Test",
    "price_cents": 1000
  }'
```

**Expected:**
- [ ] Returns 400 status (if middleware.rejectSensitiveData applied)
- [ ] Error: "PCI violation: Use Tilled hosted fields"

## Test 11: Database Verification

```sql
-- Check customers
SELECT * FROM billing_customers WHERE app_id = 'trashtech';

-- Check subscriptions
SELECT
  s.id,
  s.plan_id,
  s.price_cents,
  s.status,
  s.payment_method_type,
  c.email
FROM billing_subscriptions s
JOIN billing_customers c ON s.billing_customer_id = c.id
WHERE s.app_id = 'trashtech';

-- Check webhooks
SELECT
  event_type,
  status,
  attempt_count,
  error,
  processed_at
FROM billing_webhooks
WHERE app_id = 'trashtech'
ORDER BY received_at DESC;

-- Check for failures
SELECT * FROM billing_webhooks WHERE status = 'failed';
```

**Expected:**
- [ ] All data matches API responses
- [ ] No orphaned records
- [ ] Webhook status/error tracking works
- [ ] Timestamps make sense

## Test 12: Multi-App Isolation

Create customer for different app:
```bash
curl -X POST http://localhost:3000/api/billing/customers \
  -H "Content-Type: application/json" \
  -d '{
    "app_id": "apping",
    "email": "test@apping.com",
    "name": "Apping Test"
  }'
```

**Expected:**
- [ ] Customer created with `app_id: "apping"`
- [ ] Separate Tilled account credentials loaded
- [ ] No cross-contamination with trashtech data

## Production Readiness Checklist

After all sandbox tests pass:

- [ ] All 12 tests passed
- [ ] No webhook signature failures
- [ ] Idempotency working correctly
- [ ] ACH and card flows both work
- [ ] Error handling graceful
- [ ] Database state correct
- [ ] Logs show no errors
- [ ] Ready to configure production credentials
- [ ] Ready to process first real customer

## Next Steps

1. ✅ All sandbox tests passing
2. Configure production Tilled account
3. Update environment variables (production keys)
4. Set `TILLED_SANDBOX=false`
5. Process first real subscription
6. Monitor first 5-10 transactions closely
7. Set up alerts for failed webhooks

## Support Resources

- Tilled Docs: https://docs.tilled.com
- Tilled Sandbox Dashboard: https://sandbox.tilled.com
- Support: support@tilled.com
