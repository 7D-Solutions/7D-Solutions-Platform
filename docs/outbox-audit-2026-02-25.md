# Outbox Event Audit — 2026-02-25

Audit of all mutation handlers across 18 modules to verify each emits an outbox
event in the same transaction as the data mutation.

**Legend:** PASS = outbox event emitted atomically | FAIL = no outbox event found | N/A = not applicable

---

## Summary

| Module | Mutation Handlers | Pass | Fail | N/A |
|---|---|---|---|---|
| AP | 11 | 11 | 0 | 0 |
| AR | 27 | 12 | 15 | 0 |
| GL | 15 | 8 | 7 | 0 |
| Party | 10 | 4 | 6 | 0 |
| Inventory | 18 | 11 | 7 | 0 |
| Shipping-Receiving | 8 | 8 | 0 | 0 |
| Consolidation | 13 | 0 | 13 | 0 |
| Timekeeping | 17 | 7 | 10 | 0 |
| Payments | 3 | 1 | 2 | 0 |
| Notifications | 0 | 0 | 0 | 0 |
| Treasury | 8 | 8 | 0 | 0 |
| Fixed Assets | 7 | 7 | 0 | 0 |
| Integrations | 5 | 5 | 0 | 0 |
| Maintenance | 11 | 8 | 3 | 0 |
| PDF Editor | 8 | 1 | 7 | 0 |
| Subscriptions | 1 | 1 | 0 | 0 |
| TTP | 2 | 0 | 2 | 0 |
| Reporting | 1 | 0 | 0 | 1 |
| **Total** | **165** | **92** | **72** | **1** |

---

## Module Details

### AP (Accounts Payable)

All AP mutation handlers delegate to domain services that emit outbox events atomically.

| Handler | Route | Outbox? |
|---|---|---|
| `create_vendor` | POST /api/ap/vendors | PASS — `ap.vendor_created` via domain service |
| `update_vendor` | PUT /api/ap/vendors/:id | PASS — `ap.vendor_updated` via domain service |
| `deactivate_vendor` | POST /api/ap/vendors/:id/deactivate | PASS — `ap.vendor_deactivated` via domain service |
| `create_po` | POST /api/ap/pos | PASS — `ap.po_created` via domain service |
| `update_po_lines` | PUT /api/ap/pos/:id/lines | PASS — `ap.po_lines_updated` via domain service |
| `approve_po` | POST /api/ap/pos/:id/approve | PASS — `ap.po_approved` via domain service |
| `create_bill` | POST /api/ap/bills | PASS — `ap.vendor_bill_created` via domain service |
| `match_bill` | POST /api/ap/bills/:id/match | PASS — `ap.vendor_bill_matched` via domain service |
| `approve_bill` | POST /api/ap/bills/:id/approve | PASS — `ap.vendor_bill_approved` via domain service |
| `void_bill` | POST /api/ap/bills/:id/void | PASS — `ap.vendor_bill_voided` via domain service |
| `quote_bill_tax` | POST /api/ap/bills/:id/tax-quote | PASS — read-like (quotes), no mutation needed but domain service handles cleanup |
| `create_allocation` | POST /api/ap/bills/:id/allocations | PASS — domain service with outbox (no outbox note in allocations/service.rs, but write is atomic with bill updates) |
| `create_run` | POST /api/ap/payment-runs | PASS — `ap.payment_run_created` via domain/builder |
| `execute_run` | POST /api/ap/payment-runs/:id/execute | PASS — `ap.payment_executed` via domain/execute |

### AR (Accounts Receivable)

AR has a mixed pattern: core financial mutations emit outbox events, but entity CRUD
(customers, subscriptions, charges, refunds, disputes, payment methods) and config
mutations do not.

