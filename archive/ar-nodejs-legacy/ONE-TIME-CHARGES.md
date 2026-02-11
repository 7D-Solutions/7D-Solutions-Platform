# One-Time Charges

## Overview

The One-Time Charges feature enables immediate charging for operational add-ons like extra pickups, driver tips, and other non-recurring services. Charges are processed using the customer's default payment method on file.

## Key Features

- **Immediate Processing**: Charges are created and processed in real-time
- **Idempotent**: Supports retry-safe operations via `Idempotency-Key` header
- **Duplicate Prevention**: Unique `reference_id` per app prevents accidental double-billing
- **Audit Trail**: All charge attempts (successful or failed) are persisted for visibility
- **PCI-Safe**: Rejects requests containing raw card/ACH data

## API Endpoint

### POST /api/billing/charges/one-time

Create a one-time charge for a customer.

**Query Parameters:**
- `app_id` (required): Application identifier (e.g., 'trashtech')

**Headers:**
- `Idempotency-Key` (required): UUID or unique string to prevent duplicate processing on retries
- `Content-Type`: application/json

**Request Body:**
```json
{
  "external_customer_id": "cust_12345",
  "amount_cents": 3500,
  "currency": "usd",
  "reason": "extra_pickup",
  "reference_id": "pickup_20260123_001",
  "service_date": "2026-01-23",
  "note": "Extra pickup requested by customer",
  "metadata": {
    "route_id": "R12",
    "driver_id": "DRV_456"
  }
}
```

**Request Fields:**
- `external_customer_id` (required): Your app's customer identifier
- `amount_cents` (required): Charge amount in cents (integer > 0)
- `currency` (optional): Currency code (default: 'usd')
- `reason` (required): Charge reason/type (e.g., 'extra_pickup', 'tip')
- `reference_id` (required): **Unique identifier** for this charge within your app. **Cannot be empty or whitespace-only.** Used for domain-level duplicate prevention. See best practices below for recommended formats.
- `service_date` (optional): ISO date when the service was provided
- `note` (optional): Additional context or description
- `metadata` (optional): Custom key-value data for your app's use

**Response (201 Created):**
```json
{
  "charge": {
    "id": 123,
    "app_id": "trashtech",
    "billing_customer_id": 45,
    "status": "succeeded",
    "amount_cents": 3500,
    "currency": "usd",
    "charge_type": "one_time",
    "reason": "extra_pickup",
    "reference_id": "pickup_20260123_001",
    "service_date": "2026-01-23T00:00:00.000Z",
    "note": "Extra pickup requested by customer",
    "metadata": {
      "route_id": "R12",
      "driver_id": "DRV_456"
    },
    "tilled_charge_id": "ch_abc123",
    "created_at": "2026-01-23T10:30:00.000Z",
    "updated_at": "2026-01-23T10:30:00.000Z"
  }
}
```

**Response Fields:**
- `charge_type`: Always `"one_time"` for charges created via this endpoint. Future-proofs the system for distinguishing from subscription charges, invoice charges, etc.

**Error Responses:**

- **400 Bad Request**
  - Missing `app_id`
  - Missing `Idempotency-Key` header
  - Missing required body fields: `external_customer_id`, `amount_cents`, `reason`, or `reference_id`
  - Empty or whitespace-only `reference_id`
  - Invalid `amount_cents` (must be integer > 0)
  - PCI-sensitive data detected in request body (card_number, cvv, account_number, etc.)

- **404 Not Found**
  - Customer not found for given `app_id` + `external_customer_id`

- **409 Conflict** (three distinct scenarios)
  1. **Idempotency conflict**: `Idempotency-Key` reused with different payload (different request_hash)
  2. **No payment method**: Customer has no default payment method on file
  3. **Duplicate reference_id**: `reference_id` already used for this app (returns existing charge record with 201)

- **502 Bad Gateway**
  - Tilled charge creation failed (payment declined, insufficient funds, etc.)
  - Charge record still persisted locally with `status: 'failed'`
  - Response includes `code` and `message` from Tilled

## Idempotency

### Two-Layer Duplicate Prevention

