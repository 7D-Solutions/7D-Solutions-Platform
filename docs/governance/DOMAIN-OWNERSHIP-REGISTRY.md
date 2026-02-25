# Domain Ownership Registry

**Version**: 2.0
**Last Updated**: 2026-02-25
**Phase**: Full Platform Registry Backfill

## Purpose

This registry declares the single source of truth for each domain concept in the platform. Each domain concept has **exactly one owning module**. This prevents overlapping ownership, cross-module coupling, and inconsistent truth.

## Governance Principles

1. **Single Writer**: Only the owning module may write to its domain tables
2. **Read via API/Events**: Cross-module data access must use events or HTTP APIs
3. **No Cross-Module JOINs**: Database queries must not join tables across module boundaries
4. **Eventual Consistency**: Inter-module communication is eventually consistent via event bus
5. **Idempotent Consumers**: All event consumers must be idempotent (track processed_events)

---

## Module Domain Ownership

### AR (Accounts Receivable)

**Module Path**: `modules/ar/`
**Database**: `postgres://localhost:5432/ar_db`
**Port**: `8086`

**Domain Ownership**:
- **Customers** (`ar_customers`) - Customer master data, payment methods, delinquency status
- **Invoices** (`ar_invoices`) - Invoice lifecycle, line items, payment status
- **Invoice Line Items** (`ar_invoice_line_items`) - Detailed billing items
- **Webhook Logs** (`webhook_logs`) - Outbound webhook delivery tracking

**Domain Responsibilities**:
- Customer CRUD operations
- Invoice generation (from subscriptions or direct API)
- Invoice payment status management (from payment events)
- Revenue recognition coordination
- Webhook delivery to external systems

**External Dependencies**:
- Consumes: `subscription.bill_run.completed` (from Subscriptions)
- Consumes: `payments.payment.succeeded`, `payments.payment.failed` (from Payments)
- Produces: `ar.payment.collection.requested` (to Payments)
- Produces: `gl.posting.requested` (to GL)

---

### GL (General Ledger)

**Module Path**: `modules/gl/`
**Database**: `gl_db`
**Port**: `8090`

**Domain Ownership**:
- **Chart of Accounts** (`accounts`) - Account structure, types, classifications
- **Journal Entries** (`journal_entries`) - Double-entry transaction records
- **Journal Entry Lines** (`journal_entry_lines`) - Debit/credit line items
- **Account Balances** (`account_balances`) - Period-based balance snapshots
- **Periods** (`periods`) - Accounting period definitions and close status
- **Accruals** (`accruals`) - Accrual entries with auto-reversal scheduling
- **FX Rates** (`fx_rates`) - Exchange rate pairs with effective dates
- **RevRec Contracts** (`revrec_contracts`) - Revenue recognition contracts
- **RevRec Schedules** (`revrec_schedules`) - Recognition schedule per contract

**Domain Responsibilities**:
- Maintain chart of accounts structure
- Record journal entries from posting requests
- Calculate and store account balances
- Period close/open management
- Financial statement generation (trial balance, P&L, balance sheet, cash flow)
- Accrual creation and auto-reversal
- FX rate management and revaluation
- Revenue recognition (contracts, schedules, posting)

**External Dependencies**:
- Consumes: `gl.posting.requested` (from AR, AP, Fixed Assets, Maintenance)
- Consumes: `gl.reversal.requested` (from AR or other modules)
- Produces: `gl.posting.accepted`, `gl.posting.rejected`, `gl.accrual_created`, `gl.accrual_reversed`, `gl.fx_revaluation_posted`, `gl.fx_realized_posted`, `fx.rate_updated`, `revrec.*`

---

### Payments

**Module Path**: `modules/payments/`
**Database**: `postgres://localhost:5432/payments_db`
**Port**: `8088`

**Domain Ownership**:
- **Payment Attempts** (`payment_attempts`) - Payment transaction attempts, statuses, reconciliation
- **Payment Configurations** - Processor configuration, retry windows

**Domain Responsibilities**:
- Payment collection processing
- Payment status lifecycle (attempting → succeeded/failed/unknown)
- UNKNOWN reconciliation (timeout handling)
- Retry scheduling and gating
- Payment method processing (delegated to Tilled/Stripe)