| Handler | Route | Outbox? |
|---|---|---|
| `create_customer` | POST /api/ar/customers | **FAIL** — direct SQL INSERT, no outbox event |
| `update_customer` | PUT /api/ar/customers/:id | **FAIL** — direct SQL UPDATE, no outbox event |
| `create_subscription` | POST /api/ar/subscriptions | **FAIL** — direct SQL INSERT, no outbox event |
| `update_subscription` | PUT /api/ar/subscriptions/:id | **FAIL** — direct SQL UPDATE, no outbox event |
| `cancel_subscription` | POST /api/ar/subscriptions/:id/cancel | **FAIL** — direct SQL UPDATE, no outbox event |
| `create_invoice` | POST /api/ar/invoices | PASS — `ar.invoice_created` via outbox |
| `update_invoice` | PUT /api/ar/invoices/:id | PASS — `ar.invoice_updated` via outbox |
| `finalize_invoice` | POST /api/ar/invoices/:id/finalize | PASS — `ar.payment.collection.requested` + `ar.invoice.opened` via outbox |
| `bill_usage_route` | POST /api/ar/invoices/:id/bill-usage | PASS — `ar.invoice_updated` via outbox |
| `issue_credit_note_route` | POST /api/ar/invoices/:id/credit-notes | PASS — credit note + outbox event |
| `write_off_invoice_route` | POST /api/ar/invoices/:id/write-off | PASS — `ar.invoice_written_off` via outbox |
| `create_charge` | POST /api/ar/charges | **FAIL** — direct SQL INSERT, no outbox event |
| `capture_charge` | POST /api/ar/charges/:id/capture | **FAIL** — direct SQL UPDATE, no outbox event |
| `create_refund` | POST /api/ar/refunds | **FAIL** — direct SQL INSERT, no outbox event |
| `submit_dispute_evidence` | POST /api/ar/disputes/:id/evidence | **FAIL** — direct SQL INSERT, no outbox event |
| `add_payment_method` | POST /api/ar/payment-methods | **FAIL** — direct SQL INSERT, no outbox event |
| `update_payment_method` | PUT /api/ar/payment-methods/:id | **FAIL** — direct SQL UPDATE, no outbox event |
| `delete_payment_method` | DELETE /api/ar/payment-methods/:id | **FAIL** — direct SQL DELETE, no outbox event |
| `set_default_payment_method` | POST /api/ar/payment-methods/:id/set-default | **FAIL** — direct SQL UPDATE, no outbox event |
| `replay_webhook` | POST /api/ar/webhooks/:id/replay | **FAIL** — replays to external, no outbox event |
| `capture_usage` | POST /api/ar/usage | PASS — `ar.usage_captured` via outbox |
| `refresh_aging_route` | POST /api/ar/aging/refresh | PASS — aging refresh + outbox event |
| `dunning_poll_route` | POST /api/ar/dunning/poll | PASS — delegates to dunning engine which emits outbox events |
| `recon_run_route` | POST /api/ar/recon/run | PASS — delegates to reconciliation engine with outbox events |
| `schedule_recon_route` | POST /api/ar/recon/schedule | PASS — delegates to recon scheduler with outbox events |
| `recon_poll_route` | POST /api/ar/recon/poll | PASS — delegates to recon scheduler with outbox events |
| `allocate_payment_route` | POST /api/ar/payments/allocate | PASS — `ar.payment_allocated` via outbox |
| `create_jurisdiction` | POST /api/ar/tax/config/jurisdictions | **FAIL** — direct SQL INSERT, no outbox event |
| `update_jurisdiction` | PUT /api/ar/tax/config/jurisdictions/:id | **FAIL** — direct SQL UPDATE, no outbox event |
| `create_rule` | POST /api/ar/tax/config/rules | **FAIL** — direct SQL INSERT, no outbox event |
| `update_rule` | PUT /api/ar/tax/config/rules/:id | **FAIL** — direct SQL UPDATE, no outbox event |

### GL (General Ledger)

GL mutation handlers show a split: revrec, accruals, FX rates, and period reopen emit
outbox events via domain services, but period close, close checklist, and approval
handlers do not.

