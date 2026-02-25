# Error Code Registry

Canonical list of error codes returned by each module. All HTTP error responses
MUST use codes from this registry. Error codes follow the pattern
`<lowercase_snake_case>` and are returned in the `error` field of the JSON
response body alongside a human-readable `message`.

**Response shape:**

```json
{
  "error": "validation_error",
  "message": "amount_cents must be non-negative"
}
```

> **Note:** Some modules (TTP) use a `code` field instead of `error`. These will
> be aligned in a future bead.

---

## Cross-Module (shared by all modules)

| Code | HTTP Status | Description |
|------|-------------|-------------|
| `unauthorized` | 401 | Missing or invalid authentication token. |
| `forbidden` | 403 | Caller lacks required permissions (e.g. admin token invalid). |
| `not_found` | 404 | Generic resource not found. |
| `validation_error` | 400 / 422 | Request body or query params failed validation. |
| `database_error` | 500 | Internal database error (details hidden from client). |
| `internal_error` | 500 | Catch-all internal server error. |
| `conflict` | 409 | Resource already exists or concurrent modification detected. |
| `rate_limit_exceeded` | 429 | Request rate limit exceeded; retry after backoff. |

---

## AP (Accounts Payable)

### Vendors

| Code | HTTP Status | Description |
|------|-------------|-------------|
| `vendor_not_found` | 404 | Vendor with given ID not found. |
| `duplicate_vendor_name` | 409 | Active vendor with this name already exists for the tenant. |

### Bills

| Code | HTTP Status | Description |
|------|-------------|-------------|
| `bill_not_found` | 404 | Bill with given ID not found. |
| `duplicate_invoice` | 409 | Invoice reference already exists for this vendor. |
| `invalid_transition` | 422 | Bill cannot transition between the requested statuses. |
| `empty_lines` | 422 | Bill must have at least one line item. |
| `match_policy_violation` | 422 | Bill does not meet the configured matching policy. |
| `tax_error` | 422 | Tax calculation or validation failed. |

### Purchase Orders

| Code | HTTP Status | Description |
|------|-------------|-------------|
| `po_not_found` | 404 | Purchase order with given ID not found. |
| `po_not_draft` | 422 | PO cannot be edited because it is not in draft status. |
| `invalid_transition` | 422 | PO cannot transition between the requested statuses. |
| `empty_lines` | 422 | PO must have at least one line item. |

### Bill Matching

| Code | HTTP Status | Description |
|------|-------------|-------------|
| `invalid_bill_status` | 422 | Bill status does not allow matching. |
| `no_matchable_lines` | 422 | Bill has no lines to match. |

### Allocations

| Code | HTTP Status | Description |
|------|-------------|-------------|
| `invalid_bill_status` | 422 | Bill status does not accept allocations. |
| `over_allocation` | 422 | Allocation would exceed open balance. |

### Payment Runs

| Code | HTTP Status | Description |
|------|-------------|-------------|
| `no_eligible_bills` | 422 | No eligible bills found for tenant/currency combination. |
| `duplicate_run_id` | 409 | Payment run ID already exists for a different tenant. |
| `invalid_status` | 409 | Payment run cannot be executed in its current status. |

### Tax Reports

| Code | HTTP Status | Description |
|------|-------------|-------------|
| `invalid_range` | 400 | `from` date must be before `to` date. |

---

## AR (Accounts Receivable)

### Customers

| Code | HTTP Status | Description |
|------|-------------|-------------|
| `validation_error` | 400 | Customer field validation failed (e.g. email required). |
| `not_found` | 404 | Customer not found. |

### Invoices

| Code | HTTP Status | Description |
|------|-------------|-------------|
| `not_found` | 404 | Invoice or related resource not found. |
| `outbox_error` | 500 | Failed to enqueue event to outbox. |
| `transaction_error` | 500 | Failed to commit database transaction. |
| `party_service_unavailable` | 503 | Party Master service is unreachable. |
| `party_not_found` | 422 | Referenced party does not exist. |

### Charges

| Code | HTTP Status | Description |
|------|-------------|-------------|
| `not_found` | 404 | Charge not found. |

### Refunds

| Code | HTTP Status | Description |
|------|-------------|-------------|
| `not_found` | 404 | Charge or refund not found. |

### Disputes

| Code | HTTP Status | Description |
|------|-------------|-------------|
| `not_found` | 404 | Dispute not found. |

### Credit Notes

| Code | HTTP Status | Description |
|------|-------------|-------------|
| `credit_note_error` | 422 | Credit note creation or processing failed. |
| `invalid_currency` | 400 | Currency must not be empty. |