**External Dependencies**:
- Consumes: `ar.payment.collection.requested` (from AR)
- Produces: `payments.payment.succeeded`, `payments.payment.failed` (to AR)
- External: Tilled payment processor API

---

### Subscriptions

**Module Path**: `modules/subscriptions/`
**Database**: `postgres://localhost:5432/subscriptions_db`
**Port**: `8087`

**Domain Ownership**:
- **Subscription Plans** (`subscription_plans`) - Plan definitions, pricing, schedules
- **Subscriptions** (`subscriptions`) - Customer subscription instances
- **Subscription Invoice Attempts** (`subscription_invoice_attempts`) - Billing cycle tracking

**Domain Responsibilities**:
- Subscription lifecycle (active, paused, cancelled)
- Billing cycle management (next_bill_date advancement)
- Bill run execution (trigger invoice generation)
- Cycle gating (prevent duplicate invoices per cycle)

**External Dependencies**:
- Produces: `subscription.bill_run.completed` (to AR)
- Foreign Key Reference: `ar_customer_id` (string, not enforced at DB level)

---

### Notifications

**Module Path**: `modules/notifications/`
**Database**: `postgres://localhost:5432/notifications_db`
**Port**: `8089`

**Domain Ownership**:
- **Scheduled Notifications** (`scheduled_notifications`) - Notification scheduling and delivery tracking
- **Dead-Letter Queue** (`dead_letter_queue`) - Failed delivery records for retry
- **Event Processing State** (`processed_events`) - Consumer idempotency tracking

**Domain Responsibilities**:
- Email/SMS notification delivery
- Notification scheduling and dead-letter management
- Delivery status tracking
- Low stock alert generation
- Close calendar reminders

**External Dependencies**:
- Consumes: `ar.events.invoice.issued`, `payments.events.payment.succeeded`, `payments.events.payment.failed` (from AR/Payments), `inventory.low_stock_triggered` (handler built, not yet wired)
- Produces: `notifications.delivery.succeeded`, `notifications.low_stock.alert.created`, `notifications.close_calendar.reminder`
- External: Email/SMS service providers

---

### Shipping-Receiving

**Module Path**: `modules/shipping-receiving/`
**Database**: `shipping_receiving_db`
**Port**: `8103`

**Domain Ownership**:
- **Shipments** (`shipments`) - Inbound/outbound shipment headers with direction, carrier, tracking, status
- **Shipment Lines** (`shipment_lines`) - Per-SKU line items with quantities (expected, received, accepted, rejected, shipped)
- **Shipment Status History** (`shipment_status_history`) - Append-only audit trail of status transitions (planned, not yet in schema)
- **Events Outbox** (`sr_events_outbox`) - Module outbox for NATS event publishing
- **Processed Events** (`sr_processed_events`) - Consumer idempotency and replay safety

**Domain Responsibilities**:
- Shipment lifecycle management (inbound: expected → closed; outbound: created → delivered)
- State machine guard enforcement (quantity invariants, transition validity)
- Inventory movement triggering (receipts on inbound close, issues on outbound ship)
- Traceability linkage (PO refs for inbound, sales order refs for outbound)
- Operational dashboards (open-by-status, overdue, by carrier, by direction)

**External Dependencies**:
- Consumes: `ap.po.approved` (auto-create inbound shipment)
- Consumes: `sales.so.released` (auto-create outbound shipment)
- Produces: `shipping_receiving.shipment_created`, `shipping_receiving.shipment_status_changed`, `shipping_receiving.inbound_closed`, `shipping_receiving.outbound_shipped`, `shipping_receiving.outbound_delivered`
- Calls: Inventory HTTP API (create receipt / create issue, idempotent)
- References: Party (`carrier_party_id`), AP (`po_id`, `po_line_id`), AR (`source_ref_id`)

---

### AP (Accounts Payable)

**Module Path**: `modules/ap/`
**Database**: `ap_db`
**Port**: `8093`