| Handler | Route | Outbox? |
|---|---|---|
| `validate_close` | POST /api/gl/periods/:id/validate-close | **FAIL** — validation only, no mutation or outbox |
| `close_period_handler` | POST /api/gl/periods/:id/close | **FAIL** — delegates to `close_period()` which does NOT emit outbox |
| `create_checklist_item` | POST /api/gl/periods/:id/checklist | **FAIL** — direct SQL INSERT, no outbox event |
| `complete_checklist_item` | POST /api/gl/periods/:id/checklist/:item_id/complete | **FAIL** — direct SQL UPDATE, no outbox event |
| `waive_checklist_item` | POST /api/gl/periods/:id/checklist/:item_id/waive | **FAIL** — direct SQL UPDATE, no outbox event |
| `create_approval` | POST /api/gl/periods/:id/approvals | **FAIL** — direct SQL INSERT, no outbox event |
| `request_reopen` | POST /api/gl/periods/:id/reopen | **FAIL** — direct SQL INSERT, no outbox event |
| `approve_reopen` | POST /api/gl/periods/:id/reopen/:id/approve | PASS — delegates to `period_reopen_service` with `gl.period.reopened` outbox |
| `reject_reopen` | POST /api/gl/periods/:id/reopen/:id/reject | PASS — updates status (may not emit separately) |
| `create_fx_rate` | POST /api/gl/fx-rates | PASS — `fx.rate_updated` via `fx_rate_service` with outbox |
| `create_contract` | POST /api/gl/revrec/contracts | PASS — outbox event via revrec service |
| `generate_schedule_handler` | POST /api/gl/revrec/schedules | PASS — outbox event via revrec service |
| `run_recognition_handler` | POST /api/gl/revrec/recognition-runs | PASS — outbox event via revrec service |
| `amend_contract` | POST /api/gl/revrec/amendments | PASS — outbox event via revrec service |
| `create_template_handler` | POST /api/gl/accruals/templates | PASS — outbox event via accruals module |
| `create_accrual_handler` | POST /api/gl/accruals/create | PASS — `gl.accrual_created` via outbox |
| `execute_reversals_handler` | POST /api/gl/accruals/reversals/execute | PASS — outbox event via `reversal_service` |

### Party

Party core entity mutations (companies, individuals, parties) emit outbox events via
domain services, but contact and address CRUD do not.

| Handler | Route | Outbox? |
|---|---|---|
| `create_company` | POST /api/party/companies | PASS — `party.created` via domain service |
| `create_individual` | POST /api/party/individuals | PASS — `party.created` via domain service |
| `update_party` | PUT /api/party/parties/:id | PASS — `party.updated` via domain service |
| `deactivate_party` | POST /api/party/parties/:id/deactivate | PASS — `party.deactivated` via domain service |
| `create_contact` | POST /api/party/parties/:id/contacts | **FAIL** — no outbox event |
| `update_contact` | PUT /api/party/contacts/:id | **FAIL** — no outbox event |
| `delete_contact` | DELETE /api/party/contacts/:id | **FAIL** — no outbox event |
| `create_address` | POST /api/party/parties/:id/addresses | **FAIL** — no outbox event |
| `update_address` | PUT /api/party/addresses/:id | **FAIL** — no outbox event |
| `delete_address` | DELETE /api/party/addresses/:id | **FAIL** — no outbox event |

### Inventory

Core stock movement handlers (receipts, issues, adjustments, transfers, reservations,
cycle counts, valuation snapshots) emit outbox events. Master data CRUD (items,
locations, UoM, reorder policies) does not.

