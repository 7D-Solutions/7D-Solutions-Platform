# Consumer Guide — Module APIs

> **Who reads this:** Agents building vertical apps on the 7D Platform.
> **What it covers:** Party Master (+ contacts, addresses), AR Module, Maintenance, Identity SoD, Notifications, and the canonical "First Invoice" flow.
> **Parent:** [PLATFORM-CONSUMER-GUIDE.md](./PLATFORM-CONSUMER-GUIDE.md)

## Contents

1. [Party Master](#party-master) — create company/individual, get, search, list, update, deactivate
2. [Party Contacts Extension](#party-contacts-extension) — create/list/update/delete contacts, set-primary, primary-contacts
3. [Party Addresses Extension](#party-addresses-extension) — create/list/get/update/delete addresses
4. [AR Module — Customers and Invoices](#ar-module--customers-and-invoices) — create AR customer, create invoice, lookup, all endpoints
5. [Maintenance Module](#maintenance-module) — assets, calibration, downtime, meters, work orders, plans
6. [Identity SoD (Segregation of Duties)](#identity-sod-segregation-of-duties) — policy CRUD, evaluate, decision log
7. [Notifications Module](#notifications-module) — templates, send, deliveries, inbox, DLQ
8. [Complete "First Invoice" Flow](#complete-first-invoice-flow) — end-to-end sequence: register → login → party → AR customer → invoice

---

## Revision History

| Rev | Date | Changed By | Summary |
|-----|------|-----------|---------|
| 1.0 | 2026-02-20 | Platform Orchestrator | Extracted from PLATFORM-CONSUMER-GUIDE.md. Party Master endpoints, AR endpoints, complete first-invoice flow. |
| 2.0 | 2026-03-04 | MaroonHarbor | Added Party Contacts/Addresses extension, Maintenance module, Identity SoD, Notifications module (templates, sends, inbox, DLQ). |

---

## Party Master

Source: `modules/party/src/http/party.rs`, `modules/party/src/domain/party/models.rs`

**Base URL:** `http://7d-party:8098`

`party_id` is the universal cross-module counterparty key. Create a party before creating AR customers, AP vendors, or any other counterparty record.

### Create a Company

```
POST /api/party/companies
x-app-id: <your-app-id>
Content-Type: application/json
```

Request body:
```json
{
  "display_name": "Acme Waste Management",
  "legal_name": "Acme Waste Management LLC",
  "trade_name": "Acme Waste",
  "registration_number": "TX-12345678",
  "tax_id": "12-3456789",
  "country_of_incorporation": "US",
  "industry_code": "5621",
  "email": "billing@acme.com",
  "phone": "+15551234567",
  "website": "https://acme.com",
  "address_line1": "123 Main St",
  "address_line2": "Suite 400",
  "city": "Austin",
  "state": "TX",
  "postal_code": "78701",
  "country": "US",
  "metadata": { "crm_id": "acme-001" }
}
```

Required fields: **`display_name`**, **`legal_name`**. All others are optional.

Response `201 Created` → `PartyView`:
```json
{
  "id": "a1b2c3d4-e5f6-7890-abcd-ef1234567890",
  "app_id": "<your-app-id>",
  "party_type": "company",
  "status": "active",
  "display_name": "Acme Waste Management",
  "email": "billing@acme.com",
  "phone": "+15551234567",
  "website": "https://acme.com",
  "address_line1": "123 Main St",
  "city": "Austin",
  "state": "TX",
  "postal_code": "78701",
  "country": "US",
  "metadata": { "crm_id": "acme-001" },
  "created_at": "2026-02-19T10:00:00Z",
  "updated_at": "2026-02-19T10:00:00Z",
  "legal_name": "Acme Waste Management LLC",
  "tax_id": "12-3456789",
  "external_refs": []
}
```

**The `id` field is your `party_id`. Store it everywhere.**

### Create an Individual

```
POST /api/party/individuals
x-app-id: <your-app-id>
Content-Type: application/json
```

Request body:
```json
{
  "display_name": "John Smith",
  "first_name": "John",
  "last_name": "Smith",
  "middle_name": "A",
  "email": "john@example.com",
  "phone": "+15559876543",
  "job_title": "Driver",
  "department": "Operations",
  "metadata": { "employee_id": "D-0042" }
}
```

Required fields: **`display_name`**, **`first_name`**, **`last_name`**. All others optional.

Response: Same `PartyView` shape, `party_type` = `"individual"`.

### Get a Party

```
GET /api/party/parties/{party_id}
x-app-id: <your-app-id>
```

Response: `PartyView` (200) or `{ "error": "not_found", "message": "..." }` (404).

### Search Parties

```
GET /api/party/parties/search?name=Acme&party_type=company&limit=20&offset=0
x-app-id: <your-app-id>
```

Query parameters (all optional): `name` (partial match), `party_type` (`company`|`individual`|`contact`), `status` (`active`|`inactive`), `external_system`, `external_id`, `limit` (default 50, max 200), `offset`.

### List Parties

```
GET /api/party/parties
x-app-id: <your-app-id>
```

### Update a Party

```
PUT /api/party/parties/{party_id}
x-app-id: <your-app-id>
Content-Type: application/json
```

Send only the fields you want to update.

### Deactivate a Party

```
POST /api/party/parties/{party_id}/deactivate
x-app-id: <your-app-id>
```

Response: `204 No Content`.

---

## Party Contacts Extension

Source: `modules/party/src/http/contacts.rs`, `modules/party/src/domain/contact.rs`

**Base URL:** `http://7d-party:8098`

Contacts are named people linked to a party (e.g. billing contact, operations manager). Mutation routes require `party.mutate` permission.

### Create a Contact

```
POST /api/party/parties/{party_id}/contacts
Authorization: Bearer <jwt>
x-correlation-id: <uuid>
Content-Type: application/json
```

Request body:
```json
{
  "first_name": "Jane",
  "last_name": "Doe",
  "email": "jane@acme.com",
  "phone": "+15551234567",
  "role": "billing",
  "is_primary": true,
  "metadata": { "department": "finance" }
}
```

Required fields: **`first_name`**, **`last_name`**. All others optional.

Response `201 Created` → `Contact`:
```json
{
  "id": "c1d2e3f4-...",
  "party_id": "a1b2c3d4-...",
  "app_id": "<your-app-id>",
  "first_name": "Jane",
  "last_name": "Doe",
  "email": "jane@acme.com",
  "phone": "+15551234567",
  "role": "billing",
  "is_primary": true,
  "metadata": { "department": "finance" },
  "created_at": "2026-03-04T10:00:00Z",
  "updated_at": "2026-03-04T10:00:00Z",
  "deactivated_at": null
}
```

### List Contacts for a Party

```
GET /api/party/parties/{party_id}/contacts
Authorization: Bearer <jwt>
```

Response: array of `Contact` objects.

### Get a Contact

```
GET /api/party/contacts/{id}
Authorization: Bearer <jwt>
```

### Update a Contact

```
PUT /api/party/contacts/{id}
Authorization: Bearer <jwt>
x-correlation-id: <uuid>
Content-Type: application/json
```

Send only the fields you want to update:
```json
{
  "email": "jane.doe@acme.com",
  "role": "operations"
}
```

### Deactivate a Contact (Soft-Delete)

```
DELETE /api/party/contacts/{id}
Authorization: Bearer <jwt>
x-correlation-id: <uuid>
```

Response: `204 No Content`. Sets `deactivated_at` timestamp.

### Set Primary Contact for a Role

```
POST /api/party/parties/{party_id}/contacts/{id}/set-primary
Authorization: Bearer <jwt>
x-correlation-id: <uuid>
Content-Type: application/json
```

```json
{ "role": "billing" }
```

Marks this contact as the primary for the given role on the party. Any previous primary for that role is demoted.

### Get Primary Contacts Map

```
GET /api/party/parties/{party_id}/primary-contacts
Authorization: Bearer <jwt>
```

Response: array of `{ "role": "billing", "contact": { ... } }` entries — one per role that has a designated primary.

---

## Party Addresses Extension

Source: `modules/party/src/http/addresses.rs`

**Base URL:** `http://7d-party:8098`

Addresses linked to a party. Mutation routes require `party.mutate` permission.

```
POST   /api/party/parties/{party_id}/addresses       — create address
GET    /api/party/parties/{party_id}/addresses       — list addresses for a party
GET    /api/party/addresses/{id}                     — get address
PUT    /api/party/addresses/{id}                     — update address
DELETE /api/party/addresses/{id}                     — delete address
```

Headers: same as contacts (Bearer JWT, x-correlation-id for mutations).

---

## AR Module — Customers and Invoices

Source: `modules/ar/src/models/customer.rs`, `modules/ar/src/models/invoice.rs`, `modules/ar/src/routes/customers.rs`

**Base URL:** `http://7d-ar:8086`

### Step 1: Create an AR Customer

**You must do this before creating invoices.**

```
POST /api/ar/customers
x-app-id: <your-app-id>
x-tenant-id: <tenant-uuid>
Authorization: Bearer <jwt>
x-correlation-id: <uuid>
x-actor-id: <uuid>
Content-Type: application/json
```

Request body:
```json
{
  "email": "billing@acme.com",
  "name": "Acme Waste Management",
  "external_customer_id": "crm-acme-001",
  "party_id": "a1b2c3d4-e5f6-7890-abcd-ef1234567890",
  "metadata": { "segment": "enterprise" }
}
```

Field notes:
- `email`: **Effectively required** — the create handler validates it is non-empty and contains `@`
- `name`: Optional display name
- `external_customer_id`: Optional — your internal ID for this customer (useful for lookup)
- `party_id`: Optional UUID — link to Party Master record

Response `201 Created`:
```json
{
  "id": 42,
  "app_id": "<your-app-id>",
  "email": "billing@acme.com",
  "name": "Acme Waste Management",
  "external_customer_id": "crm-acme-001",
  "status": "active",
  "party_id": "a1b2c3d4-e5f6-7890-abcd-ef1234567890",
  "tilled_customer_id": null,
  "default_payment_method_id": null,
  "payment_method_type": null,
  "metadata": { "segment": "enterprise" },
  "retry_attempt_count": 0,
  "created_at": "2026-02-19T10:00:00Z",
  "updated_at": "2026-02-19T10:00:00Z"
}
```

**The `id` field (integer) is your `ar_customer_id`. You need this for invoice creation.**

Error cases:
- `400 Bad Request`: email missing or invalid
- `409 Conflict`: email already exists for this `app_id`

### Step 2: Create an Invoice

```
POST /api/ar/invoices
x-app-id: <your-app-id>
x-tenant-id: <tenant-uuid>
Authorization: Bearer <jwt>
x-correlation-id: <uuid>
x-actor-id: <uuid>
Content-Type: application/json
```

Request body:
```json
{
  "ar_customer_id": 42,
  "amount_cents": 15000,
  "currency": "usd",
  "due_at": "2026-03-19T00:00:00",
  "party_id": "a1b2c3d4-e5f6-7890-abcd-ef1234567890",
  "billing_period_start": "2026-02-01T00:00:00",
  "billing_period_end": "2026-02-28T23:59:59",
  "line_item_details": [
    { "description": "Trash pickup - 10 stops", "amount_cents": 15000 }
  ],
  "metadata": { "job_id": "job-8812" }
}
```

Required fields: **`ar_customer_id`** (integer from Step 1), **`amount_cents`** (integer, in cents).

Optional fields: `subscription_id`, `status` (default: `"draft"`), `currency` (default: `"usd"`), `due_at`, `billing_period_start`, `billing_period_end`, `line_item_details` (JSON), `compliance_codes` (JSON), `correlation_id`, `party_id`, `metadata`.

Invoice status values: `"draft"` | `"open"` | `"paid"` | `"void"` | `"uncollectible"`

Response: `201 Created` with full invoice object including `id` (integer), `tilled_invoice_id`, `status`, `amount_cents`, etc.

### Lookup AR Customer by External ID

```
GET /api/ar/customers?external_customer_id=crm-acme-001&limit=1
x-app-id: <your-app-id>
```

Use this to check if you already created an AR customer before creating a duplicate.

### Other AR Endpoints

Source: `modules/ar/src/routes/mod.rs`

Mutation routes (require `ar.mutate` permission):
```
POST   /api/ar/invoices/{id}/finalize
POST   /api/ar/invoices/{id}/bill-usage
POST   /api/ar/invoices/{id}/credit-notes
POST   /api/ar/invoices/{id}/write-off
POST   /api/ar/charges
POST   /api/ar/refunds
POST   /api/ar/disputes
POST   /api/ar/payment-methods
POST   /api/ar/webhooks
POST   /api/ar/usage
POST   /api/ar/dunning/run
POST   /api/ar/allocation/run
POST   /api/ar/reconciliation/run
POST   /api/ar/tax-config
```

Read routes:
```
GET    /api/ar/invoices
GET    /api/ar/invoices/{id}
GET    /api/ar/customers
GET    /api/ar/customers/{id}
GET    /api/ar/subscriptions
GET    /api/ar/aging
```

---

## Maintenance Module

Source: `modules/maintenance/src/http/`, `modules/maintenance/src/main.rs`

**Base URL:** `http://7d-maintenance:8101`

Full CMMS (Computerized Maintenance Management System). Read routes require `maintenance.read`, mutation routes require `maintenance.mutate`.

### Assets

```
POST  /api/maintenance/assets                        — create asset
GET   /api/maintenance/assets                        — list assets (?asset_type=, &status=, &limit=, &offset=)
GET   /api/maintenance/assets/{asset_id}             — get asset detail
PATCH /api/maintenance/assets/{asset_id}             — update asset
```

### Calibration

```
POST /api/maintenance/assets/{asset_id}/calibration-events   — record calibration event
GET  /api/maintenance/assets/{asset_id}/calibration-status   — get current calibration status
```

### Downtime Events

```
POST /api/maintenance/downtime-events                — create downtime event
GET  /api/maintenance/downtime-events                — list all downtime events
GET  /api/maintenance/downtime-events/{id}           — get downtime event
GET  /api/maintenance/assets/{asset_id}/downtime     — list downtime for a specific asset
```

### Meters

```
POST /api/maintenance/meter-types                    — create meter type
GET  /api/maintenance/meter-types                    — list meter types
POST /api/maintenance/assets/{asset_id}/readings     — record meter reading
GET  /api/maintenance/assets/{asset_id}/readings     — list readings for asset
```

### Maintenance Plans

```
POST  /api/maintenance/plans                         — create plan
GET   /api/maintenance/plans                         — list plans
GET   /api/maintenance/plans/{plan_id}               — get plan detail
PATCH /api/maintenance/plans/{plan_id}               — update plan
POST  /api/maintenance/plans/{plan_id}/assign        — assign plan to asset
GET   /api/maintenance/assignments                   — list all plan assignments
```

### Work Orders

```
POST  /api/maintenance/work-orders                   — create work order
GET   /api/maintenance/work-orders                   — list work orders
GET   /api/maintenance/work-orders/{wo_id}           — get work order detail
PATCH /api/maintenance/work-orders/{wo_id}/transition — transition status (e.g. open → in_progress → completed)
```

### Work Order Parts & Labor

```
POST   /api/maintenance/work-orders/{wo_id}/parts            — add part to work order
GET    /api/maintenance/work-orders/{wo_id}/parts            — list parts on work order
DELETE /api/maintenance/work-orders/{wo_id}/parts/{part_id}  — remove part

POST   /api/maintenance/work-orders/{wo_id}/labor            — add labor entry
GET    /api/maintenance/work-orders/{wo_id}/labor            — list labor entries
DELETE /api/maintenance/work-orders/{wo_id}/labor/{labor_id} — remove labor entry
```

All mutation routes require `Authorization: Bearer <jwt>` with `maintenance.mutate` permission. Read routes require `maintenance.read`.

---

## Identity SoD (Segregation of Duties)

Source: `platform/identity-auth/src/routes/auth.rs`, `platform/identity-auth/src/db/sod.rs`

**Base URL:** `http://7d-auth-lb:8080`

Separation of Duties enforcement. Prevents single-user approval of conflicting actions.

### Upsert SoD Policy

```
POST /api/auth/sod/policies
Content-Type: application/json
```

Request body:
```json
{
  "tenant_id": "550e8400-...",
  "action_key": "approve_purchase_order",
  "primary_role_id": "role-uuid-1",
  "conflicting_role_id": "role-uuid-2",
  "allow_override": false,
  "override_requires_approval": true,
  "actor_user_id": "user-uuid",
  "idempotency_key": "sod-policy-123",
  "causation_id": "cause-uuid"
}
```

Required: **`tenant_id`**, **`action_key`**, **`primary_role_id`**, **`conflicting_role_id`**, **`allow_override`**, **`override_requires_approval`**. Others optional.

Response `200 OK` → policy object with `idempotent_replay` flag.

### Evaluate SoD

```
POST /api/auth/sod/evaluate
Content-Type: application/json
```

Request body:
```json
{
  "tenant_id": "550e8400-...",
  "action_key": "approve_purchase_order",
  "actor_user_id": "user-uuid",
  "subject_user_id": "other-user-uuid",
  "override_granted_by": null,
  "override_ticket": null,
  "idempotency_key": "eval-123",
  "causation_id": "cause-uuid"
}
```

Required: **`tenant_id`**, **`action_key`**, **`actor_user_id`**. Others optional.

Response: decision result including `allowed` boolean and matched policies.

### List SoD Policies

```
GET /api/auth/sod/policies/{tenant_id}/{action_key}
```

### Delete SoD Policy

```
DELETE /api/auth/sod/policies/{tenant_id}/{rule_id}
```

---

## Notifications Module

Source: `modules/notifications/src/http/`, `modules/notifications/src/main.rs`

**Base URL:** `http://7d-notifications:8089`

Multi-channel notification system with versioned templates, delivery tracking, in-app inbox, and dead-letter queue. Read routes require `notifications.read`, mutation routes require `notifications.mutate`.

### Templates

```
POST /api/templates                     — publish new template version
GET  /api/templates/{key}               — get latest template + version history
```

Publish request:
```json
{
  "template_key": "invoice_overdue",
  "channel": "email",
  "subject": "Invoice {{invoice_number}} is overdue",
  "body": "Dear {{customer_name}}, your invoice {{invoice_number}} for {{amount}} is past due.",
  "required_vars": ["invoice_number", "customer_name", "amount"]
}
```

Response `201 Created`:
```json
{
  "id": "uuid",
  "template_key": "invoice_overdue",
  "version": 1,
  "channel": "email",
  "subject": "Invoice {{invoice_number}} is overdue",
  "body": "Dear {{customer_name}}...",
  "required_vars": ["invoice_number", "customer_name", "amount"],
  "created_at": "2026-03-04T10:00:00Z"
}
```

### Send a Notification

```
POST /api/notifications/send
Authorization: Bearer <jwt>
Content-Type: application/json
```

Request body:
```json
{
  "template_key": "invoice_overdue",
  "channel": "email",
  "recipients": ["billing@acme.com"],
  "payload_json": {
    "invoice_number": "INV-001",
    "customer_name": "Acme Corp",
    "amount": "$1,500.00"
  },
  "correlation_id": "uuid",
  "causation_id": "uuid"
}
```

Required: **`template_key`**, **`channel`**, **`recipients`**, **`payload_json`**. Others optional.

Response `201 Created`:
```json
{
  "id": "send-uuid",
  "status": "delivered",
  "template_key": "invoice_overdue",
  "template_version": 1,
  "channel": "email",
  "rendered_hash": "abcdef0123456789",
  "receipt_count": 1
}
```

### Get Send Detail + Receipts

```
GET /api/notifications/{id}
Authorization: Bearer <jwt>
```

Response includes `receipts` array with per-recipient delivery status.

### Query Delivery Receipts

```
GET /api/deliveries?correlation_id=&recipient=&from=&to=&limit=50&offset=0
Authorization: Bearer <jwt>
```

### In-App Inbox

```
GET  /api/inbox?user_id=<uid>&unread_only=true&category=billing&page_size=25&offset=0
GET  /api/inbox/{id}?user_id=<uid>
POST /api/inbox/{id}/read?user_id=<uid>
POST /api/inbox/{id}/unread?user_id=<uid>
POST /api/inbox/{id}/dismiss?user_id=<uid>
POST /api/inbox/{id}/undismiss?user_id=<uid>
```

### Dead-Letter Queue (DLQ)

Operator endpoints for notifications that exhausted retry attempts.

```
GET  /api/dlq?limit=50&offset=0&channel=email&template_key=invoice_overdue
GET  /api/dlq/{id}                     — detail + delivery attempts history
POST /api/dlq/{id}/replay              — reset to pending for re-dispatch
POST /api/dlq/{id}/abandon             — mark as permanently abandoned
```

### Admin Endpoints

Require `X-Admin-Token` header.

```
POST /api/notifications/admin/projection-status
POST /api/notifications/admin/consistency-check
GET  /api/notifications/admin/projections
```

---

## Complete "First Invoice" Flow

This is the canonical sequence for billing a customer for the first time.

```
1. Register user (once per user):
   POST http://7d-auth-lb:8080/api/auth/register
   Body: { tenant_id, user_id, email, password }

2. Login to get JWT:
   POST http://7d-auth-lb:8080/api/auth/login
   Body: { tenant_id, email, password }
   → save access_token, refresh_token

3. Create party in Party Master (once per customer entity):
   POST http://7d-party:8098/api/party/companies
   Headers: x-app-id, x-tenant-id, Authorization: Bearer <jwt>, x-correlation-id, x-actor-id
   Body: { display_name, legal_name, email, ... }
   → save party_id (UUID)

4. Create AR customer (once per billing relationship):
   POST http://7d-ar:8086/api/ar/customers
   Headers: x-app-id, x-tenant-id, Authorization: Bearer <jwt>, x-correlation-id, x-actor-id
   Body: { email, name, party_id: <from step 3>, external_customer_id }
   → save ar_customer_id (integer)

5. Create invoice:
   POST http://7d-ar:8086/api/ar/invoices
   Headers: x-app-id, x-tenant-id, Authorization: Bearer <jwt>, x-correlation-id, x-actor-id
   Body: { ar_customer_id: <from step 4>, amount_cents, party_id: <from step 3>, ... }
   → invoice created
```

**In your operational DB:** Store `party_id` (UUID) and `ar_customer_id` (integer) on your domain tables so you can reference them without re-querying Party Master or AR.

Example schema for your domain entity table (rename `orders` to your domain concept):
```sql
CREATE TABLE orders (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    customer_party_id UUID NOT NULL,     -- Party Master party_id
    ar_customer_id INTEGER,               -- AR module ar_customer_id; NULL until billing relationship established
    status TEXT NOT NULL DEFAULT 'pending',
    scheduled_at TIMESTAMPTZ,
    -- ... your domain fields
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
```

---

> See `docs/PLATFORM-CONSUMER-GUIDE.md` for the master index and critical concepts.
