# Consumer Guide — Module APIs

> **Who reads this:** Agents building vertical apps on the 7D Platform.
> **What it covers:** Party Master, AR Module, and the canonical "First Invoice" flow.
> **Parent:** [PLATFORM-CONSUMER-GUIDE.md](./PLATFORM-CONSUMER-GUIDE.md)

## Contents

1. [Party Master](#party-master) — create company/individual, get, search, list, update, deactivate
2. [AR Module — Customers and Invoices](#ar-module--customers-and-invoices) — create AR customer, create invoice, lookup, all endpoints
3. [Complete "First Invoice" Flow](#complete-first-invoice-flow) — end-to-end sequence: register → login → party → AR customer → invoice

---

## Revision History

| Rev | Date | Changed By | Summary |
|-----|------|-----------|---------|
| 1.0 | 2026-02-20 | Platform Orchestrator | Extracted from PLATFORM-CONSUMER-GUIDE.md. Party Master endpoints, AR endpoints, complete first-invoice flow. |

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