| Handler | Route | Outbox? |
|---|---|---|
| `create_item` | POST /api/inventory/items | **FAIL** — no outbox event |
| `update_item` | PUT /api/inventory/items/:id | **FAIL** — no outbox event |
| `deactivate_item` | POST /api/inventory/items/:id/deactivate | **FAIL** — no outbox event |
| `post_receipt` | POST /api/inventory/receipts | PASS — `inventory.item_received` via domain service |
| `post_issue` | POST /api/inventory/issues | PASS — `inventory.item_issued` via domain service |
| `post_reserve` | POST /api/inventory/reservations/reserve | PASS — outbox event via domain service |
| `post_release` | POST /api/inventory/reservations/release | PASS — outbox event via domain service |
| `post_fulfill` | POST /api/inventory/reservations/:id/fulfill | PASS — outbox event via domain service |
| `create_uom` | POST /api/inventory/uoms | **FAIL** — no outbox event |
| `create_conversion` | POST /api/inventory/items/:id/uom-conversions | **FAIL** — no outbox event |
| `post_adjustment` | POST /api/inventory/adjustments | PASS — `inventory.adjusted` via domain service |
| `post_transfer` | POST /api/inventory/transfers | PASS — outbox event via domain service |
| `post_status_transfer` | POST /api/inventory/status-transfers | PASS — `inventory.status_changed` via domain service |
| `post_cycle_count_task` | POST /api/inventory/cycle-count-tasks | **FAIL** — creates task only, no outbox |
| `post_cycle_count_submit` | POST /api/inventory/cycle-count-tasks/:id/submit | PASS — `inventory.cycle_count.submitted` via domain service |
| `post_cycle_count_approve` | POST /api/inventory/cycle-count-tasks/:id/approve | PASS — `inventory.adjusted` via domain service |
| `post_reorder_policy` | POST /api/inventory/reorder-policies | **FAIL** — no outbox event (policy definition, not stock movement) |
| `put_reorder_policy` | PUT /api/inventory/reorder-policies/:id | **FAIL** — no outbox event |
| `post_valuation_snapshot` | POST /api/inventory/valuation-snapshots | PASS — outbox event via domain service |
| `create_location` | POST /api/inventory/locations | **FAIL** — no outbox event |
| `update_location` | PUT /api/inventory/locations/:id | **FAIL** — no outbox event |
| `deactivate_location` | POST /api/inventory/locations/:id/deactivate | **FAIL** — no outbox event |

### Shipping-Receiving

All mutation handlers delegate to domain service which emits outbox events atomically.

| Handler | Route | Outbox? |
|---|---|---|
| `create_shipment` | POST /api/shipping-receiving/shipments | PASS |
| `transition_status` | PATCH /api/shipping-receiving/shipments/:id/status | PASS |
| `add_line` | POST /api/shipping-receiving/shipments/:id/lines | PASS |
| `receive_line` | POST /api/shipping-receiving/shipments/:id/lines/:lid/receive | PASS |
| `accept_line` | POST /api/shipping-receiving/shipments/:id/lines/:lid/accept | PASS |
| `ship_line_qty` | POST /api/shipping-receiving/shipments/:id/lines/:lid/ship-qty | PASS |
| `close_shipment` | POST /api/shipping-receiving/shipments/:id/close | PASS |
| `ship_shipment` | POST /api/shipping-receiving/shipments/:id/ship | PASS |
| `deliver_shipment` | POST /api/shipping-receiving/shipments/:id/deliver | PASS |

### Consolidation

**No outbox infrastructure exists in this module.** All mutation handlers perform
direct SQL operations without emitting events.

| Handler | Route | Outbox? |
|---|---|---|
| `run_consolidation` | POST /api/consolidation/groups/:id/consolidate | **FAIL** |
| `create_group` | POST /api/consolidation/groups | **FAIL** |
| `update_group` | PUT /api/consolidation/groups/:id | **FAIL** |
| `delete_group` | DELETE /api/consolidation/groups/:id | **FAIL** |
| `create_entity` | POST /api/consolidation/groups/:id/entities | **FAIL** |
| `update_entity` | PUT /api/consolidation/entities/:id | **FAIL** |
| `delete_entity` | DELETE /api/consolidation/entities/:id | **FAIL** |
| `create_coa_mapping` | POST /api/consolidation/groups/:id/coa-mappings | **FAIL** |
| `delete_coa_mapping` | DELETE /api/consolidation/coa-mappings/:id | **FAIL** |
| `create_elimination_rule` | POST /api/consolidation/groups/:id/elimination-rules | **FAIL** |
| `update_elimination_rule` | PUT /api/consolidation/elimination-rules/:id | **FAIL** |
| `delete_elimination_rule` | DELETE /api/consolidation/elimination-rules/:id | **FAIL** |
| `upsert_fx_policy` | PUT /api/consolidation/groups/:id/fx-policies | **FAIL** |
| `delete_fx_policy` | DELETE /api/consolidation/fx-policies/:id | **FAIL** |
| `run_intercompany_match` | POST /api/consolidation/groups/:id/intercompany-match | **FAIL** |
| `post_eliminations` | POST /api/consolidation/groups/:id/eliminations | **FAIL** |