**Domain Ownership**:
- **Vendors** (`vendors`) - Vendor master data with party linkage
- **Purchase Orders** (`purchase_orders`) - PO headers and lifecycle
- **PO Lines** - Line items per purchase order
- **PO Receipt Links** (`po_receipt_links`) - Links between PO lines and inventory receipts
- **Vendor Bills** (`vendor_bills`) - Supplier invoices for 3-way match
- **Three-Way Match** (`three_way_match`) - PO ↔ receipt ↔ bill reconciliation
- **AP Allocations / Payment Runs** (`ap_allocations`, `payment_runs`, `payment_run_items`, `payment_run_executions`) - Vendor payment scheduling and execution
- **AP Tax Snapshots** (`ap_tax_snapshots`) - Tax calculation snapshots
- **Idempotency Keys** (`idempotency_keys`) - Consumer replay safety

**Domain Responsibilities**:
- Vendor management (CRUD, party linkage)
- Purchase order lifecycle (draft → approved → closed)
- Vendor bill matching (3-way match: PO ↔ receipt ↔ bill)
- Payment run scheduling and execution
- PO receipt linking (from inventory receipt events)

**External Dependencies**:
- Consumes: `inventory.item_received` (link receipts to PO lines)
- Produces: `ap.po_created`, `ap.po_approved`, `ap.po_closed`, `ap.po_line_received_linked`, `ap.vendor_created`, `ap.vendor_updated`, `ap.vendor_bill_created`, `ap.vendor_bill_matched`, `ap.vendor_bill_approved`, `ap.vendor_bill_voided`, `ap.payment_run_created`, `ap.payment_executed`
- References: Party (`party_id` on vendors)

---

### Inventory

**Module Path**: `modules/inventory/`
**Database**: `inventory_db`
**Port**: `8092`

**Domain Ownership**:
- **Items** (`items`) - Item master data, tracking mode (none/lot/serial)
- **Inventory Ledger** (`inventory_ledger`) - Transaction journal (receipts, issues, adjustments, transfers)
- **FIFO Layers** (`fifo_layers`) - Cost layer tracking for FIFO valuation
- **Reservations** (`inventory_reservations`) - Stock reservations against orders
- **On-Hand Projection** (`item_on_hand_projection`) - Materialized on-hand quantities
- **UOMs** (`uoms`) - Unit of measure definitions
- **Lots** (`inventory_lots`) - Lot/batch tracking
- **Serial Instances** (`inventory_serial_instances`) - Individual serial number tracking
- **Status Buckets** (`status_buckets`) - Quality/hold status categories
- **Locations** (`locations`) - Warehouse/bin locations
- **Status Transfers** (`status_transfers`) - Quality status transitions
- **Adjustments** (`adjustments`) - Inventory count adjustments
- **Cycle Counts** (`cycle_count_*`) - Cycle count headers, lines, approvals
- **Transfers** (`inv_transfers`) - Inter-location stock transfers
- **Reorder Policies** (`reorder_policies`) - Min/max and reorder point rules
- **Valuation Snapshots** (`valuation_snapshots`) - Point-in-time inventory valuation
- **Low Stock State** (`low_stock_state`) - Low stock detection state

**Domain Responsibilities**:
- Item master management
- Stock ledger (receipts, issues, adjustments, transfers)
- FIFO cost layer tracking and valuation
- Lot and serial number tracking
- Location/warehouse management
- Cycle count workflow (count → approve → adjust)
- Reorder point monitoring and low stock alerts
- Valuation snapshot generation

**External Dependencies**:
- Produces: `inventory.item_received`, `inventory.item_issued`, `inventory.adjusted`, `inventory.transfer_completed`, `inventory.low_stock_triggered`, `inventory.cycle_count_submitted`, `inventory.cycle_count_approved`, `inventory.status_changed`, `inventory.valuation_snapshot_created`
- Consumes: none

---

### Party Master

**Module Path**: `modules/party/`
**Database**: `party_db`
**Port**: `8098`

**Domain Ownership**:
- **Parties** (`parties`) - Organizations and individuals (customers, vendors, carriers, employees)
- **Party External Refs** (`party_external_refs`) - External system cross-references
- **Contacts** (`contacts`) - Contact persons linked to parties
- **Addresses** (`addresses`) - Physical/mailing addresses linked to parties

