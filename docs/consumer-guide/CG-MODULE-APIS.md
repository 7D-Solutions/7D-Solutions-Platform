# Consumer Guide — Module APIs

> **Who reads this:** Agents building vertical apps on the 7D Platform.
> **What it covers:** Party Master (+ contacts, addresses), AR Module, GL Module, Inventory Module, Subscriptions Module, Payments Module, AP Module, TTP Module, Treasury Module, Fixed Assets Module, Consolidation Module, BOM Module, Production Module, Quality Inspection Module, Numbering Module, Workflow Module, Maintenance, Identity SoD, Notifications, and the canonical "First Invoice" flow.
> **Parent:** [PLATFORM-CONSUMER-GUIDE.md](./PLATFORM-CONSUMER-GUIDE.md)

## Contents

1. [Party Master](#party-master) — create company/individual, get, search, list, update, deactivate
2. [Party Contacts Extension](#party-contacts-extension) — create/list/update/delete contacts, set-primary, primary-contacts
3. [Party Addresses Extension](#party-addresses-extension) — create/list/get/update/delete addresses
4. [AR Module — Customers and Invoices](#ar-module--customers-and-invoices) — create AR customer, create invoice, lookup, all endpoints
5. [GL Module](#gl-module) — trial balance, income statement, balance sheet, cash flow, period close, FX rates, accruals, revenue recognition, exports
6. [Inventory Module](#inventory-module) — items, receipts, issues, transfers, adjustments, reservations, locations, lots/serials, cycle counts, valuation, reorder, revisions, labels, expiry, genealogy
7. [Subscriptions Module](#subscriptions-module) — bill run execution
8. [Payments Module](#payments-module) — checkout sessions, webhooks, payment queries
9. [AP Module](#ap-module--accounts-payable) — vendors, purchase orders, bills, 3-way matching, payment terms, payment runs, aging, tax reports
10. [TTP Module](#ttp-module--third-party-pricing) — billing runs, metering events, service agreements, price trace
11. [Treasury Module](#treasury-module) — bank/credit-card accounts, reconciliation, GL linkage, statement import, cash position, forecast
12. [Fixed Assets Module](#fixed-assets-module) — categories, assets, depreciation schedules/runs, disposals
13. [Consolidation Module](#consolidation-module) — groups, entities, COA mappings, elimination rules, FX policies, intercompany matching, consolidated statements
14. [BOM Module](#bom-module) — BOM headers, revisions, effectivity, lines, explosion, where-used, ECOs
15. [Production Module](#production-module) — workcenters, work orders, routings, operations, component issues, FG receipts, time entries, downtime
16. [Quality Inspection Module](#quality-inspection-module) — inspection plans, receiving/in-process/final inspections, disposition transitions, queries
17. [Numbering Module](#numbering-module) — sequence allocation, gap-free reservations, confirmation, formatting policies
18. [Workflow Module](#workflow-module) — definitions, instances, transitions, advance
19. [Maintenance Module](#maintenance-module) — assets, calibration, downtime, meters, work orders, plans
20. [Identity SoD (Segregation of Duties)](#identity-sod-segregation-of-duties) — policy CRUD, evaluate, decision log
21. [Notifications Module](#notifications-module) — templates, send, deliveries, inbox, DLQ
22. [Complete "First Invoice" Flow](#complete-first-invoice-flow) — end-to-end sequence: register → login → party → AR customer → invoice

---

## Revision History

| Rev | Date | Changed By | Summary |
|-----|------|-----------|---------|
| 1.0 | 2026-02-20 | Platform Orchestrator | Extracted from PLATFORM-CONSUMER-GUIDE.md. Party Master endpoints, AR endpoints, complete first-invoice flow. |
| 2.0 | 2026-03-04 | MaroonHarbor | Added Party Contacts/Addresses extension, Maintenance module, Identity SoD, Notifications module (templates, sends, inbox, DLQ). |
| 3.0 | 2026-03-12 | DarkOwl | Added GL module (financial reports, period close, FX rates, accruals, revrec, exports), Inventory module (items, receipts, issues, transfers, reservations, lots/serials, locations, cycle counts, valuation, reorder, revisions, labels, expiry, genealogy), Subscriptions module (bill runs), Payments module (checkout sessions, webhooks). |
| 4.0 | 2026-03-11 | DarkOwl | Added AP module (vendors, POs, bills, 3-way matching, payment terms, payment runs, aging, tax reports), TTP module (billing runs, metering, service agreements), Treasury module (accounts, reconciliation, GL linkage, statement import, cash position, forecast), Fixed Assets module (categories, assets, depreciation, disposals), Consolidation module (groups, entities, COA mappings, eliminations, FX policies, intercompany, consolidated statements). |
| 5.0 | 2026-03-11 | DarkOwl | Added BOM module (headers, revisions, effectivity, lines, explosion, where-used, ECOs with lifecycle), Production module (workcenters, work orders, routings, operations, component issues, FG receipts, time entries, downtime), Quality Inspection module (plans, receiving/in-process/final inspections, disposition state machine), Numbering module (allocation, gap-free reservations, confirmation, formatting policies), Workflow module (definitions, instances, transitions). |

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

## GL Module

Source: `modules/gl/src/http/`, `modules/gl/src/main.rs`

**Base URL:** `http://7d-gl:8090`

General Ledger — financial reporting, period close lifecycle, FX rates, accruals, revenue recognition, and data exports. Tenant identity derived from JWT claims. Read routes are open; mutation routes require `gl.post` permission.

### Financial Reports

```
GET /api/gl/trial-balance?period_id={uuid}&currency=USD
GET /api/gl/income-statement?period_id={uuid}&currency=USD
GET /api/gl/balance-sheet?period_id={uuid}&currency=USD
GET /api/gl/cash-flow?period_id={uuid}&currency=USD
GET /api/gl/detail?period_id={uuid}&account_code={code}
GET /api/gl/accounts/{account_code}/activity?period_id={uuid}
GET /api/gl/periods/{period_id}/summary
Authorization: Bearer <jwt>
```

All report endpoints accept `period_id` (UUID) and optional `currency` (ISO 4217, defaults to USD).

### Reporting Currency Reports

Multi-currency reporting — same reports translated into the tenant's reporting currency.

```
GET /api/gl/reporting/trial-balance?period_id={uuid}
GET /api/gl/reporting/income-statement?period_id={uuid}
GET /api/gl/reporting/balance-sheet?period_id={uuid}
Authorization: Bearer <jwt>
```

### Period Close Lifecycle

```
POST /api/gl/periods/{period_id}/validate-close   — pre-flight validation (read-only)
POST /api/gl/periods/{period_id}/close             — atomically close period
GET  /api/gl/periods/{period_id}/close-status      — query close status
Authorization: Bearer <jwt>
```

**Validate close** request body:
```json
{}
```

Response `200 OK`:
```json
{
  "period_id": "uuid",
  "tenant_id": "tenant-abc",
  "can_close": true,
  "validation_report": [ ... ],
  "validated_at": "2026-03-12T00:00:00Z"
}
```

**Close period** request body:
```json
{
  "closed_by": "user-uuid",
  "close_reason": "Month-end close March 2026"
}
```

Response `200 OK`:
```json
{
  "period_id": "uuid",
  "tenant_id": "tenant-abc",
  "success": true,
  "close_status": { ... },
  "validation_report": [ ... ],
  "timestamp": "2026-03-12T00:00:00Z"
}
```

Error cases:
- `404 Not Found`: period not found for tenant
- `409 Conflict`: period already closed (idempotent — returns existing close status)

### Close Checklist & Approvals

```
POST /api/gl/periods/{period_id}/checklist                         — create checklist item
GET  /api/gl/periods/{period_id}/checklist                         — get checklist status
POST /api/gl/periods/{period_id}/checklist/{item_id}/complete      — mark item complete
POST /api/gl/periods/{period_id}/checklist/{item_id}/waive         — waive item
POST /api/gl/periods/{period_id}/approvals                         — create approval
GET  /api/gl/periods/{period_id}/approvals                         — list approvals
Authorization: Bearer <jwt>
```

### Period Reopen

```
POST /api/gl/periods/{period_id}/reopen                            — request controlled reopen
GET  /api/gl/periods/{period_id}/reopen                            — list reopen requests
POST /api/gl/periods/{period_id}/reopen/{request_id}/approve       — approve reopen
POST /api/gl/periods/{period_id}/reopen/{request_id}/reject        — reject reopen
Authorization: Bearer <jwt>
```

**Request reopen** body:
```json
{
  "requested_by": "user-uuid",
  "reason": "Missed accrual entry for vendor invoice"
}
```

### FX Rates

```
POST /api/gl/fx-rates            — create FX rate (idempotent on idempotency_key)
GET  /api/gl/fx-rates/latest     — get latest rate for currency pair
Authorization: Bearer <jwt>
```

**Create FX rate** request body:
```json
{
  "base_currency": "EUR",
  "quote_currency": "USD",
  "rate": 1.0850,
  "effective_at": "2026-03-12T00:00:00Z",
  "source": "ECB",
  "idempotency_key": "fx-eur-usd-20260312"
}
```

Response `200 OK`:
```json
{
  "rate_id": "uuid",
  "created": true
}
```

**Get latest rate** query:
```
GET /api/gl/fx-rates/latest?base_currency=EUR&quote_currency=USD&as_of=2026-03-12T00:00:00Z
```

Response:
```json
{
  "id": "uuid",
  "tenant_id": "tenant-abc",
  "base_currency": "EUR",
  "quote_currency": "USD",
  "rate": 1.085,
  "inverse_rate": 0.9217,
  "effective_at": "2026-03-12T00:00:00Z",
  "source": "ECB",
  "created_at": "2026-03-12T00:00:00Z"
}
```

### Revenue Recognition (Revrec)

```
POST /api/gl/revrec/contracts          — create revenue contract with obligations
POST /api/gl/revrec/schedules          — generate recognition schedule for an obligation
POST /api/gl/revrec/recognition-runs   — execute recognition run for a period
POST /api/gl/revrec/amendments         — amend a contract (mid-cycle versioning)
Authorization: Bearer <jwt>
```

All revrec endpoints are idempotent — duplicates return `409 Conflict`.

**Create contract** request body:
```json
{
  "contract_id": "uuid",
  "customer_id": "cust-001",
  "contract_name": "Annual support agreement",
  "contract_start": "2026-01-01",
  "contract_end": "2026-12-31",
  "total_transaction_price_minor": 1200000,
  "currency": "USD",
  "performance_obligations": [
    {
      "obligation_id": "uuid",
      "name": "Premium support",
      "allocated_amount_minor": 1200000,
      "recognition_pattern": { "type": "straight_line" },
      "satisfaction_start": "2026-01-01",
      "satisfaction_end": "2026-12-31"
    }
  ]
}
```

**Recognition run** request body:
```json
{
  "period": "2026-03",
  "posting_date": "2026-03-31"
}
```

### Accruals

```
POST /api/gl/accruals/templates           — create accrual template
POST /api/gl/accruals/create              — create accrual instance from template
POST /api/gl/accruals/reversals/execute   — execute auto-reversals for target period
Authorization: Bearer <jwt>
```

### Exports

```
POST /api/gl/exports
Authorization: Bearer <jwt>
```

Request body:
```json
{
  "format": "quickbooks",
  "export_type": "chart_of_accounts",
  "idempotency_key": "export-coa-20260312",
  "period_id": "uuid"
}
```

Supported formats: `quickbooks`, `xero`. Export types: `chart_of_accounts`, `journal_entries`. `period_id` required for `journal_entries`.

---

## Inventory Module

Source: `modules/inventory/src/http/`, `modules/inventory/src/main.rs`

**Base URL:** `http://7d-inventory:8092`

Inventory management — item master, stock movements (receipts/issues/transfers/adjustments), reservations, lot/serial tracking, cycle counts, valuation snapshots, reorder policies, item revisions, labels, expiry management, and lot genealogy. Tenant identity derived from JWT claims. Read routes require `inventory.read`, mutation routes require `inventory.mutate`.

### Item Master

```
POST  /api/inventory/items                   — create item
GET   /api/inventory/items/{id}              — get item
PUT   /api/inventory/items/{id}              — update item
POST  /api/inventory/items/{id}/deactivate   — soft-delete item
PUT   /api/inventory/items/{id}/make-buy     — set make/buy classification
Authorization: Bearer <jwt>
```

**Create item** request body:
```json
{
  "sku": "WIDGET-001",
  "name": "Standard Widget",
  "description": "10mm steel widget",
  "inventory_account_ref": "1200",
  "cogs_account_ref": "5000",
  "variance_account_ref": "5010",
  "uom": "ea",
  "tracking_mode": "lot",
  "make_buy": "buy"
}
```

Required fields: **`sku`**, **`name`**, **`inventory_account_ref`**, **`cogs_account_ref`**, **`variance_account_ref`**, **`tracking_mode`** (`none` | `lot` | `serial`).

Response `201 Created` → Item object with `id` (UUID), `tenant_id`, `sku`, `name`, `active`, `tracking_mode`, `make_buy`, GL account refs, `created_at`, `updated_at`.

Error cases:
- `409 Conflict`: SKU already exists for tenant
- `422 Unprocessable Entity`: validation failure

### Stock Receipts

```
POST /api/inventory/receipts
Authorization: Bearer <jwt>
```

Request body:
```json
{
  "item_id": "uuid",
  "warehouse_id": "uuid",
  "location_id": "uuid",
  "quantity": 100,
  "unit_cost_minor": 1500,
  "currency": "USD",
  "source_type": "purchase",
  "purchase_order_id": "uuid",
  "idempotency_key": "rcpt-20260312-001",
  "lot_code": "LOT-2026-03A",
  "serial_codes": null,
  "uom_id": "uuid"
}
```

Required fields: **`item_id`**, **`warehouse_id`**, **`quantity`** (>0), **`unit_cost_minor`** (>0), **`currency`**, **`idempotency_key`**. `lot_code` required for lot-tracked items. `serial_codes` required for serial-tracked items (length must equal quantity).

Source types: `purchase` (default), `production`, `return`.

Response `201 Created` / `200 OK` (idempotency replay):
```json
{
  "receipt_line_id": "uuid",
  "ledger_entry_id": 42,
  "layer_id": "uuid",
  "event_id": "uuid",
  "tenant_id": "tenant-abc",
  "item_id": "uuid"
}
```

### Stock Issues

```
POST /api/inventory/issues
Authorization: Bearer <jwt>
```

Issues consume stock using FIFO costing. Same pattern as receipts — requires `item_id`, `warehouse_id`, `quantity`, `idempotency_key`.

### Transfers

```
POST /api/inventory/transfers           — transfer stock between warehouses
POST /api/inventory/status-transfers    — transfer stock between statuses
Authorization: Bearer <jwt>
```

### Adjustments

```
POST /api/inventory/adjustments
Authorization: Bearer <jwt>
```

### Reservations

```
POST /api/inventory/reservations/reserve       — reserve stock for a demand source
POST /api/inventory/reservations/release       — release a reservation
POST /api/inventory/reservations/{id}/fulfill  — fulfill a reservation (issue stock)
Authorization: Bearer <jwt>
```

### Locations

```
POST  /api/inventory/locations                              — create location
GET   /api/inventory/locations/{id}                         — get location
PUT   /api/inventory/locations/{id}                         — update location
POST  /api/inventory/locations/{id}/deactivate              — soft-delete location
GET   /api/inventory/warehouses/{warehouse_id}/locations    — list locations in warehouse
Authorization: Bearer <jwt>
```

### Units of Measure (UoM)

```
POST /api/inventory/uoms                           — create UoM
GET  /api/inventory/uoms                           — list UoMs
POST /api/inventory/items/{id}/uom-conversions     — create UoM conversion for item
GET  /api/inventory/items/{id}/uom-conversions     — list UoM conversions for item
Authorization: Bearer <jwt>
```

### Lot & Serial Queries

```
GET /api/inventory/items/{item_id}/lots                              — list lots for item
GET /api/inventory/items/{item_id}/serials                           — list serials for item
GET /api/inventory/items/{item_id}/lots/{lot_code}/trace             — trace lot through movements
GET /api/inventory/items/{item_id}/serials/{serial_code}/trace       — trace serial through movements
Authorization: Bearer <jwt>
```

### Movement History

```
GET /api/inventory/items/{item_id}/history
Authorization: Bearer <jwt>
```

### Cycle Counts

```
POST /api/inventory/cycle-count-tasks                       — create cycle count task
POST /api/inventory/cycle-count-tasks/{task_id}/submit      — submit count results
POST /api/inventory/cycle-count-tasks/{task_id}/approve     — approve cycle count
Authorization: Bearer <jwt>
```

### Reorder Policies

```
POST /api/inventory/reorder-policies                    — create reorder policy
PUT  /api/inventory/reorder-policies/{id}               — update reorder policy
GET  /api/inventory/reorder-policies/{id}               — get reorder policy
GET  /api/inventory/items/{item_id}/reorder-policies    — list policies for item
Authorization: Bearer <jwt>
```

### Valuation Snapshots

```
POST /api/inventory/valuation-snapshots         — create valuation snapshot
GET  /api/inventory/valuation-snapshots         — list snapshots
GET  /api/inventory/valuation-snapshots/{id}    — get snapshot detail
Authorization: Bearer <jwt>
```

### Item Revisions

```
POST /api/inventory/items/{item_id}/revisions                                   — create revision
GET  /api/inventory/items/{item_id}/revisions                                   — list revisions
GET  /api/inventory/items/{item_id}/revisions/at?as_of=2026-03-12T00:00:00Z    — get revision at point in time
POST /api/inventory/items/{item_id}/revisions/{revision_id}/activate            — activate revision
PUT  /api/inventory/items/{item_id}/revisions/{revision_id}/policy-flags        — update revision policy flags
Authorization: Bearer <jwt>
```

### Labels

```
POST /api/inventory/items/{item_id}/labels     — generate label
GET  /api/inventory/items/{item_id}/labels     — list labels for item
GET  /api/inventory/labels/{label_id}          — get label by id
Authorization: Bearer <jwt>
```

### Expiry Management

```
PUT  /api/inventory/lots/{lot_id}/expiry          — set/update lot expiry date
POST /api/inventory/expiry-alerts/scan            — scan for expiring lots
Authorization: Bearer <jwt>
```

### Lot Genealogy

```
POST /api/inventory/lots/split                — split a lot into child lots
POST /api/inventory/lots/merge                — merge lots into a parent lot
GET  /api/inventory/lots/{lot_id}/children    — get child lots
GET  /api/inventory/lots/{lot_id}/parents     — get parent lots
Authorization: Bearer <jwt>
```

---

## Subscriptions Module

Source: `modules/subscriptions/src/http.rs`, `modules/subscriptions/src/main.rs`

**Base URL:** `http://7d-subscriptions:8087`

Subscription billing — manages recurring billing cycles. Finds active subscriptions due for billing, creates and finalizes invoices via the AR module, and advances the next bill date. Mutation routes require `subscriptions.mutate` permission.

### Execute Bill Run

```
POST /api/bill-runs/execute
Authorization: Bearer <jwt>
Content-Type: application/json
```

Request body:
```json
{
  "bill_run_id": "br-2026-03-12",
  "execution_date": "2026-03-12"
}
```

Both fields are optional. `bill_run_id` is auto-generated if omitted. `execution_date` defaults to today.

Response `200 OK`:
```json
{
  "bill_run_id": "br-2026-03-12",
  "subscriptions_processed": 15,
  "invoices_created": 14,
  "failures": 1,
  "execution_time": "2026-03-12T10:00:00Z"
}
```

Idempotent: if `bill_run_id` was already executed, returns the cached result.

The bill run:
1. Finds all active subscriptions for the tenant with `next_bill_date <= execution_date`
2. Creates an AR invoice for each subscription via `POST http://7d-ar:8086/api/ar/invoices`
3. Finalizes each invoice via `POST http://7d-ar:8086/api/ar/invoices/{id}/finalize`
4. Advances `next_bill_date` based on the subscription's schedule (`weekly`, `monthly`, or 4-week default)
5. Emits a `billrun.completed` event via outbox

---

## Payments Module

Source: `modules/payments/src/http/`, `modules/payments/src/main.rs`

**Base URL:** `http://7d-payments:8088`

Payment processing — checkout session management backed by Tilled.js. Platform owns the payment processor integration; vertical apps never call Tilled directly. All routes (except webhook) require `payments.mutate` permission.

### Create Checkout Session

```
POST /api/payments/checkout-sessions
Authorization: Bearer <jwt>
Content-Type: application/json
```

Request body:
```json
{
  "invoice_id": "inv-001",
  "tenant_id": "tenant-abc",
  "amount": 15000,
  "currency": "usd",
  "return_url": "https://app.example.com/payment/success",
  "cancel_url": "https://app.example.com/payment/cancel"
}
```

Required fields: **`invoice_id`**, **`amount`** (positive, in minor currency units), **`currency`**. `return_url` and `cancel_url` must be absolute HTTPS URLs.

Response `200 OK`:
```json
{
  "session_id": "uuid",
  "payment_intent_id": "pi_xxx",
  "client_secret": "pi_xxx_secret_yyy"
}
```

Pass `client_secret` to Tilled.js `confirmPayment()` on the frontend.

### Get Checkout Session

```
GET /api/payments/checkout-sessions/{id}
Authorization: Bearer <jwt>
```

Response `200 OK`:
```json
{
  "session_id": "uuid",
  "status": "presented",
  "payment_intent_id": "pi_xxx",
  "invoice_id": "inv-001",
  "tenant_id": "tenant-abc",
  "amount": 15000,
  "currency": "usd",
  "return_url": "https://...",
  "cancel_url": "https://..."
}
```

For non-terminal sessions (`created`, `presented`), this endpoint polls Tilled for live status.

### Present Checkout Session

```
POST /api/payments/checkout-sessions/{id}/present
Authorization: Bearer <jwt>
```

Idempotent transition: `created` → `presented` (called when the hosted pay page loads). Returns `200 OK`.

### Poll Checkout Session Status

```
GET /api/payments/checkout-sessions/{id}/status
Authorization: Bearer <jwt>
```

Lightweight status poll (no secrets returned). Used for client-side polling after payment.

Response `200 OK`:
```json
{
  "session_id": "uuid",
  "status": "completed"
}
```

Session status values: `created` → `presented` → `completed` | `failed` | `canceled` | `expired`

### Tilled Webhook

```
POST /api/payments/webhook/tilled
```

Receives Tilled payment processor callbacks. No JWT required — authenticated via webhook signature (`tilled-signature` header). Transitions checkout session status based on payment intent events:

- `payment_intent.succeeded` → `completed`
- `payment_intent.payment_failed` → `failed`
- `payment_intent.canceled` → `canceled`

Unknown event types are acknowledged with `200 OK`.

---

## AP Module — Accounts Payable

Source: `modules/ap/src/main.rs`, `modules/ap/src/http/`

**Base URL:** `http://7d-ap:8093`

Full accounts payable lifecycle: vendors, purchase orders, bills with 3-way matching, payment terms, payment runs, aging reports, and tax reports. Mutation routes require `ap.mutate`, read routes require `ap.read`.

### Vendors

| Method | Path | Purpose |
|--------|------|---------|
| `POST` | `/api/ap/vendors` | Create vendor |
| `GET` | `/api/ap/vendors` | List vendors |
| `GET` | `/api/ap/vendors/{vendor_id}` | Get vendor by ID |
| `PUT` | `/api/ap/vendors/{vendor_id}` | Update vendor |
| `POST` | `/api/ap/vendors/{vendor_id}/deactivate` | Deactivate vendor |

**Create vendor:**

```bash
curl -X POST http://7d-ap:8093/api/ap/vendors \
  -H "Authorization: Bearer $JWT" \
  -H "X-App-Id: my-vertical" \
  -H "Content-Type: application/json" \
  -d '{
    "name": "Acme Supplies",
    "currency": "USD",
    "payment_method": "ach",
    "default_gl_account_id": "550e8400-e29b-41d4-a716-446655440000"
  }'
```

```json
{
  "vendor_id": "a1b2c3d4-...",
  "tenant_id": "...",
  "name": "Acme Supplies",
  "currency": "USD",
  "payment_method": "ach",
  "default_gl_account_id": "550e8400-...",
  "status": "active",
  "created_at": "2026-03-11T10:00:00Z",
  "updated_at": "2026-03-11T10:00:00Z"
}
```

Errors: `404` vendor not found, `409` duplicate vendor name, `422` validation failure.

### Purchase Orders

| Method | Path | Purpose |
|--------|------|---------|
| `POST` | `/api/ap/pos` | Create purchase order (draft) |
| `GET` | `/api/ap/pos` | List POs (filter: `?vendor_id=...&status=...`) |
| `GET` | `/api/ap/pos/{po_id}` | Get PO with lines |
| `PUT` | `/api/ap/pos/{po_id}/lines` | Update PO lines |
| `POST` | `/api/ap/pos/{po_id}/approve` | Approve PO |

**Create purchase order:**

```bash
curl -X POST http://7d-ap:8093/api/ap/pos \
  -H "Authorization: Bearer $JWT" \
  -H "X-App-Id: my-vertical" \
  -H "Content-Type: application/json" \
  -d '{
    "vendor_id": "a1b2c3d4-...",
    "currency": "USD",
    "lines": [
      {
        "description": "Widget A",
        "quantity": 100,
        "unit_price_minor": 1500,
        "gl_account_id": "..."
      }
    ]
  }'
```

```json
{
  "po_id": "...",
  "tenant_id": "...",
  "vendor_id": "a1b2c3d4-...",
  "status": "draft",
  "currency": "USD",
  "lines": [
    {
      "line_id": "...",
      "description": "Widget A",
      "quantity": 100,
      "unit_price_minor": 1500,
      "gl_account_id": "..."
    }
  ],
  "created_at": "2026-03-11T10:00:00Z"
}
```

### Bills

| Method | Path | Purpose |
|--------|------|---------|
| `POST` | `/api/ap/bills` | Create vendor bill |
| `GET` | `/api/ap/bills` | List bills (filter: `?vendor_id=...&include_voided=true`) |
| `GET` | `/api/ap/bills/{bill_id}` | Get bill with lines |
| `POST` | `/api/ap/bills/{bill_id}/match` | Run 3-way match (PO + receipt + bill) |
| `POST` | `/api/ap/bills/{bill_id}/approve` | Approve bill (enforces match policy) |
| `POST` | `/api/ap/bills/{bill_id}/void` | Void bill (requires `reason`) |
| `POST` | `/api/ap/bills/{bill_id}/tax-quote` | Get tax quote for bill |

**Create bill:**

```bash
curl -X POST http://7d-ap:8093/api/ap/bills \
  -H "Authorization: Bearer $JWT" \
  -H "X-App-Id: my-vertical" \
  -H "Content-Type: application/json" \
  -d '{
    "vendor_id": "a1b2c3d4-...",
    "po_id": "...",
    "invoice_number": "INV-2026-001",
    "currency": "USD",
    "lines": [
      {
        "description": "Widget A",
        "quantity": 100,
        "unit_price_minor": 1500,
        "gl_account_id": "..."
      }
    ]
  }'
```

```json
{
  "bill_id": "...",
  "tenant_id": "...",
  "vendor_id": "a1b2c3d4-...",
  "po_id": "...",
  "invoice_number": "INV-2026-001",
  "status": "pending",
  "currency": "USD",
  "lines": [...],
  "match_status": null,
  "created_at": "2026-03-11T10:00:00Z"
}
```

**3-way match:** Compares bill lines against the purchase order and receiving records. Returns match result with discrepancies.

**Void bill:**

```bash
curl -X POST http://7d-ap:8093/api/ap/bills/{bill_id}/void \
  -H "Authorization: Bearer $JWT" \
  -H "X-App-Id: my-vertical" \
  -H "Content-Type: application/json" \
  -d '{ "reason": "Duplicate invoice" }'
```

### Bill Allocations

| Method | Path | Purpose |
|--------|------|---------|
| `POST` | `/api/ap/bills/{bill_id}/allocations` | Create payment allocation |
| `GET` | `/api/ap/bills/{bill_id}/allocations` | List allocations for bill |
| `GET` | `/api/ap/bills/{bill_id}/balance` | Get remaining bill balance |

**Create allocation** (idempotent via `idempotency_key`):

```bash
curl -X POST http://7d-ap:8093/api/ap/bills/{bill_id}/allocations \
  -H "Authorization: Bearer $JWT" \
  -H "X-App-Id: my-vertical" \
  -H "Content-Type: application/json" \
  -d '{
    "amount_minor": 150000,
    "currency": "USD",
    "allocation_type": "payment",
    "payment_run_id": "...",
    "idempotency_key": "alloc-001"
  }'
```

```json
{
  "allocation_id": "...",
  "bill_id": "...",
  "amount_minor": 150000,
  "currency": "USD",
  "allocation_type": "payment",
  "payment_run_id": "...",
  "created_at": "2026-03-11T10:00:00Z"
}
```

### Payment Terms

| Method | Path | Purpose |
|--------|------|---------|
| `POST` | `/api/ap/payment-terms` | Create payment terms (idempotent via `idempotency_key`) |
| `GET` | `/api/ap/payment-terms` | List payment terms |
| `GET` | `/api/ap/payment-terms/{term_id}` | Get payment terms by ID |
| `PUT` | `/api/ap/payment-terms/{term_id}` | Update payment terms |
| `POST` | `/api/ap/bills/{bill_id}/assign-terms` | Assign terms to bill |

**Create payment terms:**

```bash
curl -X POST http://7d-ap:8093/api/ap/payment-terms \
  -H "Authorization: Bearer $JWT" \
  -H "X-App-Id: my-vertical" \
  -H "Content-Type: application/json" \
  -d '{
    "name": "Net 30",
    "due_days": 30,
    "discount_days": 10,
    "discount_percent": "2.00",
    "idempotency_key": "terms-net30"
  }'
```

```json
{
  "term_id": "...",
  "tenant_id": "...",
  "name": "Net 30",
  "due_days": 30,
  "discount_days": 10,
  "discount_percent": "2.00",
  "created_at": "2026-03-11T10:00:00Z"
}
```

### Payment Runs

| Method | Path | Purpose |
|--------|------|---------|
| `POST` | `/api/ap/payment-runs` | Create payment run (idempotent via `run_id`) |
| `GET` | `/api/ap/payment-runs/{run_id}` | Get payment run status |
| `POST` | `/api/ap/payment-runs/{run_id}/execute` | Execute payment run |

**Create payment run:**

```bash
curl -X POST http://7d-ap:8093/api/ap/payment-runs \
  -H "Authorization: Bearer $JWT" \
  -H "X-App-Id: my-vertical" \
  -H "Content-Type: application/json" \
  -d '{
    "run_id": "run-2026-03-11",
    "currency": "USD",
    "scheduled_date": "2026-03-15",
    "payment_method": "ach",
    "created_by": "user-uuid",
    "due_on_or_before": "2026-03-15",
    "vendor_ids": ["a1b2c3d4-..."]
  }'
```

```json
{
  "run_id": "run-2026-03-11",
  "tenant_id": "...",
  "status": "pending",
  "currency": "USD",
  "scheduled_date": "2026-03-15",
  "payment_method": "ach",
  "created_at": "2026-03-11T10:00:00Z"
}
```

**Execute:** Submits all pending payments in the run and records allocations against bills.

### Aging Report

| Method | Path | Purpose |
|--------|------|---------|
| `GET` | `/api/ap/aging` | AP aging report |

**Query parameters:** `as_of` (date, `YYYY-MM-DD`), `by_vendor` (bool, default false).

```bash
curl "http://7d-ap:8093/api/ap/aging?as_of=2026-03-11&by_vendor=true" \
  -H "Authorization: Bearer $JWT" \
  -H "X-App-Id: my-vertical"
```

```json
{
  "as_of": "2026-03-11",
  "buckets_by_currency": {
    "USD": { "current": 50000, "days_30": 25000, "days_60": 10000, "days_90": 5000, "over_90": 0 }
  },
  "vendor_breakdown": [
    {
      "vendor_id": "...",
      "vendor_name": "Acme Supplies",
      "buckets": { "current": 50000, "days_30": 25000, "days_60": 0, "days_90": 0, "over_90": 0 }
    }
  ]
}
```

### Tax Reports

| Method | Path | Purpose |
|--------|------|---------|
| `GET` | `/api/ap/tax/reports/summary` | Tax summary for period |
| `GET` | `/api/ap/tax/reports/export` | Export tax report (CSV or JSON) |

**Query parameters:** `from`, `to` (dates, `YYYY-MM-DD`). Export also accepts `format=csv|json`.

```bash
curl "http://7d-ap:8093/api/ap/tax/reports/summary?from=2026-01-01&to=2026-03-31" \
  -H "Authorization: Bearer $JWT" \
  -H "X-App-Id: my-vertical"
```

---

## TTP Module — Third-Party Pricing

Source: `modules/ttp/src/http/mod.rs`, `modules/ttp/src/http/`

**Base URL:** `http://7d-ttp:8100`

Usage-based billing engine: ingest metering events, execute billing runs, manage service agreements. Mutation routes require `ttp.mutate`.

### Billing Runs

| Method | Path | Purpose |
|--------|------|---------|
| `POST` | `/api/ttp/billing-runs` | Execute billing run for a period |

**Execute billing run** (idempotent via `idempotency_key`):

```bash
curl -X POST http://7d-ttp:8100/api/ttp/billing-runs \
  -H "Authorization: Bearer $JWT" \
  -H "X-App-Id: my-vertical" \
  -H "Content-Type: application/json" \
  -d '{
    "billing_period": "2026-03",
    "idempotency_key": "bill-2026-03-run1"
  }'
```

```json
{
  "run_id": "...",
  "tenant_id": "...",
  "billing_period": "2026-03",
  "parties_billed": 12,
  "total_amount_minor": 4500000,
  "currency": "USD",
  "was_noop": false
}
```

`was_noop: true` when the same idempotency key was already processed — no new invoices created.

### Metering Events

| Method | Path | Purpose |
|--------|------|---------|
| `POST` | `/api/metering/events` | Ingest metering events (batch) |
| `GET` | `/api/metering/trace` | Price trace for a billing period |

**Ingest events:**

```bash
curl -X POST http://7d-ttp:8100/api/metering/events \
  -H "Authorization: Bearer $JWT" \
  -H "X-App-Id: my-vertical" \
  -H "Content-Type: application/json" \
  -d '{
    "events": [
      {
        "dimension": "api_calls",
        "quantity": 1500,
        "occurred_at": "2026-03-11T08:30:00Z",
        "idempotency_key": "evt-20260311-0830",
        "source_ref": "batch-42"
      }
    ]
  }'
```

```json
{
  "ingested": 1,
  "duplicates": 0,
  "results": [
    { "idempotency_key": "evt-20260311-0830", "status": "accepted" }
  ]
}
```

Each event has its own `idempotency_key` for deduplication. Duplicate events return `"status": "duplicate"`.

**Price trace** — shows how metered usage maps to pricing for a period:

```bash
curl "http://7d-ttp:8100/api/metering/trace?period=2026-03" \
  -H "Authorization: Bearer $JWT" \
  -H "X-App-Id: my-vertical"
```

### Service Agreements

| Method | Path | Purpose |
|--------|------|---------|
| `GET` | `/api/ttp/service-agreements` | List service agreements |

**Query parameters:** `status` — `active`, `suspended`, `cancelled`, or `all` (default: `active`).

```bash
curl "http://7d-ttp:8100/api/ttp/service-agreements?status=active" \
  -H "Authorization: Bearer $JWT" \
  -H "X-App-Id: my-vertical"
```

```json
{
  "tenant_id": "...",
  "items": [
    {
      "agreement_id": "...",
      "party_id": "...",
      "plan_code": "enterprise-monthly",
      "amount_minor": 500000,
      "currency": "USD",
      "billing_cycle": "monthly",
      "status": "active",
      "effective_from": "2026-01-01",
      "effective_to": null
    }
  ],
  "count": 1
}
```

---

## Treasury Module

Source: `modules/treasury/src/main.rs`, `modules/treasury/src/http/`

**Base URL:** `http://7d-treasury:8094`

Bank and credit-card account management, bank reconciliation with auto/manual matching, GL linkage, statement import (CSV), cash position and forecasting. Mutation routes require `treasury.mutate`.

### Accounts

| Method | Path | Purpose |
|--------|------|---------|
| `POST` | `/api/treasury/accounts/bank` | Create bank account |
| `POST` | `/api/treasury/accounts/credit-card` | Create credit card account |
| `GET` | `/api/treasury/accounts` | List all accounts |
| `GET` | `/api/treasury/accounts/{id}` | Get account by ID |
| `PUT` | `/api/treasury/accounts/{id}` | Update account |
| `POST` | `/api/treasury/accounts/{id}/deactivate` | Deactivate account |

**Create bank account** (idempotent via `X-Idempotency-Key` header):

```bash
curl -X POST http://7d-treasury:8094/api/treasury/accounts/bank \
  -H "Authorization: Bearer $JWT" \
  -H "X-App-Id: my-vertical" \
  -H "X-Idempotency-Key: bank-acct-001" \
  -H "Content-Type: application/json" \
  -d '{
    "name": "Operating Account",
    "bank_name": "First National",
    "account_number_last4": "1234",
    "currency": "USD",
    "gl_account_id": "..."
  }'
```

```json
{
  "id": "...",
  "tenant_id": "...",
  "account_type": "bank",
  "name": "Operating Account",
  "bank_name": "First National",
  "account_number_last4": "1234",
  "currency": "USD",
  "gl_account_id": "...",
  "status": "active",
  "created_at": "2026-03-11T10:00:00Z"
}
```

**Create credit card account** — same pattern, POST to `/api/treasury/accounts/credit-card`.

### Reconciliation

| Method | Path | Purpose |
|--------|------|---------|
| `POST` | `/api/treasury/recon/auto-match` | Auto-match statement lines to payments |
| `POST` | `/api/treasury/recon/manual-match` | Manually match a statement line to a payment |
| `GET` | `/api/treasury/recon/matches` | List matches (filter: `?account_id=...&include_superseded=true`) |
| `GET` | `/api/treasury/recon/unmatched` | List unmatched items |

**Auto-match:**

```bash
curl -X POST http://7d-treasury:8094/api/treasury/recon/auto-match \
  -H "Authorization: Bearer $JWT" \
  -H "X-App-Id: my-vertical" \
  -H "Content-Type: application/json" \
  -d '{ "account_id": "..." }'
```

**Unmatched items** response splits into `statement_lines` (imported but unmatched) and `payment_transactions` (recorded but not on statement).

### GL Linkage

| Method | Path | Purpose |
|--------|------|---------|
| `POST` | `/api/treasury/recon/gl-link` | Link bank transaction to GL journal entry |
| `GET` | `/api/treasury/recon/gl-unmatched-txns` | List bank transactions not linked to GL |
| `POST` | `/api/treasury/recon/gl-unmatched-entries` | Find unmatched GL entries from a list of IDs |

**Link to GL:**

```bash
curl -X POST http://7d-treasury:8094/api/treasury/recon/gl-link \
  -H "Authorization: Bearer $JWT" \
  -H "X-App-Id: my-vertical" \
  -H "Content-Type: application/json" \
  -d '{
    "bank_transaction_id": "...",
    "gl_entry_id": "..."
  }'
```

### Statement Import

| Method | Path | Purpose |
|--------|------|---------|
| `POST` | `/api/treasury/statements/import` | Import bank/credit-card statement (CSV, multipart) |

**Multipart form upload:**

```bash
curl -X POST http://7d-treasury:8094/api/treasury/statements/import \
  -H "Authorization: Bearer $JWT" \
  -H "X-App-Id: my-vertical" \
  -F "file=@statement.csv" \
  -F "account_id=..." \
  -F "period_start=2026-03-01" \
  -F "period_end=2026-03-31" \
  -F "opening_balance_minor=1000000" \
  -F "closing_balance_minor=1250000" \
  -F "format=generic"
```

Supported formats: `generic`, `chase_credit`, `amex_credit`.

```json
{
  "statement_id": "...",
  "account_id": "...",
  "lines_imported": 47,
  "period_start": "2026-03-01",
  "period_end": "2026-03-31",
  "opening_balance_minor": 1000000,
  "closing_balance_minor": 1250000
}
```

### Cash Position & Forecast

| Method | Path | Purpose |
|--------|------|---------|
| `GET` | `/api/treasury/cash-position` | Current cash position by account and currency |
| `GET` | `/api/treasury/forecast` | Cash flow forecast (reads AR/AP aging) |

**Cash position:**

```bash
curl http://7d-treasury:8094/api/treasury/cash-position \
  -H "Authorization: Bearer $JWT" \
  -H "X-App-Id: my-vertical"
```

```json
{
  "positions": [
    {
      "account_id": "...",
      "account_name": "Operating Account",
      "currency": "USD",
      "balance_minor": 1250000
    }
  ],
  "totals_by_currency": {
    "USD": 1250000
  }
}
```

**Forecast** aggregates AR aging (expected inflows) and AP aging (expected outflows) from their respective databases to project future cash balances.

---

## Fixed Assets Module

Source: `modules/fixed-assets/src/main.rs`, `modules/fixed-assets/src/http/`

**Base URL:** `http://7d-fixed-assets:8104`

Asset lifecycle management: categories, asset register, straight-line depreciation with schedule generation and run execution, and disposals/impairments. Mutation routes require `fixed_assets.mutate`.

### Categories

| Method | Path | Purpose |
|--------|------|---------|
| `POST` | `/api/fixed-assets/categories` | Create category |
| `GET` | `/api/fixed-assets/categories` | List categories |
| `GET` | `/api/fixed-assets/categories/{id}` | Get category by ID |
| `PUT` | `/api/fixed-assets/categories/{id}` | Update category |
| `DELETE` | `/api/fixed-assets/categories/{id}` | Deactivate category |

**Create category:**

```bash
curl -X POST http://7d-fixed-assets:8104/api/fixed-assets/categories \
  -H "Authorization: Bearer $JWT" \
  -H "X-App-Id: my-vertical" \
  -H "Content-Type: application/json" \
  -d '{
    "code": "MACH",
    "name": "Machinery & Equipment",
    "useful_life_months": 120,
    "depreciation_method": "straight_line",
    "asset_gl_account_id": "...",
    "depreciation_gl_account_id": "...",
    "expense_gl_account_id": "..."
  }'
```

```json
{
  "id": "...",
  "tenant_id": "...",
  "code": "MACH",
  "name": "Machinery & Equipment",
  "useful_life_months": 120,
  "depreciation_method": "straight_line",
  "asset_gl_account_id": "...",
  "depreciation_gl_account_id": "...",
  "expense_gl_account_id": "...",
  "status": "active",
  "created_at": "2026-03-11T10:00:00Z"
}
```

Errors: `404` not found, `409` duplicate category code, `422` validation failure.

### Assets

| Method | Path | Purpose |
|--------|------|---------|
| `POST` | `/api/fixed-assets/assets` | Register asset |
| `GET` | `/api/fixed-assets/assets` | List assets (filter: `?status=active`) |
| `GET` | `/api/fixed-assets/assets/{id}` | Get asset by ID |
| `PUT` | `/api/fixed-assets/assets/{id}` | Update asset |
| `DELETE` | `/api/fixed-assets/assets/{id}` | Deactivate asset |

**Register asset:**

```bash
curl -X POST http://7d-fixed-assets:8104/api/fixed-assets/assets \
  -H "Authorization: Bearer $JWT" \
  -H "X-App-Id: my-vertical" \
  -H "Content-Type: application/json" \
  -d '{
    "tag": "MACH-001",
    "name": "CNC Milling Machine",
    "category_id": "...",
    "acquisition_date": "2026-01-15",
    "acquisition_cost_minor": 5000000,
    "residual_value_minor": 500000,
    "currency": "USD",
    "location": "Building A, Floor 2"
  }'
```

```json
{
  "id": "...",
  "tenant_id": "...",
  "tag": "MACH-001",
  "name": "CNC Milling Machine",
  "category_id": "...",
  "acquisition_date": "2026-01-15",
  "acquisition_cost_minor": 5000000,
  "residual_value_minor": 500000,
  "currency": "USD",
  "location": "Building A, Floor 2",
  "status": "active",
  "created_at": "2026-03-11T10:00:00Z"
}
```

Errors: `404` not found, `409` duplicate asset tag, `422` validation failure, `400` invalid status transition.

### Depreciation

| Method | Path | Purpose |
|--------|------|---------|
| `POST` | `/api/fixed-assets/depreciation/schedule` | Generate depreciation schedule for an asset |
| `POST` | `/api/fixed-assets/depreciation/runs` | Execute depreciation run (posts journal entries) |
| `GET` | `/api/fixed-assets/depreciation/runs` | List depreciation runs |
| `GET` | `/api/fixed-assets/depreciation/runs/{id}` | Get depreciation run by ID |

**Generate schedule** (idempotent — regenerates if called again):

```bash
curl -X POST http://7d-fixed-assets:8104/api/fixed-assets/depreciation/schedule \
  -H "Authorization: Bearer $JWT" \
  -H "X-App-Id: my-vertical" \
  -H "Content-Type: application/json" \
  -d '{ "asset_id": "..." }'
```

Uses straight-line method: `(acquisition_cost - residual_value) / useful_life_months`.

**Execute depreciation run** (idempotent — posts unposted periods up to `as_of_date`):

```bash
curl -X POST http://7d-fixed-assets:8104/api/fixed-assets/depreciation/runs \
  -H "Authorization: Bearer $JWT" \
  -H "X-App-Id: my-vertical" \
  -H "Content-Type: application/json" \
  -d '{ "as_of_date": "2026-03-31" }'
```

```json
{
  "run_id": "...",
  "tenant_id": "...",
  "as_of_date": "2026-03-31",
  "assets_processed": 15,
  "periods_posted": 45,
  "total_depreciation_minor": 375000,
  "created_at": "2026-03-11T10:00:00Z"
}
```

### Disposals

| Method | Path | Purpose |
|--------|------|---------|
| `POST` | `/api/fixed-assets/disposals` | Dispose or impair an asset |
| `GET` | `/api/fixed-assets/disposals` | List disposals |
| `GET` | `/api/fixed-assets/disposals/{id}` | Get disposal by ID |

**Dispose asset** (idempotent):

```bash
curl -X POST http://7d-fixed-assets:8104/api/fixed-assets/disposals \
  -H "Authorization: Bearer $JWT" \
  -H "X-App-Id: my-vertical" \
  -H "Content-Type: application/json" \
  -d '{
    "asset_id": "...",
    "disposal_date": "2026-03-11",
    "disposal_type": "sale",
    "proceeds_minor": 2000000,
    "currency": "USD",
    "reason": "Equipment upgrade"
  }'
```

```json
{
  "id": "...",
  "asset_id": "...",
  "disposal_date": "2026-03-11",
  "disposal_type": "sale",
  "proceeds_minor": 2000000,
  "net_book_value_minor": 2500000,
  "gain_loss_minor": -500000,
  "currency": "USD",
  "reason": "Equipment upgrade",
  "created_at": "2026-03-11T10:00:00Z"
}
```

---

## Consolidation Module

Source: `modules/consolidation/src/http/mod.rs`, `modules/consolidation/src/http/`

**Base URL:** `http://7d-consolidation:8105`

Multi-entity financial consolidation: group management, entity mapping, chart-of-accounts mapping, elimination rules, FX policies, intercompany matching and elimination, and consolidated financial statements. Mutation routes require `consolidation.mutate`.

### Groups

| Method | Path | Purpose |
|--------|------|---------|
| `POST` | `/api/consolidation/groups` | Create consolidation group |
| `GET` | `/api/consolidation/groups` | List groups |
| `GET` | `/api/consolidation/groups/{id}` | Get group by ID |
| `PUT` | `/api/consolidation/groups/{id}` | Update group |
| `DELETE` | `/api/consolidation/groups/{id}` | Delete group |
| `GET` | `/api/consolidation/groups/{id}/validate` | Validate group configuration |

**Create group:**

```bash
curl -X POST http://7d-consolidation:8105/api/consolidation/groups \
  -H "Authorization: Bearer $JWT" \
  -H "X-App-Id: my-vertical" \
  -H "Content-Type: application/json" \
  -d '{
    "name": "ACME Holdings",
    "reporting_currency": "USD",
    "fiscal_year_end_month": 12
  }'
```

```json
{
  "id": "...",
  "tenant_id": "...",
  "name": "ACME Holdings",
  "reporting_currency": "USD",
  "fiscal_year_end_month": 12,
  "created_at": "2026-03-11T10:00:00Z"
}
```

**Validate** checks that entities, COA mappings, elimination rules, and FX policies are consistent.

### Entities

| Method | Path | Purpose |
|--------|------|---------|
| `POST` | `/api/consolidation/groups/{group_id}/entities` | Add entity to group |
| `GET` | `/api/consolidation/groups/{group_id}/entities` | List entities in group |
| `GET` | `/api/consolidation/entities/{id}` | Get entity by ID |
| `PUT` | `/api/consolidation/entities/{id}` | Update entity |
| `DELETE` | `/api/consolidation/entities/{id}` | Remove entity from group |

**Add entity:**

```bash
curl -X POST http://7d-consolidation:8105/api/consolidation/groups/{group_id}/entities \
  -H "Authorization: Bearer $JWT" \
  -H "X-App-Id: my-vertical" \
  -H "Content-Type: application/json" \
  -d '{
    "name": "ACME Manufacturing",
    "entity_code": "ACME-MFG",
    "functional_currency": "EUR",
    "ownership_percent": "100.00",
    "gl_database_url": "postgres://..."
  }'
```

### COA Mappings

| Method | Path | Purpose |
|--------|------|---------|
| `POST` | `/api/consolidation/groups/{group_id}/coa-mappings` | Create COA mapping |
| `GET` | `/api/consolidation/groups/{group_id}/coa-mappings` | List COA mappings |
| `DELETE` | `/api/consolidation/coa-mappings/{id}` | Delete COA mapping |

Maps entity-level chart of accounts codes to consolidated group-level accounts.

### Elimination Rules

| Method | Path | Purpose |
|--------|------|---------|
| `POST` | `/api/consolidation/groups/{group_id}/elimination-rules` | Create elimination rule |
| `GET` | `/api/consolidation/groups/{group_id}/elimination-rules` | List elimination rules |
| `GET` | `/api/consolidation/elimination-rules/{id}` | Get elimination rule |
| `PUT` | `/api/consolidation/elimination-rules/{id}` | Update elimination rule |
| `DELETE` | `/api/consolidation/elimination-rules/{id}` | Delete elimination rule |

Rules define how intercompany balances are eliminated during consolidation (e.g., intercompany receivables vs payables).

### FX Policies

| Method | Path | Purpose |
|--------|------|---------|
| `PUT` | `/api/consolidation/groups/{group_id}/fx-policies` | Set FX translation policy |
| `GET` | `/api/consolidation/groups/{group_id}/fx-policies` | Get FX policies |
| `DELETE` | `/api/consolidation/fx-policies/{id}` | Delete FX policy |

Controls how foreign-currency entity balances are translated to the group reporting currency.

### Consolidation Engine

| Method | Path | Purpose |
|--------|------|---------|
| `POST` | `/api/consolidation/groups/{group_id}/consolidate` | Run consolidation |
| `GET` | `/api/consolidation/groups/{group_id}/trial-balance` | Get consolidated trial balance (cached) |

**Run consolidation:**

```bash
curl -X POST http://7d-consolidation:8105/api/consolidation/groups/{group_id}/consolidate \
  -H "Authorization: Bearer $JWT" \
  -H "X-App-Id: my-vertical" \
  -H "Content-Type: application/json" \
  -d '{
    "period_id": "2026-Q1",
    "as_of": "2026-03-31"
  }'
```

```json
{
  "rows": [
    {
      "account_code": "1000",
      "account_name": "Cash",
      "debit_minor": 5000000,
      "credit_minor": 0,
      "currency": "USD"
    }
  ],
  "input_hash": "sha256:...",
  "entity_hashes": {
    "ACME-MFG": "sha256:...",
    "ACME-DIST": "sha256:..."
  }
}
```

**Trial balance** (GET) returns cached result with `?as_of=YYYY-MM-DD` query parameter.

### Intercompany

| Method | Path | Purpose |
|--------|------|---------|
| `POST` | `/api/consolidation/groups/{group_id}/intercompany-match` | Match intercompany balances |
| `POST` | `/api/consolidation/groups/{group_id}/eliminations` | Post elimination journal entries to GL |

**Intercompany match:**

```bash
curl -X POST http://7d-consolidation:8105/api/consolidation/groups/{group_id}/intercompany-match \
  -H "Authorization: Bearer $JWT" \
  -H "X-App-Id: my-vertical" \
  -H "Content-Type: application/json" \
  -d '{
    "period_id": "2026-Q1",
    "as_of": "2026-03-31"
  }'
```

Returns matched intercompany pairs and suggested elimination entries.

**Post eliminations** (idempotent per group + period):

```bash
curl -X POST http://7d-consolidation:8105/api/consolidation/groups/{group_id}/eliminations \
  -H "Authorization: Bearer $JWT" \
  -H "X-App-Id: my-vertical" \
  -H "Content-Type: application/json" \
  -d '{
    "period_id": "2026-Q1",
    "as_of": "2026-03-31",
    "reporting_currency": "USD"
  }'
```

Posts elimination journal entries to the GL module.

### Consolidated Statements

| Method | Path | Purpose |
|--------|------|---------|
| `GET` | `/api/consolidation/groups/{group_id}/pl` | Consolidated P&L |
| `GET` | `/api/consolidation/groups/{group_id}/balance-sheet` | Consolidated balance sheet |

**Query parameter:** `as_of` (date, `YYYY-MM-DD`).

```bash
curl "http://7d-consolidation:8105/api/consolidation/groups/{group_id}/pl?as_of=2026-03-31" \
  -H "Authorization: Bearer $JWT" \
  -H "X-App-Id: my-vertical"
```

---

## BOM Module

Source: `modules/bom/src/http/bom_routes.rs`, `modules/bom/src/http/eco_routes.rs`, `modules/bom/src/main.rs`

**Base URL:** `http://7d-bom:8107`

Bill of Materials management with multi-level revision control, effectivity dating, multi-level explosion, where-used analysis, and Engineering Change Orders (ECOs). Read routes require `bom.read`, mutation routes require `bom.mutate`.

### BOM Headers

| Method | Path | Purpose |
|--------|------|---------|
| `POST` | `/api/bom` | Create BOM header for a part |
| `GET` | `/api/bom/{bom_id}` | Get BOM header by ID |

**Create BOM:**

```bash
curl -X POST http://7d-bom:8107/api/bom \
  -H "Authorization: Bearer $JWT" \
  -H "X-App-Id: my-vertical" \
  -H "Content-Type: application/json" \
  -d '{
    "part_id": "a1b2c3d4-e5f6-7890-abcd-ef1234567890",
    "description": "Main assembly BOM"
  }'
```

Required: **`part_id`**. `description` is optional.

Response `201 Created`:
```json
{
  "id": "...",
  "tenant_id": "...",
  "part_id": "a1b2c3d4-e5f6-7890-abcd-ef1234567890",
  "description": "Main assembly BOM",
  "created_at": "2026-03-11T10:00:00Z",
  "updated_at": "2026-03-11T10:00:00Z"
}
```

### Revisions

| Method | Path | Purpose |
|--------|------|---------|
| `POST` | `/api/bom/{bom_id}/revisions` | Create new revision |
| `GET` | `/api/bom/{bom_id}/revisions` | List all revisions |
| `POST` | `/api/bom/revisions/{revision_id}/effectivity` | Set effectivity date range |

**Create revision:**

```bash
curl -X POST http://7d-bom:8107/api/bom/{bom_id}/revisions \
  -H "Authorization: Bearer $JWT" \
  -H "X-App-Id: my-vertical" \
  -H "Content-Type: application/json" \
  -d '{ "revision_label": "A" }'
```

Required: **`revision_label`**.

Response `201 Created`:
```json
{
  "id": "...",
  "bom_id": "...",
  "tenant_id": "...",
  "revision_label": "A",
  "status": "draft",
  "effective_from": null,
  "effective_to": null,
  "created_at": "2026-03-11T10:00:00Z",
  "updated_at": "2026-03-11T10:00:00Z"
}
```

**Set effectivity:**

```bash
curl -X POST http://7d-bom:8107/api/bom/revisions/{revision_id}/effectivity \
  -H "Authorization: Bearer $JWT" \
  -H "X-App-Id: my-vertical" \
  -H "Content-Type: application/json" \
  -d '{
    "effective_from": "2026-04-01T00:00:00Z",
    "effective_to": "2027-03-31T23:59:59Z"
  }'
```

Required: **`effective_from`**. `effective_to` is optional (open-ended if null). Returns `409` if date range overlaps an existing revision.

### Lines (Components)

| Method | Path | Purpose |
|--------|------|---------|
| `POST` | `/api/bom/revisions/{revision_id}/lines` | Add component line |
| `GET` | `/api/bom/revisions/{revision_id}/lines` | List lines for revision |
| `PUT` | `/api/bom/lines/{line_id}` | Update component line |
| `DELETE` | `/api/bom/lines/{line_id}` | Remove component line |

**Add component line:**

```bash
curl -X POST http://7d-bom:8107/api/bom/revisions/{revision_id}/lines \
  -H "Authorization: Bearer $JWT" \
  -H "X-App-Id: my-vertical" \
  -H "Content-Type: application/json" \
  -d '{
    "component_item_id": "...",
    "quantity": 2.0,
    "uom": "EA",
    "scrap_factor": 0.05,
    "find_number": 10
  }'
```

Required: **`component_item_id`**, **`quantity`**. Others optional.

Response `201 Created`:
```json
{
  "id": "...",
  "revision_id": "...",
  "tenant_id": "...",
  "component_item_id": "...",
  "quantity": 2.0,
  "uom": "EA",
  "scrap_factor": 0.05,
  "find_number": 10,
  "created_at": "2026-03-11T10:00:00Z",
  "updated_at": "2026-03-11T10:00:00Z"
}
```

**Update line** (PUT) accepts partial updates: `quantity`, `uom`, `scrap_factor`, `find_number` — all optional.

**Delete line** returns `204 No Content`.

### Explosion & Where-Used

| Method | Path | Purpose |
|--------|------|---------|
| `GET` | `/api/bom/{bom_id}/explosion` | Multi-level BOM explosion |
| `GET` | `/api/bom/where-used/{item_id}` | Where-used analysis for a component |

**Explosion** recursively expands all sub-assemblies into a flat list of components with rolled-up quantities:

```bash
curl "http://7d-bom:8107/api/bom/{bom_id}/explosion?date=2026-04-01T00:00:00Z&max_depth=5" \
  -H "Authorization: Bearer $JWT" \
  -H "X-App-Id: my-vertical"
```

Query parameters: `date` (ISO 8601, filters by effectivity), `max_depth` (limits recursion depth).

```json
[
  {
    "level": 1,
    "parent_part_id": "...",
    "component_item_id": "...",
    "quantity": 2.0,
    "uom": "EA",
    "scrap_factor": 0.05,
    "revision_id": "...",
    "revision_label": "A"
  }
]
```

Returns `422` if a cycle is detected in the BOM structure.

**Where-used** finds all BOMs that reference a given item as a component:

```bash
curl "http://7d-bom:8107/api/bom/where-used/{item_id}?date=2026-04-01T00:00:00Z" \
  -H "Authorization: Bearer $JWT" \
  -H "X-App-Id: my-vertical"
```

```json
[
  {
    "bom_id": "...",
    "part_id": "...",
    "revision_id": "...",
    "revision_label": "A",
    "quantity": 2.0,
    "uom": "EA"
  }
]
```

### Engineering Change Orders (ECO)

| Method | Path | Purpose |
|--------|------|---------|
| `POST` | `/api/eco` | Create ECO (auto-numbers via Numbering service) |
| `GET` | `/api/eco/{eco_id}` | Get ECO |
| `POST` | `/api/eco/{eco_id}/submit` | Submit ECO for review |
| `POST` | `/api/eco/{eco_id}/approve` | Approve ECO |
| `POST` | `/api/eco/{eco_id}/reject` | Reject ECO |
| `POST` | `/api/eco/{eco_id}/apply` | Apply ECO (sets effectivity on linked revisions) |
| `POST` | `/api/eco/{eco_id}/bom-revisions` | Link BOM revision to ECO |
| `GET` | `/api/eco/{eco_id}/bom-revisions` | List BOM revision links |
| `POST` | `/api/eco/{eco_id}/doc-revisions` | Link document revision to ECO |
| `GET` | `/api/eco/{eco_id}/doc-revisions` | List document revision links |
| `GET` | `/api/eco/{eco_id}/audit` | Get ECO audit trail |
| `GET` | `/api/eco/history/{part_id}` | ECO history for a specific part |

**Create ECO:**

```bash
curl -X POST http://7d-bom:8107/api/eco \
  -H "Authorization: Bearer $JWT" \
  -H "X-App-Id: my-vertical" \
  -H "Content-Type: application/json" \
  -d '{
    "title": "Update gear ratio for Model X",
    "description": "Change gear component from 3:1 to 4:1 ratio",
    "created_by": "engineer-001"
  }'
```

Required: **`title`**, **`created_by`**. `eco_number` is optional — if omitted, auto-allocated from the Numbering service.

Response `201 Created`:
```json
{
  "id": "...",
  "tenant_id": "...",
  "eco_number": "ECO-00001",
  "title": "Update gear ratio for Model X",
  "description": "Change gear component from 3:1 to 4:1 ratio",
  "status": "draft",
  "created_by": "engineer-001",
  "approved_by": null,
  "approved_at": null,
  "applied_at": null,
  "created_at": "2026-03-11T10:00:00Z",
  "updated_at": "2026-03-11T10:00:00Z"
}
```

**ECO lifecycle:** `draft` → `submitted` → `approved`/`rejected` → `applied`. Each transition requires `actor` and optional `comment`. Apply also requires `effective_from` and optional `effective_to`.

**Link BOM revision to ECO:**

```bash
curl -X POST http://7d-bom:8107/api/eco/{eco_id}/bom-revisions \
  -H "Authorization: Bearer $JWT" \
  -H "X-App-Id: my-vertical" \
  -H "Content-Type: application/json" \
  -d '{
    "bom_id": "...",
    "before_revision_id": "...",
    "after_revision_id": "..."
  }'
```

---

## Production Module

Source: `modules/production/src/http/`, `modules/production/src/main.rs`

**Base URL:** `http://7d-production:8108`

Shop floor execution: workcenters, work orders (draft→released→closed), routing templates with steps, operation tracking, component issues, finished goods receipts, labor time entries, and workcenter downtime.

### Workcenters

| Method | Path | Purpose |
|--------|------|---------|
| `POST` | `/api/production/workcenters` | Create workcenter |
| `GET` | `/api/production/workcenters` | List workcenters |
| `GET` | `/api/production/workcenters/{id}` | Get workcenter |
| `PUT` | `/api/production/workcenters/{id}` | Update workcenter |
| `POST` | `/api/production/workcenters/{id}/deactivate` | Deactivate workcenter |

**Create workcenter:**

```bash
curl -X POST http://7d-production:8108/api/production/workcenters \
  -H "Authorization: Bearer $JWT" \
  -H "X-App-Id: my-vertical" \
  -H "Content-Type: application/json" \
  -d '{
    "code": "CNC-01",
    "name": "CNC Milling Center 1",
    "description": "5-axis CNC mill",
    "capacity": 8,
    "cost_rate_minor": 15000
  }'
```

Required: **`code`**, **`name`**. Others optional. `cost_rate_minor` is in minor currency units (cents).

Response `201 Created`:
```json
{
  "workcenter_id": "...",
  "tenant_id": "...",
  "code": "CNC-01",
  "name": "CNC Milling Center 1",
  "description": "5-axis CNC mill",
  "capacity": 8,
  "cost_rate_minor": 15000,
  "is_active": true,
  "created_at": "2026-03-11T10:00:00Z",
  "updated_at": "2026-03-11T10:00:00Z"
}
```

Errors: `409` duplicate workcenter code, `422` validation failure.

### Work Orders

| Method | Path | Purpose |
|--------|------|---------|
| `POST` | `/api/production/work-orders` | Create work order (starts in `draft`) |
| `GET` | `/api/production/work-orders/{id}` | Get work order |
| `POST` | `/api/production/work-orders/{id}/release` | Release: draft → released |
| `POST` | `/api/production/work-orders/{id}/close` | Close: released → closed |

**Create work order:**

```bash
curl -X POST http://7d-production:8108/api/production/work-orders \
  -H "Authorization: Bearer $JWT" \
  -H "X-App-Id: my-vertical" \
  -H "Content-Type: application/json" \
  -d '{
    "order_number": "WO-2026-001",
    "item_id": "...",
    "bom_revision_id": "...",
    "routing_template_id": "...",
    "planned_quantity": 100,
    "planned_start": "2026-04-01T08:00:00Z",
    "planned_end": "2026-04-05T17:00:00Z",
    "correlation_id": "unique-idempotency-key"
  }'
```

Required: **`order_number`**, **`item_id`**, **`bom_revision_id`**, **`planned_quantity`** (> 0). `correlation_id` enables idempotent creation — duplicate requests return the existing work order.

Response `201 Created`:
```json
{
  "work_order_id": "...",
  "tenant_id": "...",
  "order_number": "WO-2026-001",
  "status": "draft",
  "item_id": "...",
  "bom_revision_id": "...",
  "routing_template_id": "...",
  "planned_quantity": 100,
  "completed_quantity": 0,
  "planned_start": "2026-04-01T08:00:00Z",
  "planned_end": "2026-04-05T17:00:00Z",
  "actual_start": null,
  "actual_end": null,
  "material_cost_minor": 0,
  "labor_cost_minor": 0,
  "overhead_cost_minor": 0,
  "created_at": "2026-03-11T10:00:00Z",
  "updated_at": "2026-03-11T10:00:00Z"
}
```

**Status lifecycle:** `draft` → `released` (sets `actual_start`) → `closed` (sets `actual_end`). Invalid transitions return `422`.

### Routings

| Method | Path | Purpose |
|--------|------|---------|
| `POST` | `/api/production/routings` | Create routing template |
| `GET` | `/api/production/routings` | List routing templates |
| `GET` | `/api/production/routings/{id}` | Get routing template |
| `PUT` | `/api/production/routings/{id}` | Update routing (draft only) |
| `POST` | `/api/production/routings/{id}/release` | Release: draft → released (immutable) |
| `GET` | `/api/production/routings/{id}/steps` | List routing steps |
| `POST` | `/api/production/routings/{id}/steps` | Add routing step (draft only) |
| `GET` | `/api/production/routings/by-item?item_id=&effective_date=` | Find routings by item and date |

**Create routing:**

```bash
curl -X POST http://7d-production:8108/api/production/routings \
  -H "Authorization: Bearer $JWT" \
  -H "X-App-Id: my-vertical" \
  -H "Content-Type: application/json" \
  -d '{
    "name": "Standard Assembly Routing",
    "description": "3-step assembly process",
    "item_id": "...",
    "bom_revision_id": "...",
    "revision": "1",
    "effective_from_date": "2026-04-01"
  }'
```

Required: **`name`**. Others optional. `revision` defaults to `"1"`.

**Add routing step:**

```bash
curl -X POST http://7d-production:8108/api/production/routings/{id}/steps \
  -H "Authorization: Bearer $JWT" \
  -H "X-App-Id: my-vertical" \
  -H "Content-Type: application/json" \
  -d '{
    "sequence_number": 10,
    "workcenter_id": "...",
    "operation_name": "CNC Milling",
    "description": "Mill housing to spec",
    "setup_time_minutes": 30,
    "run_time_minutes": 45,
    "is_required": true
  }'
```

Required: **`sequence_number`** (> 0), **`workcenter_id`**, **`operation_name`**. Workcenter must be active. Cannot add steps to released routings.

### Operations (Work Order Execution)

| Method | Path | Purpose |
|--------|------|---------|
| `GET` | `/api/production/work-orders/{id}/operations` | List operations for a work order |
| `POST` | `/api/production/work-orders/{id}/operations/initialize` | Initialize operations from routing template |
| `POST` | `/api/production/work-orders/{wo_id}/operations/{op_id}/start` | Start operation |
| `POST` | `/api/production/work-orders/{wo_id}/operations/{op_id}/complete` | Complete operation |

Initialize copies routing steps into the work order as executable operation instances.

### Component Issues & FG Receipts

| Method | Path | Purpose |
|--------|------|---------|
| `POST` | `/api/production/work-orders/{id}/component-issues` | Issue components to work order |
| `POST` | `/api/production/work-orders/{id}/fg-receipt` | Receive finished goods from work order |

These transactions record material consumption and production output, updating material cost accumulators.

### Time Entries

| Method | Path | Purpose |
|--------|------|---------|
| `POST` | `/api/production/time-entries/start` | Start labor timer |
| `POST` | `/api/production/time-entries/manual` | Record manual time entry |
| `POST` | `/api/production/time-entries/{id}/stop` | Stop running timer |
| `GET` | `/api/production/work-orders/{id}/time-entries` | List time entries for work order |

### Workcenter Downtime

| Method | Path | Purpose |
|--------|------|---------|
| `POST` | `/api/production/workcenters/{id}/downtime/start` | Start downtime period |
| `GET` | `/api/production/workcenters/{id}/downtime` | List downtime for workcenter |
| `POST` | `/api/production/downtime/{id}/end` | End downtime period |
| `GET` | `/api/production/downtime/active` | List all active downtime |

---

## Quality Inspection Module

Source: `modules/quality-inspection/src/http/inspection_routes.rs`, `modules/quality-inspection/src/main.rs`

**Base URL:** `http://7d-quality-inspection:8106`

Quality inspection management: inspection plans with characteristics, receiving inspections (triggered by inventory receipts), in-process inspections (during production operations), final inspections (work order completion), and disposition state machine. Mutation routes require `quality_inspection.mutate`, read routes require `quality_inspection.read`. Inspector authorization is checked against the Workforce-Competence database.

### Inspection Plans

| Method | Path | Purpose |
|--------|------|---------|
| `POST` | `/api/quality-inspection/plans` | Create inspection plan |
| `GET` | `/api/quality-inspection/plans/{plan_id}` | Get inspection plan |
| `POST` | `/api/quality-inspection/plans/{plan_id}/activate` | Activate plan |

**Create inspection plan:**

```bash
curl -X POST http://7d-quality-inspection:8106/api/quality-inspection/plans \
  -H "Authorization: Bearer $JWT" \
  -H "X-App-Id: my-vertical" \
  -H "Content-Type: application/json" \
  -d '{
    "part_id": "...",
    "plan_name": "Gear Housing Inspection",
    "revision": "A",
    "characteristics": [
      {
        "name": "Outer Diameter",
        "characteristic_type": "dimensional",
        "nominal": 25.4,
        "tolerance_low": 25.35,
        "tolerance_high": 25.45,
        "uom": "mm"
      }
    ],
    "sampling_method": "100_percent",
    "sample_size": null
  }'
```

Required: **`part_id`**, **`plan_name`**, **`characteristics`** (array). Others optional.

Response `201 Created`:
```json
{
  "id": "...",
  "tenant_id": "...",
  "part_id": "...",
  "plan_name": "Gear Housing Inspection",
  "revision": "A",
  "status": "draft",
  "characteristics": [...],
  "sampling_method": "100_percent",
  "sample_size": null,
  "created_at": "2026-03-11T10:00:00Z",
  "updated_at": "2026-03-11T10:00:00Z"
}
```

### Inspections

| Method | Path | Purpose |
|--------|------|---------|
| `POST` | `/api/quality-inspection/inspections` | Create receiving inspection |
| `POST` | `/api/quality-inspection/inspections/in-process` | Create in-process inspection |
| `POST` | `/api/quality-inspection/inspections/final` | Create final inspection |
| `GET` | `/api/quality-inspection/inspections/{inspection_id}` | Get inspection |

**Create receiving inspection** (triggered when goods are received):

```bash
curl -X POST http://7d-quality-inspection:8106/api/quality-inspection/inspections \
  -H "Authorization: Bearer $JWT" \
  -H "X-App-Id: my-vertical" \
  -H "Content-Type: application/json" \
  -d '{
    "plan_id": "...",
    "receipt_id": "...",
    "lot_id": "...",
    "part_id": "...",
    "part_revision": "A",
    "inspector_id": "...",
    "result": "pass",
    "notes": "All dimensions within spec"
  }'
```

All fields optional. The receiving inspection is also auto-created by the receipt event bridge when inventory receipts arrive via NATS.

**Create in-process inspection** (during production):

```bash
curl -X POST http://7d-quality-inspection:8106/api/quality-inspection/inspections/in-process \
  -H "Authorization: Bearer $JWT" \
  -H "X-App-Id: my-vertical" \
  -H "Content-Type: application/json" \
  -d '{
    "wo_id": "...",
    "op_instance_id": "...",
    "plan_id": "...",
    "inspector_id": "...",
    "result": "pass"
  }'
```

Required: **`wo_id`**, **`op_instance_id`**.

**Create final inspection** (at work order completion):

```bash
curl -X POST http://7d-quality-inspection:8106/api/quality-inspection/inspections/final \
  -H "Authorization: Bearer $JWT" \
  -H "X-App-Id: my-vertical" \
  -H "Content-Type: application/json" \
  -d '{
    "wo_id": "...",
    "lot_id": "...",
    "plan_id": "...",
    "inspector_id": "...",
    "result": "pass"
  }'
```

Required: **`wo_id`**.

### Disposition Transitions

| Method | Path | Purpose |
|--------|------|---------|
| `POST` | `/api/quality-inspection/inspections/{inspection_id}/hold` | Place on hold |
| `POST` | `/api/quality-inspection/inspections/{inspection_id}/release` | Release from hold |
| `POST` | `/api/quality-inspection/inspections/{inspection_id}/accept` | Accept inspection |
| `POST` | `/api/quality-inspection/inspections/{inspection_id}/reject` | Reject inspection |

Each transition requires authorization check against the Workforce-Competence database to verify the inspector has the required certification. Returns `403` if inspector is not authorized.

```bash
curl -X POST http://7d-quality-inspection:8106/api/quality-inspection/inspections/{inspection_id}/accept \
  -H "Authorization: Bearer $JWT" \
  -H "X-App-Id: my-vertical" \
  -H "Content-Type: application/json" \
  -d '{
    "inspector_id": "...",
    "reason": "All characteristics within tolerance"
  }'
```

### Inspection Queries

| Method | Path | Purpose |
|--------|------|---------|
| `GET` | `/api/quality-inspection/inspections/by-part-rev?part_id=&part_revision=` | Inspections by part+revision |
| `GET` | `/api/quality-inspection/inspections/by-receipt?receipt_id=` | Inspections by receipt |
| `GET` | `/api/quality-inspection/inspections/by-wo?wo_id=&inspection_type=` | Inspections by work order |
| `GET` | `/api/quality-inspection/inspections/by-lot?lot_id=` | Inspections by lot |

All query endpoints accept the specified query parameters. `part_revision` and `inspection_type` are optional filters.

---

## Numbering Module

Source: `modules/numbering/src/http/`, `modules/numbering/src/main.rs`

**Base URL:** `http://7d-numbering:8120`

Centralized, atomic, idempotent sequence numbering for all platform entities (invoices, work orders, ECOs, etc.). Supports standard (immediate) and gap-free (reserved→confirmed) modes with configurable formatting policies. Allocation routes require `numbering.allocate`.

### Allocate Number

```
POST /allocate
Authorization: Bearer <jwt>
Content-Type: application/json
```

Request body:
```json
{
  "entity": "work_order",
  "idempotency_key": "wo-create-abc123",
  "gap_free": false
}
```

Required: **`entity`** (1-100 chars), **`idempotency_key`** (1-512 chars). `gap_free` is optional (default `false`, only honoured on first allocation for a new sequence).

Response `201 Created` (new allocation) or `200 OK` (idempotent replay):
```json
{
  "tenant_id": "...",
  "entity": "work_order",
  "number_value": 42,
  "formatted_number": "WO-000042",
  "idempotency_key": "wo-create-abc123",
  "replay": false,
  "status": "confirmed",
  "expires_at": null
}
```

For gap-free sequences, `status` is `"reserved"` and `expires_at` contains the ISO 8601 expiry timestamp. Reserved numbers must be confirmed via `POST /confirm` before expiry or they will be recycled.

### Confirm Number (Gap-Free)

```
POST /confirm
Authorization: Bearer <jwt>
Content-Type: application/json
```

```json
{
  "entity": "invoice",
  "idempotency_key": "inv-create-xyz789"
}
```

Response `200 OK`:
```json
{
  "tenant_id": "...",
  "entity": "invoice",
  "number_value": 1001,
  "idempotency_key": "inv-create-xyz789",
  "status": "confirmed",
  "replay": false
}
```

Returns `404` if no reservation found, `409` if reservation is in an invalid state.

### Formatting Policies

| Method | Path | Purpose |
|--------|------|---------|
| `PUT` | `/policies/{entity}` | Upsert formatting policy |
| `GET` | `/policies/{entity}` | Get formatting policy |

**Upsert policy:**

```bash
curl -X PUT http://7d-numbering:8120/policies/work_order \
  -H "Authorization: Bearer $JWT" \
  -H "X-App-Id: my-vertical" \
  -H "Content-Type: application/json" \
  -d '{
    "pattern": "{prefix}{number}",
    "prefix": "WO-",
    "padding": 6
  }'
```

Required: **`pattern`** (must contain `{number}` token). `prefix` defaults to `""`, `padding` defaults to `0` (range 0-20).

Response `200 OK`:
```json
{
  "tenant_id": "...",
  "entity": "work_order",
  "pattern": "{prefix}{number}",
  "prefix": "WO-",
  "padding": 6,
  "version": 1
}
```

Once a policy is set, all future allocations for that entity return a `formatted_number` field in addition to the raw `number_value`.

---

## Workflow Module

Source: `modules/workflow/src/http/definitions.rs`, `modules/workflow/src/http/instances.rs`, `modules/workflow/src/main.rs`

**Base URL:** `http://7d-workflow:8110`

Generic, definition-driven workflow engine. Define step graphs with allowed transitions, then start instances against entities (e.g., purchase orders, ECOs). Every state change is recorded as a transition with full audit trail. All routes require `workflow.mutate`.

### Definitions

| Method | Path | Purpose |
|--------|------|---------|
| `POST` | `/api/workflow/definitions` | Create workflow definition |
| `GET` | `/api/workflow/definitions` | List definitions (?active_only=true&limit=50&offset=0) |
| `GET` | `/api/workflow/definitions/{def_id}` | Get definition by ID |

**Create definition:**

```bash
curl -X POST http://7d-workflow:8110/api/workflow/definitions \
  -H "Authorization: Bearer $JWT" \
  -H "X-App-Id: my-vertical" \
  -H "Content-Type: application/json" \
  -d '{
    "name": "Purchase Order Approval",
    "description": "Two-step PO approval workflow",
    "steps": [
      {
        "step_id": "pending_review",
        "label": "Pending Review",
        "allowed_transitions": ["manager_approved", "__cancelled__"]
      },
      {
        "step_id": "manager_approved",
        "label": "Manager Approved",
        "allowed_transitions": ["__completed__", "__cancelled__"]
      }
    ],
    "initial_step_id": "pending_review"
  }'
```

Required: **`name`**, **`steps`** (JSON array), **`initial_step_id`** (must match a `step_id` in steps). Steps must have unique `step_id` values.

Response `201 Created`:
```json
{
  "id": "...",
  "tenant_id": "...",
  "name": "Purchase Order Approval",
  "description": "Two-step PO approval workflow",
  "version": 1,
  "steps": [...],
  "initial_step_id": "pending_review",
  "is_active": true,
  "created_at": "2026-03-11T10:00:00Z",
  "updated_at": "2026-03-11T10:00:00Z"
}
```

Errors: `400` validation failure (empty steps, missing initial_step_id), `409` duplicate name+version.

### Instances

| Method | Path | Purpose |
|--------|------|---------|
| `POST` | `/api/workflow/instances` | Start workflow instance |
| `GET` | `/api/workflow/instances` | List instances (?entity_type=&entity_id=&status=&definition_id=&limit=50&offset=0) |
| `GET` | `/api/workflow/instances/{instance_id}` | Get instance |
| `PATCH` | `/api/workflow/instances/{instance_id}/advance` | Advance to next step |
| `GET` | `/api/workflow/instances/{instance_id}/transitions` | List transition history |

**Start instance:**

```bash
curl -X POST http://7d-workflow:8110/api/workflow/instances \
  -H "Authorization: Bearer $JWT" \
  -H "X-App-Id: my-vertical" \
  -H "Content-Type: application/json" \
  -d '{
    "definition_id": "...",
    "entity_type": "purchase_order",
    "entity_id": "PO-2026-001",
    "context": { "amount": 50000, "department": "engineering" },
    "idempotency_key": "start-po-2026-001"
  }'
```

Required: **`definition_id`**, **`entity_type`**, **`entity_id`**. Definition must be active. `idempotency_key` enables safe retries.

Response `201 Created`:
```json
{
  "id": "...",
  "tenant_id": "...",
  "definition_id": "...",
  "entity_type": "purchase_order",
  "entity_id": "PO-2026-001",
  "current_step_id": "pending_review",
  "status": "active",
  "context": { "amount": 50000, "department": "engineering" },
  "started_at": "2026-03-11T10:00:00Z",
  "completed_at": null,
  "cancelled_at": null,
  "created_at": "2026-03-11T10:00:00Z",
  "updated_at": "2026-03-11T10:00:00Z"
}
```

**Advance instance:**

```bash
curl -X PATCH http://7d-workflow:8110/api/workflow/instances/{instance_id}/advance \
  -H "Authorization: Bearer $JWT" \
  -H "X-App-Id: my-vertical" \
  -H "Content-Type: application/json" \
  -d '{
    "to_step_id": "manager_approved",
    "action": "approve",
    "actor_id": "...",
    "actor_type": "user",
    "comment": "Budget approved",
    "metadata": { "approved_amount": 50000 },
    "idempotency_key": "approve-po-2026-001"
  }'
```

Required: **`to_step_id`**, **`action`**. Target step must exist in the definition. Special pseudo-steps: `__completed__` (terminates with completed status), `__cancelled__` (terminates with cancelled status).

Response `200 OK`:
```json
{
  "instance": {
    "id": "...",
    "current_step_id": "manager_approved",
    "status": "active",
    "...": "..."
  },
  "transition": {
    "id": "...",
    "instance_id": "...",
    "from_step_id": "pending_review",
    "to_step_id": "manager_approved",
    "action": "approve",
    "actor_id": "...",
    "actor_type": "user",
    "comment": "Budget approved",
    "transitioned_at": "2026-03-11T10:05:00Z"
  }
}
```

Errors: `404` instance/definition not found, `422` invalid transition or instance not active.

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
