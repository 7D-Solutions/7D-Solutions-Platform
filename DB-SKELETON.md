# Billing Database Skeleton - Long-Term Stability

**Date:** 2026-01-23
**Purpose:** Future-proof database structure for Phase 2-4 features
**Status:** Schema-only (no runtime behavior yet)

---

## Overview

This document describes the complete database structure for the billing module, including skeleton tables for Phase 2-4 features. All tables follow these principles:

✅ **PCI Compliant** - No full card numbers, routing numbers, or CVV storage
✅ **Multi-App Isolated** - All tables include `app_id` with proper indexes
✅ **Additive Only** - No breaking changes, safe defaults for new columns
✅ **FK-Safe Order** - Migrations applied in dependency order

---

## Phase 1: Operational Tables (Active)

### billing_customers
**Purpose:** Customer records linked to Tilled customer accounts

**Key Fields:**
- `tilled_customer_id` - Tilled customer token (unique)
- `external_customer_id` - Link to app's customer table
- `default_payment_method_id` - Fast-path for default PM lookup
- `update_source` - Provenance tracking (api/webhook/admin)
- `updated_by` - User/system that made the change
- `delinquent_since` - Start of payment delinquency (future use)
- `grace_period_end` - End of grace period before lockout (future use)

**Relations:**
- → billing_subscriptions (one-to-many)
- → billing_payment_methods (one-to-many)
- → billing_invoices (one-to-many)
- → billing_charges (one-to-many)

**Indexes:**
- `app_id` - Multi-app isolation
- `email` - Customer lookup
- `delinquent_since` - Delinquency queries

---

### billing_subscriptions
**Purpose:** Subscription records with Tilled sync

**Key Fields:**
- `tilled_subscription_id` - Tilled subscription token (unique)
- `cancel_at_period_end` - Scheduled cancellation flag
- `ended_at` - Actual end timestamp (may differ from canceled_at)
- `update_source` - Provenance tracking
- `updated_by` - User/system that made the change

**Relations:**
- → billing_customers (many-to-one)
- → billing_subscription_addons (one-to-many)
- → billing_invoices (one-to-many)
- → billing_charges (one-to-many)

**Indexes:**
- `app_id`, `billing_customer_id`, `status`, `plan_id`, `current_period_end`

---

### billing_payment_methods
**Purpose:** PCI-compliant masked payment method storage

**Key Fields:**
- `tilled_payment_method_id` - Tilled PM token (unique)
- `type` - card, ach_debit, eft_debit
- `brand`, `last4`, `exp_month`, `exp_year` - Card metadata (masked)
- `bank_name`, `bank_last4` - Bank metadata (masked)
- `is_default` - Default flag (with customer fast-path)
- `deleted_at` - Soft delete timestamp

**Relations:**
- → billing_customers (many-to-one)

**Indexes:**
- `app_id`, `billing_customer_id`, `(billing_customer_id, is_default)`

---

### billing_webhooks
**Purpose:** Webhook event tracking with retry metadata

**Key Fields:**
- `event_id` - Tilled event ID (unique, idempotency key)
- `status` - received, processing, processed, failed
- `attempt_count` - Number of processing attempts
- `last_attempt_at` - Timestamp of last retry
- `next_attempt_at` - Scheduled next retry (for worker)
- `dead_at` - Timestamp when moved to dead letter queue
- `error_code` - Error classification code

**Relations:** None

**Indexes:**
- `(app_id, status)`, `event_type`, `next_attempt_at`

---

## Phase 2: Reliability & Safety Tables (Skeleton)

### billing_idempotency_keys
**Purpose:** Prevent duplicate write operations

**Future Use:** Middleware will check this table before processing POST/PUT/PATCH requests. If idempotency key matches and request hash matches, return cached response. Implements standard idempotency pattern (Stripe-style).

**Key Fields:**
- `(app_id, idempotency_key)` - Composite unique key
- `request_hash` - SHA256 of method+path+body
- `response_body` - Cached response to replay
- `status_code` - HTTP status code
- `expires_at` - TTL (24 hours typical)

**Example:**
```http
POST /api/billing/subscriptions
Idempotency-Key: sub-create-abc123
{...}
```
→ If duplicate request with same key + hash, return cached 201 response

**Indexes:**
- `app_id`, `expires_at` (cleanup query)

---

### billing_events
**Purpose:** Forensics "black box recorder" for all system events

**Future Use:** Background job logs API requests, webhook receipts, system actions. Used for debugging, audit trails, and analytics. Payload redacted of sensitive data.