**Domain Responsibilities**:
- Canonical identity registry for all external entities
- Party lifecycle (create, update, deactivate)
- Contact and address management
- External reference linking (ERP IDs, tax IDs, etc.)

**External Dependencies**:
- Produces: `party.created`, `party.updated`, `party.deactivated`
- Consumes: none
- Referenced by: AP (vendor party_id), Shipping-Receiving (carrier_party_id), AR (customer party_id)

---

### Treasury

**Module Path**: `modules/treasury/`
**Database**: `treasury_db`
**Port**: `8094`

**Domain Ownership**:
- **Bank Accounts** - Account definitions, types (checking, savings, credit card)
- **Bank Transactions** - Individual transaction records
- **Statement Imports** - Uploaded bank statement data with content hashing
- **Reconciliation State** - Statement line matching and reconciliation status

**Domain Responsibilities**:
- Bank account management
- Statement import and parsing
- Bank reconciliation (match transactions to statements)
- Cash position tracking

**External Dependencies**:
- Consumes: `payments.payment.succeeded`, `ap.payment_executed` (auto-create bank transactions)
- Produces: none

---

### Fixed Assets

**Module Path**: `modules/fixed-assets/`
**Database**: `fixed_assets_db`
**Port**: `8104`

**Domain Ownership**:
- **Asset Categories** (`asset_categories`) - Depreciation method/life defaults per category
- **Assets** (`assets`) - Individual fixed asset records with cost basis and status
- **Depreciation Schedules** (`depreciation_schedules`) - Per-asset depreciation plans
- **Depreciation Runs** (`depreciation_runs`) - Periodic depreciation execution batches
- **Disposals** (`disposals`) - Asset disposal/retirement records
- **AP Capitalizations** (`ap_capitalizations`) - Links from AP bill lines to capitalized assets

**Domain Responsibilities**:
- Asset lifecycle (acquisition → active → disposed)
- Depreciation calculation (straight-line, declining balance)
- Periodic depreciation run execution
- Asset disposal with gain/loss calculation
- AP bill capitalization (auto-capitalize from approved vendor bills)

**External Dependencies**:
- Consumes: `ap.vendor_bill_approved` (auto-capitalize capex lines)
- Produces: asset lifecycle events, `gl.posting.requested` (depreciation/disposal GL entries)

---

### Timekeeping

**Module Path**: `modules/timekeeping/`
**Database**: `timekeeping_db`
**Port**: `8097`

**Domain Ownership**:
- **Employees** (`tk_employees`) - Employee master data for time tracking
- **Projects** (`tk_projects`) - Project/job definitions for time allocation
- **Tasks** (`tk_tasks`) - Task breakdown within projects
- **Timesheet Entries** (`tk_timesheet_entries`) - Append-only time records with billing rates
- **Approvals** (`tk_approval_requests`, `tk_approval_actions`) - Timesheet approval workflow state and audit trail
- **Allocations** (`tk_allocations`) - Resource allocation planning
- **Exports** (`tk_export_runs`) - Payroll/billing export batches with content hashing
- **Billing Rates** (`tk_billing_rates`) - Named hourly billing rates per tenant
- **Billing Runs** (`tk_billing_runs`, `tk_billing_run_entries`) - Aggregated billing records

**Domain Responsibilities**:
- Timesheet entry and editing (append-only with version history)
- Approval workflow (submit → approve → reject → recall)
- Cost allocation to projects/jobs
- Payroll/billing export generation
- Billing rate management
- GL labor cost accrual posting
- AR billable time export

**External Dependencies**:
- Produces: `timesheet_entry.created`, `timesheet_entry.corrected`, `timesheet_entry.voided`, `timesheet.submitted`, `timesheet.approved`, `timesheet.rejected`, `timesheet.recalled`, `export_run.completed`, `timekeeping.labor_cost`, `timekeeping.billable_time`
- Consumes: none

---

### Consolidation

**Module Path**: `modules/consolidation/`
**Database**: `consolidation_db`
**Port**: `8105`

**Domain Ownership**:
- **Consolidation Configs** (`consolidation_config`) - Multi-entity consolidation rules
- **Consolidation Caches** (`consolidation_caches`) - Cached consolidated balances
- **Elimination Postings** (`elimination_postings`) - Intercompany elimination entries