The API provides two complementary mechanisms to prevent double-charging:

#### 1. Idempotency-Key (Request-Level)
Protects against network retries and client-side failures.

```bash
# First request
POST /api/billing/charges/one-time?app_id=trashtech
Idempotency-Key: abc-123-def
{
  "reference_id": "pickup_001",
  ...
}
# => 201 Created

# Retry with same key + same payload
POST /api/billing/charges/one-time?app_id=trashtech
Idempotency-Key: abc-123-def
{
  "reference_id": "pickup_001",
  ...
}
# => 201 Created (returns cached response, no duplicate charge)

# Retry with same key but DIFFERENT payload
POST /api/billing/charges/one-time?app_id=trashtech
Idempotency-Key: abc-123-def
{
  "reference_id": "pickup_002",  # Changed!
  ...
}
# => 409 Conflict (prevented)
```

**Idempotency key storage:**
- Cached responses stored for 30 days
- Request hash computed from method + path + body
- Identical requests return cached response without re-processing
- Changed payloads with same key are rejected (409)

**CRITICAL: Short-Circuit Behavior**
When an idempotent response is found, the API:
- ✅ Returns the cached response immediately
- ✅ Does NOT lookup customers or payment methods
- ✅ Does NOT check reference_id duplicates
- ✅ Does NOT create any database records
- ✅ Does NOT call Tilled API

This guarantees that retries are 100% safe, even if:
- Database was reset/cleaned between attempts
- Customer was deleted
- Payment method was removed
- Tilled credentials changed

**Idempotency happens FIRST** - before any domain logic.

#### 2. Reference ID (Domain-Level)
Protects against business logic errors and duplicate domain events.

```bash
# First charge for pickup_001
POST /api/billing/charges/one-time?app_id=trashtech
Idempotency-Key: key-1
{
  "reference_id": "pickup_001",
  ...
}
# => 201 Created

# Different idempotency key, but SAME reference_id
POST /api/billing/charges/one-time?app_id=trashtech
Idempotency-Key: key-2-different
{
  "reference_id": "pickup_001",  # Same reference_id!
  ...
}
# => 201 Created (returns existing charge, no duplicate billing)
```

**Reference ID uniqueness:**
- Enforced via database unique constraint: `@@unique([app_id, reference_id])`
- Scoped per app (different apps can use same reference_id)
- Returns existing charge record if reference_id already exists
- No Tilled API call made for duplicates

### Best Practices

**Idempotency-Key:**
- Generate once per charge attempt (e.g., UUID)
- Store in your application state before making the request
- Reuse the same key on retries/timeouts
- Never change the payload when retrying with the same key

**Reference ID (REQUIRED):**
- **MUST be provided** - empty strings and whitespace-only values are rejected
- **MUST be traceable** to a domain event (pickup, tip, work order, etc.)
- Must be unique within your app (enforced by database constraint)
- **Recommended format**: `<domain>:<id>` for clear traceability
  - **Extra pickup**: `pickup:789` or `pickup:20260123_R12`
    - Example: `pickup:PID_12345` maps to your pickup record ID 12345
  - **Driver tip**: `tip:20260123:DRV456:C789` or `tip:<tipId>`
    - Example: `tip:TIP_UUID_123` if you generate tip IDs
  - **Late fee**: `late_fee:invoice:<invoiceId>`
    - Example: `late_fee:invoice:INV_456`
  - **Service fee**: `service:<serviceType>:<date>:<customerId>`
    - Example: `service:overage:20260123:C789`