### Write-Offs

| Code | HTTP Status | Description |
|------|-------------|-------------|
| `write_off_error` | 422 | Write-off processing failed. |

### Webhooks

| Code | HTTP Status | Description |
|------|-------------|-------------|
| `signature_error` | 400 | Webhook signature verification failed. |
| `not_found` | 404 | Webhook not found. |
| `invalid_webhook` | 422 | Webhook has no payload. |
| `processing_error` | 500 | Webhook processing failed. |

### Payment Allocation

| Code | HTTP Status | Description |
|------|-------------|-------------|
| `validation_error` | 422 | Allocation validation failed. |

### Reconciliation

| Code | HTTP Status | Description |
|------|-------------|-------------|
| `recon_error` | 500 | Reconciliation processing failed. |
| `recon_schedule_error` | 500 | Reconciliation scheduling failed. |

### Usage Billing

| Code | HTTP Status | Description |
|------|-------------|-------------|
| `outbox_error` | 500 | Failed to enqueue usage event. |
| `transaction_error` | 500 | Failed to commit usage transaction. |

### Aging

| Code | HTTP Status | Description |
|------|-------------|-------------|
| `database_error` | 500 | Aging report query or refresh failed. |

### Idempotency

| Code | HTTP Status | Description |
|------|-------------|-------------|
| `auth_error` | 401 | Missing app_id for idempotency. |

### Auth Middleware

| Code | HTTP Status | Description |
|------|-------------|-------------|
| `unauthorized` | 401 | Invalid or missing bearer token / API key. |
| `rate_limit_exceeded` | 429 | Rate limit exceeded for the API key. |

### Tax

| Code | HTTP Status | Description |
|------|-------------|-------------|
| Errors returned via `ErrorBody { error }` with dynamic messages. |

---

## GL (General Ledger)

### Period Close

| Code | HTTP Status | Description |
|------|-------------|-------------|
| _(uses `ErrorResponse { error }` with dynamic messages from `PeriodCloseError`)_ | | |
| Period not found | 404 | Period with given ID not found. |
| Period already closed | 409 | Period has already been closed. |
| Validation failed | 400 | Pre-close validation checks failed. |
| Hash mismatch | 500 | Data integrity hash mismatch. |
| FX revaluation failed | 500 | FX revaluation step failed during close. |

### Revenue Recognition

| Code | HTTP Status | Description |
|------|-------------|-------------|
| _(uses `ErrorResponse { error }` with dynamic messages from `RevrecRepoError`)_ | | |
| Duplicate contract | 409 | Contract already exists (idempotent). |
| Duplicate schedule | 409 | Schedule already exists (idempotent). |
| Obligation not found | 404 | Obligation with given ID not found. |
| Allocation mismatch | 400 | Obligation allocation sum does not match expected total. |
| Schedule sum mismatch | 400 | Schedule lines do not sum to expected total. |
| Contract not found | 404 | Contract with given ID not found. |
| Duplicate modification | 409 | Modification already exists (idempotent). |

### FX Rates

| Code | HTTP Status | Description |
|------|-------------|-------------|
| _(uses `FxRateErrorResponse` with dynamic messages)_ | | |
| Unauthorized | 401 | Missing authentication. |
| Validation error | 400 | Invalid FX rate input. |
| Not found | 404 | FX rate not found. |

### Balance Sheet / Income Statement / Trial Balance

| Code | HTTP Status | Description |
|------|-------------|-------------|
| _(uses per-report `ErrorResponse { error }` with dynamic messages)_ | | |
| Unauthorized | 401 | Missing authentication. |
| Database error | 500 | Report query failed. |

### Cash Flow

| Code | HTTP Status | Description |
|------|-------------|-------------|
| Validation error | 400 | Invalid date range (from > to). |
| Unauthorized | 401 | Missing authentication. |

### Reporting Currency

| Code | HTTP Status | Description |
|------|-------------|-------------|
| Validation error | 400 | Invalid or empty reporting currency. |
| Unauthorized | 401 | Missing authentication. |

### Accruals

| Code | HTTP Status | Description |
|------|-------------|-------------|
| _(uses `ErrorBody { error }` with `AccrualError.to_string()`)_ | | |
| Validation error | 400 | Accrual validation failed. |

---

## TTP (Tenant Transaction Pricing)

> **Note:** TTP uses `{ error, code }` instead of `{ error, message }`.

### Metering