**Key Fields:**
- `event_type` - e.g., "api.subscription.cancel", "webhook.charge.failed"
- `source` - api, webhook, system, admin
- `entity_type` - customer, subscription, payment_method, invoice, charge
- `entity_id` - Numeric ID or Tilled ID (stored as string)
- `payload` - Event data (redacted)

**Example Events:**
```
api.subscription.create → User creates subscription via API
webhook.subscription.updated → Tilled sends update webhook
system.reconciliation.divergence → Drift detected
admin.subscription.override → Manual admin action
```

**Indexes:**
- `app_id`, `event_type`, `source`, `created_at`

---

### billing_webhook_attempts
**Purpose:** Track retry attempts with exponential backoff

**Future Use:** Worker processes failed webhooks with backoff:
- Attempt 1: Immediate
- Attempt 2: 1 minute later
- Attempt 3: 5 minutes later
- Attempt 4: 15 minutes later
- Dead letter: After 4 failures

**Key Fields:**
- `event_id` - Reference to webhook event
- `attempt_number` - 1, 2, 3, 4
- `status` - failed, scheduled, succeeded, dead
- `next_attempt_at` - When to retry next
- `error_code`, `error_message` - Failure details

**Indexes:**
- `app_id`, `event_id`, `status`, `next_attempt_at` (worker query)

---

### billing_reconciliation_runs
**Purpose:** Track drift detection job executions

**Future Use:** Scheduled job (daily) compares local DB to Tilled API:
1. Fetch all subscriptions from Tilled
2. Compare to local records
3. Log divergences in billing_divergences table
4. Record run stats (entities checked, divergences found, duration)

**Key Fields:**
- `status` - started, completed, failed
- `started_at`, `finished_at` - Execution timestamps
- `stats` - JSON summary (entities_checked, divergences_found, etc.)
- `error_message` - Failure details if status=failed

**Indexes:**
- `app_id`, `status`, `started_at`

---

### billing_divergences
**Purpose:** Record drift/inconsistencies between local DB and Tilled

**Future Use:** Admin dashboard shows divergences with resolution actions:
- **missing_local** - Exists in Tilled, not in local DB → Sync from Tilled
- **missing_remote** - Exists in local DB, not in Tilled → Orphaned record
- **field_mismatch** - Status/amount differs → Manual review

**Key Fields:**
- `run_id` - Reference to reconciliation run
- `entity_type` - customer, subscription, invoice, charge
- `entity_key` - ID to identify the record
- `divergence_type` - missing_local, missing_remote, field_mismatch
- `local_snapshot`, `remote_snapshot` - JSON snapshots for comparison
- `status` - open, resolved, ignored
- `resolved_at` - When divergence was fixed

**Relations:**
- → billing_reconciliation_runs (many-to-one, cascade delete)

**Indexes:**
- `app_id`, `run_id`, `entity_type`, `status`

---

## Phase 3: Pricing Agility Tables (Skeleton)

### billing_plans
**Purpose:** Plan definitions in database (replaces env entitlements)

**Future Use:** Admin UI for managing plans without code deployments:
```json
{
  "plan_id": "trashtech-pro-monthly",
  "name": "Pro Monthly",
  "price_cents": 9900,
  "interval_unit": "month",
  "features": {
    "analytics": true,
    "max_trucks": 10
  }
}
```

**Key Fields:**
- `(app_id, plan_id)` - Composite unique key
- `interval_unit`, `interval_count` - Billing frequency
- `price_cents`, `currency` - Pricing
- `features` - JSON entitlements map
- `active` - Enable/disable without deletion
- `version_tag` - Entitlements versioning (e.g., "2026-01-01")

**Indexes:**
- `app_id`, `active`

---

### billing_coupons
**Purpose:** Discount codes for promotions

**Future Use:** Apply discounts during subscription creation:
```javascript
POST /api/billing/subscriptions
{
  "plan_id": "pro-monthly",
  "coupon_code": "LAUNCH50",  // 50% off
  ...
}
```

**Key Fields:**
- `(app_id, code)` - Composite unique key
- `coupon_type` - percent (1-100) or amount (cents)
- `value` - Discount amount
- `duration` - once, repeating, forever
- `duration_months` - For repeating coupons (e.g., 3 months)
- `max_redemptions` - Usage limit
- `redeem_by` - Expiration date

**Indexes:**
- `app_id`, `active`, `redeem_by`

---

### billing_addons
**Purpose:** Optional features/upgrades that can be added to subscriptions

**Future Use:** TrashTech could offer:
- Extra trucks: +$10/month per truck
- Custom reporting: +$50/month
- API access: +$25/month