**Domain Responsibilities**:
- Multi-entity financial consolidation
- Intercompany elimination generation
- Consolidated balance calculation and caching

**External Dependencies**:
- Reads: GL, AR, AP data via HTTP APIs (read-only aggregation)
- Produces: none

---

### Integrations

**Module Path**: `modules/integrations/`
**Database**: `integrations_db`
**Port**: `8099`

**Domain Ownership**:
- **Connector Configs** (`connector_configs`) - External system connection settings
- **External Refs** (`external_refs`) - Cross-system entity reference mapping
- **Webhook Endpoints** (`webhook_endpoints`) - Registered webhook receiver configs
- **Webhook Ingest** (`webhook_ingest`) - Inbound webhook event log

**Domain Responsibilities**:
- External system connector management
- Webhook ingestion, validation, and routing
- Cross-system entity reference tracking
- External ref lifecycle (create, update, delete)

**External Dependencies**:
- Produces: `external_ref.created`, `external_ref.updated`, `external_ref.deleted`, `webhook.received`, `webhook.routed`
- Consumes: various (routes inbound webhooks to target modules)

---

### Maintenance

**Module Path**: `modules/maintenance/`
**Database**: `maintenance_db`
**Port**: `8101`

**Domain Ownership**:
- **Work Orders** - Corrective and preventive maintenance tasks
- **Preventive Maintenance Plans** - Scheduled/meter-based maintenance triggers
- **Meter Readings** (`meter_readings`) - Equipment meter tracking
- **Parts and Labor** (`work_order_parts`, `work_order_labor`) - Cost tracking per work order
- **Tenant Config** (`maintenance_tenant_config`) - Per-tenant maintenance settings

**Domain Responsibilities**:
- Work order lifecycle (open → in_progress → completed → closed)
- Preventive maintenance scheduling (time-based and meter-based)
- Meter reading recording and threshold detection
- Parts/labor cost tracking per work order
- Overdue detection and notification
- GL cost posting for completed work orders (planned, not yet implemented)

**External Dependencies**:
- Produces: `maintenance.work_order.created`, `maintenance.work_order.status_changed`, `maintenance.work_order.completed`, `maintenance.work_order.closed`, `maintenance.work_order.cancelled`, `maintenance.work_order.overdue`, `maintenance.meter_reading.recorded`, `maintenance.plan.due`, `maintenance.plan.assigned`
- Consumes: none

---

### Reporting

**Module Path**: `modules/reporting/`
**Database**: `reporting_db`
**Port**: `8096`

**Domain Ownership**:
- **Report Definitions** - Configured report templates
- **Reporting Caches** (`reporting_caches`) - Pre-computed report data
- **Forecast Caches** (`forecast_caches`) - Forward-looking projection data

**Domain Responsibilities**:
- Cross-module data aggregation (read-only)
- Report generation and caching
- Financial forecasting projections
- Dashboard data serving

**External Dependencies**:
- Consumes: `gl.posting.requested`, `payments.payment.succeeded`, `ap.vendor_bill_created`, `ap.vendor_bill_voided`, `ap.payment_executed`, `inventory.valuation_snapshot_created`, `ar.invoice_opened`, `ar.invoice_paid`, `ar.ar_aging_updated`
- Produces: none (read-only module)

---

### PDF Editor

**Module Path**: `modules/pdf-editor/`
**Database**: `pdf_editor_db`
**Port**: `8106`

**Domain Ownership**:
- **Form Templates** (`form_templates`) - Form template definitions with field layouts
- **Form Fields** (`form_fields`) - Per-template field positions, types, validation rules
- **Form Submissions** (`form_submissions`) - Filled form data submissions (draft/submitted)

**Domain Responsibilities**:
- Form template management (CRUD)
- Form field annotation configuration
- Document generation from template + data (stateless — PDFs are returned as response bytes, not stored)
- Form submission processing and validation

**External Dependencies**:
- Produces: `pdf.form.submitted` (planned), `pdf.document.generated` (planned)
- Consumes: none (called via HTTP API by other modules)

---

### TTP (TrashTech Pro)

**Module Path**: `modules/ttp/`
**Database**: `ttp_db`
**Port**: `8100`