| Code | HTTP Status | Description |
|------|-------------|-------------|
| `unauthorized` | 401 | Missing or invalid authentication. |
| `validation_error` | 400 | Invalid event data (empty dimension, non-positive quantity, etc.). |
| `ingestion_failed` | 500 | Event ingestion failed. |
| `trace_failed` | 500 | Price trace computation failed. |

### Billing Runs

| Code | HTTP Status | Description |
|------|-------------|-------------|
| `unauthorized` | 401 | Missing or invalid authentication. |
| `validation_error` | 400 | Invalid billing period format or empty idempotency key. |
| `tenant_not_found` | 404 | Tenant not found in registry. |
| `no_app_id` | 422 | Tenant has no app_id assigned. |
| `billing_run_failed` | 500 | Billing run failed. |

### Service Agreements

| Code | HTTP Status | Description |
|------|-------------|-------------|
| `unauthorized` | 401 | Missing or invalid authentication. |
| `validation_error` | 400 | Invalid status filter. |
| `db_error` | 500 | Database query or row mapping failed. |

---

## Payments

### Checkout Sessions

| Code | HTTP Status | Description |
|------|-------------|-------------|
| _(uses `ApiError { status, message }` — no structured error codes)_ | | |
| Missing app_id | 401 | Merchant context missing. |
| Validation errors | 400 | Invalid payment amount, currency, etc. |
| Not found | 404 | Checkout session not found. |
| Internal error | 500 | Payment processing or database error. |

---

## Party Master

| Code | HTTP Status | Description |
|------|-------------|-------------|
| `party_not_found` | 404 | Party with given ID not found. |
| `not_found` | 404 | Address or contact not found. |
| `conflict` | 409 | Duplicate party or concurrent modification. |

---

## Inventory

| Code | HTTP Status | Description |
|------|-------------|-------------|
| `not_found` | 404 | Item, location, UoM, reorder policy, or valuation snapshot not found. |
| `validation_error` | 422 | Input validation failed. |
| `uom_conversion_error` | 422 | Unit of measure conversion failed. |
| `fifo_error` | 422 | FIFO costing layer error (insufficient stock). |
| `guard_error` | 422 | Domain guard check failed. |
| `task_not_found` | 404 | Cycle count task not found. |
| `task_not_open` | 422 | Cycle count task is not in open status. |
| `task_not_submitted` | 422 | Cycle count task is not in submitted status. |
| `line_not_found` | 404 | Cycle count line not found. |
| `idempotency_conflict` | 409 | Idempotency key conflict on cycle count. |

---

## Shipping & Receiving

| Code | HTTP Status | Description |
|------|-------------|-------------|
| `not_found` | 404 | Shipment or shipment line not found. |
| `validation_error` | 422 | Input validation failed (e.g. negative quantity). |
| `invalid_transition` | 422 | Shipment cannot transition between statuses. |
| `guard_failed` | 422 | Domain guard check failed. |
| `inventory_error` | 500 | Inventory integration failed. |

---

## Fixed Assets

| Code | HTTP Status | Description |
|------|-------------|-------------|
| _(uses `{ error, message }` via inline `json!` with dynamic codes)_ | | |
| Asset errors | 400/404/500 | Dynamic error code and message from domain. |
| Depreciation errors | 400/404/500 | Dynamic error code and message from domain. |
| Disposal errors | 400/404/500 | Dynamic error code and message from domain. |

---

## Consolidation

### Config

| Code | HTTP Status | Description |
|------|-------------|-------------|
| `group_not_found` | 404 | Consolidation group not found. |
| `entity_not_found` | 404 | Consolidation entity not found. |
| `rule_not_found` | 404 | Elimination rule not found. |
| `policy_not_found` | 404 | Translation policy not found. |
| `mapping_not_found` | 404 | Account mapping not found. |
| `conflict` | 409 | Duplicate or conflicting configuration. |

### Intercompany / Consolidate

| Code | HTTP Status | Description |
|------|-------------|-------------|
| _(uses `{ error }` with dynamic message string — no structured code)_ | | |

---

## Treasury

### Accounts

| Code | HTTP Status | Description |
|------|-------------|-------------|
| `validation_error` | 422 | Account validation failed. |
| `idempotent_replay` | 409 | Request already processed (idempotent replay). |
| `replay_error` | 500 | Failed to deserialize cached idempotent response. |

### Reconciliation

| Code | HTTP Status | Description |
|------|-------------|-------------|
| `validation_error` | 422 | Reconciliation input validation failed. |

### Import

| Code | HTTP Status | Description |
|------|-------------|-------------|
| `bad_request` | 400 | Invalid import request. |
| `validation_error` | 422 | Import data validation failed. |