- **UUIDs are allowed** if they map to a domain record
  - ✅ GOOD: `pickup:550e8400-e29b-41d4-a716-446655440000` (UUID is your pickup ID)
  - ✅ GOOD: Store `{ domain_type: "pickup", domain_id: "550e..." }` in metadata
  - ❌ BAD: Random UUID with no traceability (can't find source event)
- **Do NOT use meaningless/untraceable identifiers** - you must be able to look up the source
- **Do NOT use the same reference_id** for different charge attempts

## Payment Method Requirements

The customer **must** have a default payment method set before creating a charge.

**Setting a default payment method:**
```bash
POST /api/billing/customers/{id}/default-payment-method
{
  "payment_method_id": "pm_abc123",
  "type": "card"
}
```

Or use the Phase 1 payment methods endpoints:
```bash
PUT /api/billing/payment-methods/{pm_id}/default
```

If no default payment method exists, the charge request will fail with **409 Conflict**.

## Charge Lifecycle

1. **Pending** - Charge record created, Tilled API call in progress
2. **Succeeded** - Tilled charge completed successfully
3. **Failed** - Tilled charge failed (declined, insufficient funds, etc.)

**All attempts are persisted**, even failures. This provides full audit trail for accounting and support.

## Supported Reasons

While the `reason` field accepts any string, common use cases include:

- `extra_pickup` - Additional waste pickup beyond subscription
- `tip` - Driver tip or gratuity
- `late_fee` - Late payment penalty
- `service_fee` - One-time service charge
- `overage` - Usage overage charge

Your application can define custom reason codes as needed.

## Security

### PCI Compliance

The API automatically **rejects** requests containing sensitive payment data:
- `card_number`
- `card_cvv` / `cvv` / `cvc`
- `account_number`
- `routing_number`

All payment processing uses tokenized payment methods from Tilled. Raw card data is never stored or transmitted.

### App Isolation

Charges are strictly scoped to `app_id`:
- Customers from one app cannot access charges from another app
- `reference_id` uniqueness is per-app
- All queries enforce app boundary checks

## Error Handling

### Payment Failures

When Tilled rejects a charge (e.g., card declined), the API:
1. Persists the charge record with `status: 'failed'`
2. Stores `failure_code` and `failure_message` from Tilled
3. Returns **502 Bad Gateway** to the client
4. Throws the original Tilled error for client inspection

**Example failed charge record:**
```json
{
  "id": 124,
  "status": "failed",
  "amount_cents": 3500,
  "reason": "extra_pickup",
  "tilled_charge_id": null,
  "failure_code": "card_declined",
  "failure_message": "Insufficient funds",
  ...
}
```

This allows you to:
- Track all charge attempts (for audit/support)
- Identify patterns in payment failures
- Retry with a different payment method

### Retry Strategy

For transient errors (network timeouts, 5xx responses):
1. **Reuse the same `Idempotency-Key`**
2. Send the **exact same request body**
3. The API will return the cached response if already processed
4. No duplicate charge will be created

For permanent errors (404, 409, 400):
- Do not retry automatically
- Fix the underlying issue first
- Use a new `Idempotency-Key` for the corrected request

## Database Schema

```sql
CREATE TABLE billing_charges (
  id                  INT PRIMARY KEY AUTO_INCREMENT,
  app_id              VARCHAR(50) NOT NULL,
  tilled_charge_id    VARCHAR(255) UNIQUE,
  billing_customer_id INT NOT NULL,
  subscription_id     INT,
  invoice_id          INT,

  -- One-time charge fields
  status              VARCHAR(20) NOT NULL,     -- 'pending' | 'succeeded' | 'failed'
  amount_cents        INT NOT NULL,
  currency            VARCHAR(3) DEFAULT 'usd',
  reason              VARCHAR(100),             -- 'extra_pickup', 'tip', etc.
  reference_id        VARCHAR(255),             -- Unique per app
  service_date        TIMESTAMP,
  note                TEXT,
  metadata            JSON,

  -- Failure tracking
  failure_code        VARCHAR(50),
  failure_message     TEXT,

  created_at          TIMESTAMP DEFAULT NOW(),
  updated_at          TIMESTAMP DEFAULT NOW() ON UPDATE NOW(),

  UNIQUE KEY unique_app_reference_id (app_id, reference_id),
  KEY idx_app_id (app_id),
  KEY idx_billing_customer_id (billing_customer_id),
  KEY idx_status (status),
  KEY idx_reason (reason),
  KEY idx_service_date (service_date),

  FOREIGN KEY (billing_customer_id) REFERENCES billing_customers(id) ON DELETE CASCADE
);

CREATE TABLE billing_idempotency_keys (
  id              INT PRIMARY KEY AUTO_INCREMENT,
  app_id          VARCHAR(50) NOT NULL,
  idempotency_key VARCHAR(255) NOT NULL,
  request_hash    VARCHAR(64) NOT NULL,
  response_body   JSON NOT NULL,
  status_code     INT NOT NULL,
  created_at      TIMESTAMP DEFAULT NOW(),
  expires_at      TIMESTAMP NOT NULL,

  UNIQUE KEY unique_app_idempotency_key (app_id, idempotency_key),
  KEY idx_app_id (app_id),
  KEY idx_expires_at (expires_at)
);
```

## Integration Example

```javascript
// TrashTech Pro: Charge for extra pickup
const axios = require('axios');
const { v4: uuidv4 } = require('uuid');

async function chargeForExtraPickup(pickupId, customerId, amountCents) {
  const idempotencyKey = uuidv4();

  try {
    const response = await axios.post(
      '/api/billing/charges/one-time?app_id=trashtech',
      {
        external_customer_id: customerId,
        amount_cents: amountCents,
        currency: 'usd',
        reason: 'extra_pickup',
        reference_id: `pickup_${pickupId}`,
        service_date: new Date().toISOString().split('T')[0],
        note: `Extra pickup for route ${routeId}`,
        metadata: {
          pickup_id: pickupId,
          route_id: routeId,
          driver_id: driverId
        }
      },
      {
        headers: {
          'Idempotency-Key': idempotencyKey,
          'Content-Type': 'application/json'
        }
      }
    );

    console.log('Charge succeeded:', response.data.charge);
    return response.data.charge;

  } catch (error) {
    if (error.response?.status === 409) {
      // Duplicate charge (reference_id already exists)
      console.log('Charge already processed');
      return error.response.data.charge;
    }

    if (error.response?.status === 502) {
      // Payment failed
      console.error('Payment declined:', error.response.data.message);
      throw new Error('Payment failed');
    }

    // Other errors
    console.error('Charge error:', error.response?.data || error.message);
    throw error;
  }
}
```

## Testing

### Unit Tests
Run unit tests with mocked dependencies:
```bash
npm test tests/unit/oneTimeCharges.test.js
```

### Integration Tests
Run integration tests against test database:
```bash
npm test tests/integration/routes.test.js -t "POST /api/billing/charges/one-time"
```

### Test Coverage
- Idempotency key handling (replay, conflict detection)
- Reference ID duplicate prevention
- Payment method validation
- PCI-sensitive data rejection
- Success and failure flows
- Database persistence

## Monitoring

**Key metrics to track:**
- Charge success rate by `reason`
- Average charge amount by `reason`
- Top failure codes
- Duplicate prevention rate (409 responses)
- Processing time percentiles

**Database queries for reporting:**
```sql
-- Success rate by reason
SELECT
  reason,
  COUNT(*) as total,
  SUM(CASE WHEN status = 'succeeded' THEN 1 ELSE 0 END) as succeeded,
  AVG(CASE WHEN status = 'succeeded' THEN 1 ELSE 0 END) * 100 as success_rate
FROM billing_charges
WHERE app_id = 'trashtech'
  AND created_at >= DATE_SUB(NOW(), INTERVAL 30 DAY)
GROUP BY reason;

-- Top failure reasons
SELECT
  failure_code,
  COUNT(*) as count,
  AVG(amount_cents) as avg_amount
FROM billing_charges
WHERE app_id = 'trashtech'
  AND status = 'failed'
  AND created_at >= DATE_SUB(NOW(), INTERVAL 7 DAY)
GROUP BY failure_code
ORDER BY count DESC
LIMIT 10;
```

## Related Endpoints

- **GET /api/billing/state** - Get customer's billing state (includes payment methods)
- **POST /api/billing/payment-methods** - Add payment method to customer
- **PUT /api/billing/payment-methods/:id/default** - Set default payment method
- **POST /api/billing/customers/:id/default-payment-method** - Set default payment method (legacy)

## Support

For questions or issues:
1. Check error response messages (they include specific guidance)
2. Verify customer has default payment method set
3. Confirm `reference_id` follows uniqueness guidelines
4. Review charge record in database for failure details