**Domain Ownership**:
- **TTP Tenants/Service Configs** - Product-level tenant configuration
- **Metering Events** - Usage/event ingestion for billing
- **Billing Runs** (`billing_runs`) - Periodic billing execution batches
- **Billing Run Items** (`billing_run_items`) - Per-customer billing run line items
- **Billing Traces** (`billing_traces`) - Audit trail for billing calculations

**Domain Responsibilities**:
- Usage metering event ingestion
- Billing run execution (calculate charges from metering data)
- Billing trace/audit for charge verification
- Invoice creation coordination (calls AR API)

**External Dependencies**:
- Produces: `ttp.billing_run.created`, `ttp.billing_run.completed`, `ttp.billing_run.failed`, `ttp.party.invoiced`
- Calls: AR HTTP API (create invoices from billing runs)
- Consumes: none

---

## Inter-Module Command Registry

This section declares all cross-module commands (events) and their write degradation characteristics.

### Command: `Subscriptions → AR (Invoice Creation)`

**Producer**: Subscriptions
**Consumer**: AR
**Protocol**: Hybrid (HTTP API + Event Outbox)
**Trigger**: Monthly/annual billing cycle execution
**Degradation Class**: **Critical**

**Execution Pattern**:
1. **Synchronous HTTP Call**: `POST /api/ar/invoices` (create + finalize)
2. **Asynchronous Event**: `subscription.bill_run.completed` via outbox (optional notification)

**Timeout Budget**:
- **HTTP Request**: 30 seconds per call (15s create + 15s finalize)
- **Total Operation**: 60 seconds (including gating checks)
- **Outbox Publish**: 5 seconds per NATS publish attempt

**Retry Policy**:
- **HTTP Failures**: NO automatic retry (cycle gating prevents duplicates)
  - Mark attempt as 'failed' in subscription_invoice_attempts
  - Operator must investigate and manually retry via bill run API
- **Outbox Publish Failures**: Infinite retry with 1-second polling interval
  - Events remain in outbox until successfully published
  - No timeout limit (eventual consistency)

**Degradation Behavior**:
| AR State | Subscriptions Behavior | Impact |
|----------|------------------------|--------|
| **Healthy** | Invoice created within 30s | Normal operation |
| **Slow (15-30s)** | Invoice created with latency | Acceptable degradation |
| **Timeout (>30s)** | HTTP call fails, attempt marked 'failed' | Billing cycle blocked, requires operator intervention |
| **Down (sustained)** | All invoice creation fails, cycles accumulate | Revenue recognition halted, manual recovery needed |

**Failure Mode**: If AR cannot process invoice creation, customer invoices are not generated, blocking revenue recognition and cash collection.

**Recovery**:
1. Check subscription_invoice_attempts table for 'failed' attempts
2. Investigate AR module health (logs, database, connectivity)
3. Retry failed cycles via `POST /api/subscriptions/bill-run` endpoint
4. Verify invoice creation in AR database

**Monitoring**:
- Alert on HTTP timeout rate > 5%
- Alert on failed cycle attempts > 10 per hour
- Dashboard: Invoice creation latency p50/p95/p99

---

### Event: `subscription.bill_run.completed` (Deprecated - See Above)

**Note**: This event is documented above as part of the hybrid Subscriptions→AR command flow.

---

### Event: `ar.payment.collection.requested`

**Producer**: AR
**Consumer**: Payments
**Trigger**: Invoice creation or retry scheduler
**Payload**: Invoice ID, customer ID, amount, due date
**Degradation Class**: **Critical**

**Failure Mode**: If Payments cannot process this event, payment collection is delayed, impacting cash flow.
**Recovery**: Replay from outbox, manual payment initiation.

---

### Event: `payments.payment.succeeded`

**Producer**: Payments
**Consumer**: AR
**Trigger**: Successful payment processing or UNKNOWN reconciliation
**Payload**: Payment ID, invoice ID, amount, transaction ID
**Degradation Class**: **Critical**

**Failure Mode**: If AR cannot process this event, invoice status remains 'open', customer may be incorrectly charged again.
**Recovery**: Replay from outbox, manual invoice status update.

---