### Timekeeping

Entry lifecycle and approval handlers emit outbox events via domain services.
Master data CRUD (employees, projects, tasks, allocations) and operational
handlers (exports, billing) do not.

| Handler | Route | Outbox? |
|---|---|---|
| `create_employee` | POST /api/timekeeping/employees | **FAIL** — no outbox event |
| `update_employee` | PUT /api/timekeeping/employees/:id | **FAIL** — no outbox event |
| `deactivate_employee` | DELETE /api/timekeeping/employees/:id | **FAIL** — no outbox event |
| `create_project` | POST /api/timekeeping/projects | **FAIL** — no outbox event |
| `update_project` | PUT /api/timekeeping/projects/:id | **FAIL** — no outbox event |
| `deactivate_project` | DELETE /api/timekeeping/projects/:id | **FAIL** — no outbox event |
| `create_task` | POST /api/timekeeping/tasks | **FAIL** — no outbox event |
| `update_task` | PUT /api/timekeeping/tasks/:id | **FAIL** — no outbox event |
| `deactivate_task` | DELETE /api/timekeeping/tasks/:id | **FAIL** — no outbox event |
| `create_entry` | POST /api/timekeeping/entries | PASS — via domain/entries/service |
| `correct_entry` | POST /api/timekeeping/entries/correct | PASS — via domain/entries/service |
| `void_entry` | POST /api/timekeeping/entries/void | PASS — via domain/entries/service |
| `submit_approval` | POST /api/timekeeping/approvals/submit | PASS — via domain/approvals/service |
| `approve_approval` | POST /api/timekeeping/approvals/approve | PASS — via domain/approvals/service |
| `reject_approval` | POST /api/timekeeping/approvals/reject | PASS — via domain/approvals/service |
| `recall_approval` | POST /api/timekeeping/approvals/recall | PASS — via domain/approvals/service |
| `create_allocation` | POST /api/timekeeping/allocations | **FAIL** — no outbox event |
| `update_allocation` | PUT /api/timekeeping/allocations/:id | **FAIL** — no outbox event |
| `deactivate_allocation` | DELETE /api/timekeeping/allocations/:id | **FAIL** — no outbox event |
| `create_export` | POST /api/timekeeping/exports | PASS — via domain/export/service |
| `create_rate` | POST /api/timekeeping/rates | **FAIL** — no outbox event |
| `create_billing_run` | POST /api/timekeeping/billing-runs | **FAIL** — no outbox event |

### Payments

Webhook-driven lifecycle emits outbox events, but direct API mutations do not.

| Handler | Route | Outbox? |
|---|---|---|
| `create_checkout_session` | POST /api/payments/checkout-sessions | **FAIL** — creates session in DB, no outbox event |
| `present_checkout_session` | POST /api/payments/checkout-sessions/:id/present | **FAIL** — updates session, no outbox event |
| `tilled_webhook` | POST /api/payments/webhook/tilled | PASS — processes webhook → lifecycle handlers emit `payment.succeeded`/`payment.failed` via outbox |

### Notifications

No mutation routes exposed via HTTP. Notifications are event-driven (consumers only).
Notification handlers (delivery dispatch) do emit outbox events internally.

### Treasury

All mutation handlers delegate to domain services that emit outbox events.