**Key Fields:**
- `(app_id, addon_id)` - Composite unique key
- `price_cents`, `currency` - Addon pricing
- `features` - Additional entitlements granted
- `active` - Enable/disable

**Indexes:**
- `app_id`, `active`

---

### billing_subscription_addons
**Purpose:** Join table for subscriptions with addons

**Future Use:**
```javascript
// Subscription has base plan + 2 addons
{
  "subscription_id": 10,
  "plan": "pro-monthly ($99)",
  "addons": [
    { "addon_id": "extra-truck", "quantity": 5, "price": "$50" },
    { "addon_id": "api-access", "quantity": 1, "price": "$25" }
  ],
  "total": "$174/month"
}
```

**Key Fields:**
- `subscription_id`, `addon_id` - Composite unique (no duplicates)
- `quantity` - Number of units (e.g., 5 extra trucks)

**Relations:**
- → billing_subscriptions (many-to-one, cascade delete)
- → billing_addons (many-to-one, cascade delete)

**Indexes:**
- `app_id`, `subscription_id`

---

## Phase 4: Money Records Tables (Skeleton)

### billing_invoices
**Purpose:** Invoice records synced from Tilled

**Future Use:** Track billing cycles, due dates, payment status:
- Generated monthly for subscriptions
- Can be sent to customers via hosted URL
- Status tracks payment lifecycle

**Key Fields:**
- `tilled_invoice_id` - Tilled invoice token (unique)
- `billing_customer_id`, `subscription_id` - Links
- `status` - open, paid, failed, void
- `amount_cents`, `currency` - Invoice amount
- `due_at`, `paid_at` - Timestamps
- `hosted_url` - Tilled-hosted invoice page

**Relations:**
- → billing_customers (many-to-one, cascade delete)
- → billing_subscriptions (many-to-one, set null if subscription deleted)
- → billing_charges (one-to-many)

**Indexes:**
- `app_id`, `billing_customer_id`, `subscription_id`, `status`, `due_at`

---

### billing_charges
**Purpose:** Payment attempt records (succeeded or failed)

**Future Use:** Track every payment attempt:
```
Subscription renewal → Invoice created → Charge attempted
  - Success: status=succeeded
  - Failure: status=failed + failure_code + failure_message
```

**Key Fields:**
- `tilled_charge_id` - Tilled charge token (unique)
- `invoice_id`, `billing_customer_id`, `subscription_id` - Links
- `status` - succeeded, failed, pending
- `amount_cents`, `currency` - Charge amount
- `failure_code` - Error code (card_declined, insufficient_funds, etc.)
- `failure_message` - Human-readable error

**Relations:**
- → billing_invoices (many-to-one, set null)
- → billing_customers (many-to-one, cascade delete)
- → billing_subscriptions (many-to-one, set null)
- → billing_refunds (one-to-many)
- → billing_disputes (one-to-many)

**Indexes:**
- `app_id`, `billing_customer_id`, `subscription_id`, `status`

---

### billing_refunds
**Purpose:** Refund records from Tilled

**Future Use:** Process refunds via API:
```javascript
POST /api/billing/refunds
{
  "charge_id": 123,
  "amount_cents": 9900,
  "reason": "Customer cancellation"
}
```

**Key Fields:**
- `tilled_refund_id` - Tilled refund token (unique)
- `charge_id` - Original charge being refunded
- `status` - pending, succeeded, failed
- `amount_cents` - Refund amount (partial or full)
- `reason` - requested_by_customer, duplicate, fraudulent, etc.

**Relations:**
- → billing_charges (many-to-one, cascade delete)

**Indexes:**
- `app_id`, `charge_id`, `status`

---

### billing_disputes
**Purpose:** Chargeback/dispute records from Tilled

**Future Use:** Track disputed charges:
- Customer initiates chargeback with bank
- Tilled notifies via webhook
- Status tracked until resolution
- Record stored for compliance/reporting

**Key Fields:**
- `tilled_dispute_id` - Tilled dispute token (unique)
- `charge_id` - Disputed charge
- `status` - open, won, lost, closed
- `amount_cents` - Dispute amount
- `reason` - fraudulent, unrecognized, product_not_received, etc.
- `opened_at`, `closed_at` - Lifecycle timestamps

**Relations:**
- → billing_charges (many-to-one, cascade delete)

**Indexes:**
- `app_id`, `charge_id`, `status`

---

## Migration Strategy

### Migration Order (FK-Safe)