### Event: `payments.payment.failed`

**Producer**: Payments
**Consumer**: AR
**Trigger**: Payment decline or final retry exhaustion
**Payload**: Payment ID, invoice ID, failure reason
**Degradation Class**: **High**

**Failure Mode**: If AR cannot process this event, customer delinquency status is not updated, dunning process may fail.
**Recovery**: Replay from outbox, manual status reconciliation.

---

### Event: `gl.posting.requested`

**Producer**: AR (or other modules)
**Consumer**: GL
**Trigger**: Invoice payment, manual journal entry
**Payload**: Journal entry lines (account, debit/credit, amount), idempotency key
**Degradation Class**: **Critical**

**Failure Mode**: If GL cannot process this event, financial records are incomplete, financial statements are inaccurate.
**Recovery**: Replay from outbox, manual journal entry via GL API.

---

### Event: `invoice.issued` / `payment.succeeded` / `payment.failed`

**Producer**: AR / Payments
**Consumer**: Notifications
**Trigger**: Invoice creation, payment status change
**Payload**: Recipient email, event type, metadata
**Degradation Class**: **Low** (Degraded Mode Acceptable)

**Failure Mode**: If Notifications cannot process this event, customer does not receive email, but business operations continue.
**Recovery**: Replay from outbox, notifications can be delayed without financial impact.

---

## Write Degradation Classification

**P0 - Critical**: System cannot function correctly without this write. Requires immediate recovery.
- GL journal entries
- Invoice status updates (payment succeeded/failed)
- Payment attempt records
- Subscription → Invoice mapping (prevents duplicate billing)

**P1 - High**: System can operate but with significant degradation. Requires recovery within hours.
- Customer delinquency status
- Retry scheduling updates
- Period close operations

**P2 - Degraded Mode Acceptable**: System operates normally, user experience slightly degraded. Can retry indefinitely.
- Notification delivery
- Webhook logs
- Audit trail writes

---

## Cross-Module Access Patterns

### ✅ Allowed Patterns

1. **Event-Driven Updates**: Module A emits event → Module B consumes via event bus
2. **HTTP API Calls**: Module A calls Module B's REST API (synchronous, for reads only)
3. **Foreign Key References (Logical)**: Store string IDs from other modules (e.g., `ar_customer_id` in subscriptions) without DB-level foreign key constraints

### ❌ Prohibited Patterns

1. **Cross-Module Database JOINs**: `SELECT * FROM ar.invoices JOIN payments.payment_attempts` ❌
2. **Direct Table Writes**: Module A writing to Module B's tables ❌
3. **Shared Database**: Multiple modules writing to the same database ❌
4. **Synchronous Blocking Calls**: Module A waiting for Module B's write before proceeding (use eventual consistency) ❌

---

## Enforcement Mechanisms

1. **Database Separation**: Each module has its own PostgreSQL database
2. **Code Review**: All PRs must respect domain ownership boundaries
3. **Integration Tests**: Cross-module tests verify event flows, not direct table access
4. **Linting**: Custom linters detect cross-schema SQL references
5. **Invariant Assertions**: `e2e-tests/tests/oracle.rs` validates module boundaries

---

## Domain Ownership Disputes

If a domain concept's ownership is unclear:

1. **Consult This Registry First**: Check if ownership is already declared
2. **Propose Amendment**: Open GitHub issue with proposal for ownership assignment
3. **Review Criteria**:
   - Which module is the natural source of truth?
   - Which module needs to enforce invariants?
   - Which module has the highest write frequency?
4. **Update This Registry**: After approval, update this document and increment version

---

## Version History

| Version | Date       | Author        | Changes                                      |
|---------|------------|---------------|----------------------------------------------|
| 1.0     | 2026-02-16 | ChartreuseFox | Initial domain ownership registry for Phase 16 |

---

## References

- [Phase 16 Architecture](../../MEMORY.md#current-status-2026-02-16)
- [EventEnvelope Constitutional Metadata](../../platform/event-bus/src/envelope.rs)
- [Cross-Module Invariant Oracle](../../e2e-tests/tests/oracle.rs)
- [Module-Level Invariants](../bd-35x/) (AR, Payments, Subscriptions, GL)
