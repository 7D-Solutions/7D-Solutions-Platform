# Phase 3: Proration Engine API Reference

## Overview
The proration engine handles mid-cycle subscription changes with time-based calculations for:
- Plan upgrades/downgrades
- Subscription cancellations (partial refunds)
- Quantity changes

## API Endpoints

### 1. Calculate Proration Preview
**POST** `/api/billing/proration/calculate`

Calculate proration breakdown without applying changes.

#### Request Body
```json
{
  "subscription_id": 123,
  "change_date": "2026-01-15T00:00:00Z",
  "new_price_cents": 10000,
  "old_price_cents": 5000,
  "new_quantity": 1,
  "old_quantity": 1,
  "proration_behavior": "create_prorations"
}
```

#### Parameters
- `subscription_id` (required): Local billing subscription ID
- `change_date` (required): ISO 8601 date when change takes effect
- `new_price_cents` (required): New plan price in cents
- `old_price_cents` (required): Current plan price in cents
- `new_quantity` (optional, default: 1): New quantity
- `old_quantity` (optional, default: 1): Current quantity
- `proration_behavior` (optional): `create_prorations` (default), `none`, `always_invoice`

#### Response
```json
{
  "proration": {
    "subscription_id": 123,
    "change_date": "2026-01-15T00:00:00Z",
    "time_proration": {
      "daysUsed": 14,
      "daysRemaining": 17,
      "daysTotal": 31,
      "prorationFactor": 0.5484
    },
    "old_plan": {
      "price_cents": 5000,
      "quantity": 1,
      "total_cents": 5000,
      "credit_cents": 2742
    },
    "new_plan": {
      "price_cents": 10000,
      "quantity": 1,
      "total_cents": 10000,
      "charge_cents": 5484
    },
    "net_change": {
      "amount_cents": 2742,
      "type": "charge",
      "description": "Prorated charge for upgrade"
    }
  }
}
```

### 2. Apply Subscription Change with Proration
**POST** `/api/billing/subscriptions/:subscription_id/proration/apply`

Apply subscription change and create proration charges/credits.

#### Request Body
```json
{
  "new_price_cents": 10000,
  "old_price_cents": 5000,
  "new_quantity": 1,
  "old_quantity": 1,
  "new_plan_id": "pro-monthly",
  "old_plan_id": "basic-monthly",
  "proration_behavior": "create_prorations",
  "effective_date": "2026-01-15T00:00:00Z",
  "invoice_immediately": false
}
```

#### Parameters
- Path: `subscription_id` (required)
- `new_price_cents`, `old_price_cents` (required)
- `new_quantity`, `old_quantity` (optional, default: 1)
- `new_plan_id`, `old_plan_id` (optional)
- `proration_behavior` (optional): `create_prorations` (default), `none`, `always_invoice`
- `effective_date` (optional, default: now): When change takes effect
- `invoice_immediately` (optional, default: false): Whether to invoice immediately

#### Response
```json
{
  "subscription": { ... },
  "proration": { ... },
  "charges": [
    {
      "id": 456,
      "charge_type": "proration_charge",
      "amount_cents": 5484,
      "status": "pending"
    }
  ]
}
```

### 3. Calculate Cancellation Refund
**POST** `/api/billing/subscriptions/:subscription_id/proration/cancellation-refund`

Calculate refund amount for subscription cancellation.

#### Request Body
```json
{
  "cancellation_date": "2026-01-15T00:00:00Z",
  "refund_behavior": "partial_refund"
}
```

#### Parameters
- Path: `subscription_id` (required)
- `cancellation_date` (required): ISO 8601 cancellation date
- `refund_behavior` (optional): `partial_refund` (default), `account_credit`, `none`

#### Response
```json
{
  "cancellation_refund": {
    "subscription_id": 123,
    "cancellation_date": "2026-01-15T00:00:00Z",
    "refund_behavior": "partial_refund",
    "time_proration": { ... },
    "total_paid_cents": 5000,
    "refund_amount_cents": 2742,
    "action": "refund",
    "description": "Partial refund of $27.42 for unused service"
  }
}
```

## Proration Behavior Options

1. **`create_prorations`** (default)
   - Create proration charges/credits for mid-cycle changes
   - Update subscription with new price/plan
   - Store proration details in charge metadata

2. **`none`**
   - Update subscription without proration
   - No charges/credits created
   - Change takes effect immediately for billing purposes

3. **`always_invoice`**
   - Future extension: Immediately invoice proration amount
   - Currently behaves like `create_prorations`

## Time-Based Proration Logic

- Uses daily proration based on calendar days
- Normalizes all dates to UTC midnight for consistency
- Handles edge cases:
  - Change at period start: proration factor = 1.0 (full period remaining)
  - Change at period end: proration factor = 0.0 (no time remaining)
  - Leap years and month boundaries handled correctly

## Database Schema
- Uses existing `billing_charges` table
- `charge_type`: `proration_credit` (negative amount) or `proration_charge` (positive)
- Proration details stored in `metadata.proration` JSON field
- Audit trail in `billing_events` table

## Integration Notes
- Proration charges flow through existing discount and tax calculation pipeline
- Works with Phase 1 (Tax) and Phase 2 (Discount) services
- Standalone endpoints; future integration with `SubscriptionService.updateSubscription()` possible

## Testing
- **Unit tests:** 35 test cases in `tests/unit/prorationService.test.js`
- **Integration tests:** 10 test cases in `tests/integration/proration-routes.test.js`
- **Service integration tests:** `tests/integration/prorationDiscountTaxFlow.test.js`

## Phase
3 - Proration Engine
**Author:** LavenderDog
**Date:** February 4, 2026