### Reports

| Code | HTTP Status | Description |
|------|-------------|-------------|
| `database_error` | 500 | Cash position computation failed. |

---

## Timekeeping

### Employees

| Code | HTTP Status | Description |
|------|-------------|-------------|
| `not_found` | 404 | Employee not found. |
| `duplicate_code` | 409 | Employee code already exists. |

### Projects

| Code | HTTP Status | Description |
|------|-------------|-------------|
| `not_found` | 404 | Project or task not found. |
| `project_not_found` | 404 | Parent project not found. |
| `duplicate_code` | 409 | Project or task code already exists. |
| `database_error` | 500 | Database query failed. |

### Entries

| Code | HTTP Status | Description |
|------|-------------|-------------|
| `not_found` | 404 | Time entry not found. |
| `period_locked` | 422 | Entry falls in a locked period. |
| `overlap` | 409 | Time entry overlaps an existing entry. |

### Approvals

| Code | HTTP Status | Description |
|------|-------------|-------------|
| `not_found` | 404 | Approval request not found. |
| `invalid_transition` | 422 | Invalid approval status transition. |
| `duplicate` | 409 | Duplicate approval request. |

### Allocations

| Code | HTTP Status | Description |
|------|-------------|-------------|
| `not_found` | 404 | Allocation not found. |

### Billing

| Code | HTTP Status | Description |
|------|-------------|-------------|
| `no_billable_entries` | 422 | No billable entries found for the period. |

### Export

| Code | HTTP Status | Description |
|------|-------------|-------------|
| `not_found` | 404 | Export run not found. |
| `no_approved_entries` | 422 | No approved entries found for export. |
| `idempotent_replay` | 200 | Export already processed (idempotent replay). |

---

## Integrations

### Connectors

| Code | HTTP Status | Description |
|------|-------------|-------------|
| `unknown_connector_type` | 400 | Connector type is not recognized. |
| `invalid_config` | 422 | Connector configuration is invalid. |
| `action_failed` | 500 | Connector action execution failed. |
| `not_found` | 404 | Connector config not found. |

### External Refs

| Code | HTTP Status | Description |
|------|-------------|-------------|
| `not_found` | 404 | External reference not found. |
| `conflict` | 409 | Duplicate external reference. |

### Webhooks

| Code | HTTP Status | Description |
|------|-------------|-------------|
| _(uses `{ error }` with dynamic message — no structured code field)_ | | |
| Auth error | 401 | Missing or invalid authentication. |
| Invalid JSON | 400 | Malformed webhook payload. |
| Signature failed | 400 | Webhook signature verification failed. |
| Unknown system | 400 | Unknown webhook system type. |

---

## Maintenance

### Work Orders

| Code | HTTP Status | Description |
|------|-------------|-------------|
| `not_found` | 404 | Work order, asset, or plan assignment not found. |
| `invalid_transition` | 422 | Work order cannot transition between statuses. |
| `guard_failed` | 422 | Domain guard check failed. |

### Plans

| Code | HTTP Status | Description |
|------|-------------|-------------|
| `not_found` | 404 | Plan, asset, meter type, or assignment not found. |

### Meters

| Code | HTTP Status | Description |
|------|-------------|-------------|
| `not_found` | 404 | Meter type or asset not found. |

### Assets

| Code | HTTP Status | Description |
|------|-------------|-------------|
| `not_found` | 404 | Asset not found. |

### Labor / Parts

| Code | HTTP Status | Description |
|------|-------------|-------------|
| `not_found` | 404 | Work order or labor/part entry not found. |

---

## PDF Editor

### Templates

| Code | HTTP Status | Description |
|------|-------------|-------------|
| `not_found` | 404 | Template or field not found. |
| `duplicate_field_key` | 409 | Field key already exists on the template. |

### Submissions

| Code | HTTP Status | Description |
|------|-------------|-------------|
| `not_found` | 404 | Submission or template not found. |
| `already_submitted` | 409 | Submission has already been submitted. |

---

## Subscriptions

| Code | HTTP Status | Description |
|------|-------------|-------------|
| _(Only admin endpoints exposed; uses shared cross-module codes)_ | | |

---

## Reporting

| Code | HTTP Status | Description |
|------|-------------|-------------|
| _(Uses shared cross-module codes: `unauthorized`, `validation_error`, `internal_error`, `forbidden`)_ | | |

---

## Notifications

| Code | HTTP Status | Description |
|------|-------------|-------------|
| _(Only admin endpoints exposed; uses shared cross-module codes)_ | | |