**Step 1: Add columns to existing tables** (no FK dependencies)
```sql
ALTER TABLE billing_customers ADD COLUMN update_source VARCHAR(50);
ALTER TABLE billing_customers ADD COLUMN updated_by VARCHAR(255);
ALTER TABLE billing_customers ADD COLUMN delinquent_since TIMESTAMP;
ALTER TABLE billing_customers ADD COLUMN grace_period_end TIMESTAMP;

ALTER TABLE billing_subscriptions ADD COLUMN update_source VARCHAR(50);
ALTER TABLE billing_subscriptions ADD COLUMN updated_by VARCHAR(255);

ALTER TABLE billing_webhooks ADD COLUMN last_attempt_at TIMESTAMP;
ALTER TABLE billing_webhooks ADD COLUMN next_attempt_at TIMESTAMP;
ALTER TABLE billing_webhooks ADD COLUMN dead_at TIMESTAMP;
ALTER TABLE billing_webhooks ADD COLUMN error_code VARCHAR(50);
```

**Step 2: Create independent tables** (no FK dependencies)
```sql
CREATE TABLE billing_idempotency_keys (...);
CREATE TABLE billing_events (...);
CREATE TABLE billing_webhook_attempts (...);
CREATE TABLE billing_reconciliation_runs (...);
CREATE TABLE billing_plans (...);
CREATE TABLE billing_coupons (...);
CREATE TABLE billing_addons (...);
```

**Step 3: Create dependent tables** (FK to existing tables)
```sql
CREATE TABLE billing_invoices (...);
  -- FK → billing_customers, billing_subscriptions

CREATE TABLE billing_charges (...);
  -- FK → billing_customers, billing_subscriptions, billing_invoices

CREATE TABLE billing_subscription_addons (...);
  -- FK → billing_subscriptions, billing_addons

CREATE TABLE billing_divergences (...);
  -- FK → billing_reconciliation_runs
```

**Step 4: Create deeply dependent tables** (FK to Phase 4 tables)
```sql
CREATE TABLE billing_refunds (...);
  -- FK → billing_charges

CREATE TABLE billing_disputes (...);
  -- FK → billing_charges
```

### Applied Migrations

1. `20260123065209_add_phase1_payment_methods_and_fields` - Phase 1 (active)
2. `20260123151959_add_phase2_4_skeleton_tables` - Phase 2-4 skeleton (schema-only)

---

## Validation & Testing

### Database State
✅ Development database: Migration applied to `billing_db_sandbox`
✅ Test database: Migration applied to `billing_test`
✅ Prisma client regenerated successfully

### Test Results
✅ All 161 tests passing (104 unit + 57 integration)
✅ No breaking changes to existing functionality
✅ Prisma client can read/write to new tables

### No Runtime Behavior Changes
- No new routes added
- No new service methods added
- Existing endpoints unchanged
- All Phase 1 functionality works identically

---

## Future Implementation Notes

### Phase 2: Reliability & Safety (Next)

**Implementation Tasks:**
1. Add idempotency middleware to POST/PUT/PATCH routes
2. Create background worker for webhook retries (check `next_attempt_at`)
3. Create scheduled reconciliation job (daily drift detection)
4. Add event logging to all critical operations

**Estimated Effort:** 3-4 days TDD implementation

### Phase 3: Pricing Agility

**Implementation Tasks:**
1. Build admin UI for plan management (CRUD)
2. Update subscription creation to read from billing_plans table
3. Implement coupon redemption logic
4. Add addon attachment/detachment endpoints

**Estimated Effort:** 5-6 days TDD implementation

### Phase 4: Money Records

**Implementation Tasks:**
1. Add webhook handlers for invoice/charge/refund events
2. Create refund processing endpoint
3. Build dispute tracking UI
4. Add financial reporting endpoints

**Estimated Effort:** 4-5 days TDD implementation

---

## PCI Compliance Notes

### ✅ Safe to Store
- Payment method tokens from Tilled (pm_*, ch_*, etc.)
- Last 4 digits of cards/accounts
- Brand, expiration dates
- Bank names
- Masked metadata

### ❌ NEVER Store
- Full card numbers (16 digits)
- CVV/CVC codes
- Full account numbers
- Routing numbers (except last 4)
- Unencrypted cardholder data

All tables follow PCI DSS Level 1 requirements for tokenized payments.

---

## Questions & Support

**Schema Questions:** See this document
**Implementation Questions:** See phase-specific prompts (Phase 2, 3, 4)
**Production Deployment:** See `TRASHTECH-INTEGRATION-GUIDE.md`

**Status:** ✅ Skeleton Complete - Ready for Phase 2+ Implementation