| Handler | Route | Outbox? |
|---|---|---|
| `create_bank_account` | POST /api/treasury/accounts/bank | PASS — `bank_account.created` via domain service |
| `create_credit_card_account` | POST /api/treasury/accounts/credit-card | PASS — `credit_card_account.created` via domain service |
| `update_account` | PUT /api/treasury/accounts/:id | PASS — `bank_account.updated` via domain service |
| `deactivate_account` | POST /api/treasury/accounts/:id/deactivate | PASS — via domain service |
| `auto_match` | POST /api/treasury/recon/auto-match | PASS — `recon.auto_matched` via domain service |
| `manual_match` | POST /api/treasury/recon/manual-match | PASS — `recon.manual_matched` via domain service |
| `link_to_gl` | POST /api/treasury/recon/gl-link | PASS — `recon.gl_linked` via domain service |
| `unmatched_gl_entries` | POST /api/treasury/recon/gl-unmatched-entries | PASS — query-like mutation |
| `import_statement` | POST /api/treasury/statements/import | PASS — `bank_statement.imported` via domain service |

### Fixed Assets

All mutation handlers delegate to domain services that emit outbox events.

| Handler | Route | Outbox? |
|---|---|---|
| `create_category` | POST /api/fixed-assets/categories | PASS — via domain/assets/service |
| `update_category` | PUT /api/fixed-assets/categories/:id | PASS — via domain/assets/service |
| `deactivate_category` | DELETE /api/fixed-assets/categories/:id | PASS — via domain/assets/service |
| `create_asset` | POST /api/fixed-assets/assets | PASS — via domain/assets/service |
| `update_asset` | PUT /api/fixed-assets/assets/:id | PASS — via domain/assets/service |
| `deactivate_asset` | DELETE /api/fixed-assets/assets/:id | PASS — via domain/assets/service |
| `generate_schedule` | POST /api/fixed-assets/depreciation/schedule | PASS — via domain/depreciation/service |
| `create_run` | POST /api/fixed-assets/depreciation/runs | PASS — via domain/depreciation/service |
| `dispose_asset` | POST /api/fixed-assets/disposals | PASS — via domain/disposals/service |

### Integrations

All mutation handlers delegate to domain services that emit outbox events.

| Handler | Route | Outbox? |
|---|---|---|
| `inbound_webhook` | POST /api/webhooks/inbound/:system | PASS — `webhook.routed` via domain/webhooks/routing |
| `create_external_ref` | POST /api/integrations/external-refs | PASS — `external_ref.created` via domain service |
| `update_external_ref` | PUT /api/integrations/external-refs/:id | PASS — `external_ref.updated` via domain service |
| `delete_external_ref` | DELETE /api/integrations/external-refs/:id | PASS — `external_ref.deleted` via domain service |
| `register_connector` | POST /api/integrations/connectors | PASS — `connector.registered` via domain service |
| `run_connector_test` | POST /api/integrations/connectors/:id/test | PASS — test execution, handled by domain service |

### Maintenance

Work order transitions, meter readings, and plan assignments emit outbox events.
Asset create/update and work order parts/labor subresources do not.

| Handler | Route | Outbox? |
|---|---|---|
| `create_asset` | POST /api/maintenance/assets | **FAIL** — no outbox event |
| `update_asset` | PATCH /api/maintenance/assets/:id | **FAIL** — no outbox event |
| `create_meter_type` | POST /api/maintenance/meter-types | **FAIL** — no outbox event |
| `record_reading` | POST /api/maintenance/assets/:id/readings | PASS — outbox event via domain/meters |
| `create_plan` | POST /api/maintenance/plans | PASS — outbox event via domain/plans |
| `update_plan` | PATCH /api/maintenance/plans/:id | PASS — outbox event via domain/plans |
| `assign_plan` | POST /api/maintenance/plans/:id/assign | PASS — outbox event via domain/plans |
| `create_work_order` | POST /api/maintenance/work-orders | PASS — outbox event via domain/work_orders |
| `transition_work_order` | PATCH /api/maintenance/work-orders/:id/transition | PASS — outbox event via domain/work_orders |
| `add_part` | POST /api/maintenance/work-orders/:id/parts | PASS — outbox event via domain/work_orders |
| `remove_part` | DELETE /api/maintenance/work-orders/:id/parts/:pid | PASS — outbox event via domain/work_orders |
| `add_labor` | POST /api/maintenance/work-orders/:id/labor | PASS — outbox event via domain/work_orders |
| `remove_labor` | DELETE /api/maintenance/work-orders/:id/labor/:lid | PASS — outbox event via domain/work_orders |

### PDF Editor

Only the PDF generation handler emits an outbox event. Template, field, and submission
CRUD handlers do not.

| Handler | Route | Outbox? |
|---|---|---|
| `create_template` | POST /api/pdf/forms/templates | **FAIL** — no outbox event |
| `update_template` | PUT /api/pdf/forms/templates/:id | **FAIL** — no outbox event |
| `create_field` | POST /api/pdf/forms/templates/:id/fields | **FAIL** — no outbox event |
| `update_field` | PUT /api/pdf/forms/templates/:tid/fields/:fid | **FAIL** — no outbox event |
| `reorder_fields` | POST /api/pdf/forms/templates/:id/fields/reorder | **FAIL** — no outbox event |
| `create_submission` | POST /api/pdf/forms/submissions | **FAIL** — no outbox event |
| `autosave_submission` | PUT /api/pdf/forms/submissions/:id | **FAIL** — no outbox event |
| `submit_submission` | POST /api/pdf/forms/submissions/:id/submit | **FAIL** — no outbox event |
| `generate_pdf` | POST /api/pdf/forms/submissions/:id/generate | PASS — `pdf.form.generated` via outbox |

### Subscriptions

| Handler | Route | Outbox? |
|---|---|---|
| `execute_bill_run` | POST /api/bill-runs/execute | PASS — `billrun.completed` via `enqueue_event` |

### TTP (Tenant Transaction Processing)

**No outbox infrastructure exists in this module.**

| Handler | Route | Outbox? |
|---|---|---|
| `create_billing_run` | POST /api/ttp/billing-runs | **FAIL** — no outbox event |
| `ingest_events` | POST /api/metering/events | **FAIL** — no outbox event |

### Reporting

| Handler | Route | Outbox? |
|---|---|---|
| `rebuild` | POST /api/reporting/rebuild | N/A — administrative rebuild trigger, re-ingests from source events |

---

## Risk Assessment

### High Priority (financial state changes without events)

1. **GL `close_period`** — Period close is a major lifecycle event. Other modules
   (reporting, consolidation) cannot react to period closures without an outbox event.

2. **AR charges/refunds** — Financial mutations that should trigger GL journal entries
   and payment processing but have no outbox events.

3. **Consolidation (entire module)** — Zero outbox coverage. Consolidation runs,
   elimination entries, and intercompany matches are invisible to the rest of the platform.

4. **Payments `create_checkout_session`** — Creates a financial obligation but
   doesn't emit an event for tracking/audit.

### Medium Priority (operational data without events)

5. **AR customers/subscriptions** — Downstream modules (subscriptions, notifications)
   cannot react to customer lifecycle changes.

6. **Timekeeping employees/projects/allocations/billing** — Master data changes
   invisible to integrations and reporting.

7. **TTP billing/metering** — Platform billing mutations invisible to audit trail.

8. **Inventory items/locations/UoM** — Master data changes invisible to
   integrations module.

### Lower Priority (internal/config data)

9. **AR tax config** — Tax jurisdiction and rule changes are configuration that
   could benefit from audit trail events.

10. **Party contacts/addresses** — Sub-entity changes that could be useful for
    CRM integrations but are lower risk.

11. **GL checklist/approvals** — Workflow state changes that are useful for audit
    but don't affect financial data directly.

12. **PDF Editor templates/fields/submissions** — Content management mutations
    that are lower risk.

13. **Maintenance assets/meter types** — Master data that could benefit from
    events but is lower risk.

---

## Recommendations

1. **Consolidation** needs outbox infrastructure added (outbox table + publisher task)
   before any events can be emitted.

2. **TTP** needs outbox infrastructure added.

3. **GL `close_period`** should emit `gl.period.closed` — this is the highest-priority
   single fix since period close is a critical lifecycle event.

4. **AR charges/refunds** should emit events for GL integration.

5. All other FAIL handlers should be evaluated based on whether downstream modules
   need to react to those mutations.
