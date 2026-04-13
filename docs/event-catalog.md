# Event Catalog — 7D Solutions Platform

> **Generated** by `tools/event-catalog/generate.py` — do not edit manually.
> Run `python3 tools/event-catalog/generate.py` to regenerate.

**258 total subjects** across 25 publishing modules.

## Contents

- [ap](#ap) — 18 subjects
- [ar](#ar) — 29 subjects
- [doc_mgmt](#docmgmt) — 5 subjects
- [fixed-assets](#fixedassets) — 1 subject
- [gl](#gl) — 13 subjects
- [identity](#identity) — 4 subjects
- [integrations](#integrations) — 18 subjects
- [inventory](#inventory) — 27 subjects
- [maintenance](#maintenance) — 17 subjects
- [notifications](#notifications) — 11 subjects
- [numbering](#numbering) — 3 subjects
- [party](#party) — 17 subjects
- [payments](#payments) — 6 subjects
- [pdf-editor](#pdfeditor) — 2 subjects
- [production](#production) — 19 subjects
- [sales](#sales) — 1 subject
- [shipping-receiving](#shippingreceiving) — 8 subjects
- [smoke-test](#smoketest) — 1 subject
- [subscriptions](#subscriptions) — 4 subjects
- [timekeeping](#timekeeping) — 12 subjects
- [treasury](#treasury) — 7 subjects
- [ttp](#ttp) — 4 subjects
- [unknown](#unknown) — 4 subjects
- [workflow](#workflow) — 23 subjects
- [workforce-competence](#workforcecompetence) — 4 subjects

---

## ap

| Subject | Description | Consumers | Payload Fields | Schema |
|---------|-------------|-----------|----------------|--------|
| `ap.events.ap.payment_executed` | — | reporting, treasury | — | — |
| `ap.events.ap.vendor_bill_approved` | — | fixed-assets | — | — |
| `ap.events.ap.vendor_bill_created` | — | reporting | — | — |
| `ap.events.ap.vendor_bill_voided` | — | reporting | — | — |
| `ap.payment_executed` | Event emitted when a single vendor payment is executed as part of a payment run | — | payment_id, run_id, tenant_id, vendor_id, bill_ids, amount_minor, … | [schema](contracts/events/ap-payment-executed.v1.json) |
| `ap.payment_run_created` | Event emitted when a batch of vendor payments is queued for execution | — | run_id, tenant_id, items, total_minor, currency, scheduled_date, … | [schema](contracts/events/ap-payment-run-created.v1.json) |
| `ap.payment_terms_created` | — | — | — | — |
| `ap.po.approved` | — | shipping-receiving | — | — |
| `ap.po_approved` | Event emitted when a purchase order is approved for fulfillment | — | po_id, tenant_id, vendor_id, po_number, approved_amount_minor, currency, … | [schema](contracts/events/ap-po-approved.v1.json) |
| `ap.po_closed` | Event emitted when a purchase order is closed (fully received, cancelled, or manually closed) | — | po_id, tenant_id, vendor_id, po_number, close_reason, closed_by, … | [schema](contracts/events/ap-po-closed.v1.json) |
| `ap.po_created` | Event emitted when a purchase order is created | — | po_id, tenant_id, vendor_id, po_number, currency, lines, … | [schema](contracts/events/ap-po-created.v1.json) |
| `ap.po_line_received_linked` | Event emitted when a PO line is linked to a goods receipt (3-way match anchor) | — | po_id, po_line_id, tenant_id, vendor_id, receipt_id, quantity_received, … | [schema](contracts/events/ap-po-line-received-linked.v1.json) |
| `ap.vendor_bill_approved` | Event emitted when a vendor bill is approved for payment | gl | bill_id, tenant_id, vendor_id, vendor_invoice_ref, approved_amount_minor, currency, … | [schema](contracts/events/ap-vendor-bill-approved.v1.json) |
| `ap.vendor_bill_created` | Event emitted when a vendor invoice/bill is entered into AP | — | bill_id, tenant_id, vendor_id, vendor_invoice_ref, currency, lines, … | [schema](contracts/events/ap-vendor-bill-created.v1.json) |
| `ap.vendor_bill_matched` | Event emitted when a vendor bill is matched to PO lines (2-way or 3-way match) | — | bill_id, tenant_id, vendor_id, po_id, match_type, match_lines, … | [schema](contracts/events/ap-vendor-bill-matched.v1.json) |
| `ap.vendor_bill_voided` | Compensating event emitted when a vendor bill is voided (reversal) | — | bill_id, tenant_id, vendor_id, vendor_invoice_ref, original_total_minor, currency, … | [schema](contracts/events/ap-vendor-bill-voided.v1.json) |
| `ap.vendor_created` | Event emitted when a new vendor is registered in the AP module | — | vendor_id, tenant_id, name, tax_id, currency, payment_terms_days, … | [schema](contracts/events/ap-vendor-created.v1.json) |
| `ap.vendor_updated` | Event emitted when a vendor's attributes are updated | — | vendor_id, tenant_id, name, tax_id, currency, payment_terms_days, … | [schema](contracts/events/ap-vendor-updated.v1.json) |

## ar

| Subject | Description | Consumers | Payload Fields | Schema |
|---------|-------------|-----------|----------------|--------|
| `ar.credit_note_issued` | — | gl | — | — |
| `ar.events.ar.ar_aging_updated` | — | reporting | — | — |
| `ar.events.ar.credit_memo_approved` | — | — | — | — |
| `ar.events.ar.credit_memo_created` | — | — | — | — |
| `ar.events.ar.credit_note_issued` | — | — | — | — |
| `ar.events.ar.dunning_state_changed` | — | — | — | — |
| `ar.events.ar.invoice_attempting` | — | — | — | — |
| `ar.events.ar.invoice_failed_final` | — | — | — | — |
| `ar.events.ar.invoice_opened` | Event emitted when an AR invoice is created and issued | notifications, reporting, vertical-proof | invoice_id, customer_id, amount_due_minor, currency, due_date | [schema](contracts/events/ar-invoice-issued.v1.json) |
| `ar.events.ar.invoice_paid` | — | reporting | — | — |
| `ar.events.ar.invoice_settled_fx` | — | — | — | — |
| `ar.events.ar.invoice_suspended` | — | — | — | — |
| `ar.events.ar.invoice_void` | — | — | — | — |
| `ar.events.ar.invoice_written_off` | — | — | — | — |
| `ar.events.ar.milestone_invoice_created` | — | — | — | — |
| `ar.events.ar.payment.applied` | Event emitted when a payment is applied to an AR invoice | — | invoice_id, payment_id, amount_applied_minor, currency | [schema](contracts/events/ar-payment-applied.v1.json) |
| `ar.events.ar.payment.collection.requested` | Event emitted when AR requests payment collection for an invoice | — | invoice_id, customer_id, amount_minor, currency, payment_method_id | [schema](contracts/events/ar-payment-collection-requested.v1.json) |
| `ar.events.ar.payment_allocated` | — | — | — | — |
| `ar.events.ar.recon_exception_raised` | — | — | — | — |
| `ar.events.ar.recon_match_applied` | — | — | — | — |
| `ar.events.ar.recon_run_started` | — | — | — | — |
| `ar.events.ar.usage_captured` | — | — | — | — |
| `ar.events.ar.usage_invoiced` | — | — | — | — |
| `ar.events.payment.collection.requested` | — | — | — | — |
| `ar.events.tax.committed` | — | — | — | — |
| `ar.events.tax.quoted` | — | — | — | — |
| `ar.events.tax.voided` | — | — | — | — |
| `ar.invoice_settled_fx` | — | gl | — | — |
| `ar.invoice_written_off` | — | gl | — | — |

## doc_mgmt

| Subject | Description | Consumers | Payload Fields | Schema |
|---------|-------------|-----------|----------------|--------|
| `document.created` | Event emitted when a controlled document is created | — | document_id, doc_number, title, doc_type | [schema](contracts/events/doc-mgmt-document-created.v1.json) |
| `document.distribution.requested` | Event emitted when a released document distribution is requested | — | distribution_id, document_id, revision_id, doc_number, recipient_ref, channel, … | [schema](contracts/events/doc-mgmt-document-distribution-requested.v1.json) |
| `document.distribution.status.updated` | Event emitted when document distribution delivery status transitions | — | distribution_id, document_id, status, provider_message_id, failure_reason | [schema](contracts/events/doc-mgmt-document-distribution-status-updated.v1.json) |
| `document.released` | Event emitted when a controlled document is released from draft to official status | — | document_id, doc_number, revision_number | [schema](contracts/events/doc-mgmt-document-released.v1.json) |
| `revision.created` | Event emitted when a new revision is added to a controlled document | — | document_id, revision_id, revision_number | [schema](contracts/events/doc-mgmt-revision-created.v1.json) |

## fixed-assets

| Subject | Description | Consumers | Payload Fields | Schema |
|---------|-------------|-----------|----------------|--------|
| `fa_depreciation_run.depreciation_run_completed` | — | gl | — | — |

## gl

| Subject | Description | Consumers | Payload Fields | Schema |
|---------|-------------|-----------|----------------|--------|
| `fx.rate_updated` | Event emitted when an FX rate is published or updated | — | rate_id, base_currency, quote_currency, rate, inverse_rate, effective_at, … | [schema](contracts/events/gl-fx-rate-updated.v1.json) |
| `gl.accrual_created` | Event emitted when a GL accrual is created and posted | — | accrual_id, template_id, tenant_id, name, period, posting_date, … | [schema](contracts/events/gl-accrual-created.v1.json) |
| `gl.accrual_reversed` | Event emitted when a GL accrual is reversed | — | reversal_id, original_accrual_id, template_id, tenant_id, reversal_period, reversal_date, … | [schema](contracts/events/gl-accrual-reversed.v1.json) |
| `gl.events.entry.reverse.requested` | — | gl | — | — |
| `gl.events.posting.requested` | — | gl, reporting | — | — |
| `gl.export.completed` | — | — | — | — |
| `gl.export.requested` | — | — | — | — |
| `gl.fx_realized_posted` | Event emitted when realized FX gain/loss is posted to GL | — | realized_id, tenant_id, source_transaction_id, source_transaction_type, transaction_currency, reporting_currency, … | [schema](contracts/events/gl-fx-realized-posted.v1.json) |
| `gl.fx_revaluation_posted` | Event emitted when unrealized FX revaluation is posted to GL | — | revaluation_id, tenant_id, period, transaction_currency, reporting_currency, rate_used, … | [schema](contracts/events/gl-fx-revaluation-posted.v1.json) |
| `revrec.contract_created` | — | — | — | — |
| `revrec.contract_modified` | — | — | — | — |
| `revrec.recognition_posted` | — | — | — | — |
| `revrec.schedule_created` | — | — | — | — |

## identity

| Subject | Description | Consumers | Payload Fields | Schema |
|---------|-------------|-----------|----------------|--------|
| `user.lifecycle.access_review_recorded` | Event emitted when an access review decision is recorded for a user | — | user_id, reviewed_by, review_id, decision, notes | [schema](contracts/events/identity-access-review-recorded.v1.json) |
| `user.lifecycle.role_assigned` | Event emitted when a role is assigned to a user | — | user_id, role_id | [schema](contracts/events/identity-role-assigned.v1.json) |
| `user.lifecycle.role_revoked` | Event emitted when a role is revoked from a user | — | user_id, role_id | [schema](contracts/events/identity-role-revoked.v1.json) |
| `user.lifecycle.user_created` | Event emitted when a new user identity is created in the platform | — | user_id, reviewed_by, review_id, decision, notes | [schema](contracts/events/identity-user-created.v1.json) |

## integrations

| Subject | Description | Consumers | Payload Fields | Schema |
|---------|-------------|-----------|----------------|--------|
| `edi_transaction.created` | — | — | — | — |
| `edi_transaction.status_changed` | — | — | — | — |
| `external_ref.created` | Event emitted when an external reference mapping is created or upserted | — | ref_id, app_id, entity_type, entity_id, system, external_id, … | [schema](contracts/events/integrations-external-ref-created.v1.json) |
| `external_ref.deleted` | Event emitted when an external reference mapping is deleted | — | ref_id, app_id, entity_type, entity_id, system, external_id, … | [schema](contracts/events/integrations-external-ref-deleted.v1.json) |
| `external_ref.updated` | Event emitted when an external reference mapping is updated | — | ref_id, app_id, entity_type, entity_id, system, external_id, … | [schema](contracts/events/integrations-external-ref-updated.v1.json) |
| `file_job.created` | — | — | — | — |
| `file_job.status_changed` | — | — | — | — |
| `integrations.order.ingested` | — | — | — | — |
| `integrations.poll.amazon_sp` | — | — | — | — |
| `integrations.poll.ebay` | — | — | — | — |
| `integrations.qbo.invoice_created` | — | — | — | — |
| `integrations.qbo.invoice_sync_failed` | — | — | — | — |
| `outbound_webhook.created` | — | — | — | — |
| `outbound_webhook.deleted` | — | — | — | — |
| `outbound_webhook.updated` | — | — | — | — |
| `qbo.entity.synced` | — | — | — | — |
| `webhook.received` | Event emitted when a raw webhook payload is persisted to the ingest table | — | ingest_id, system, event_type, idempotency_key, received_at | [schema](contracts/events/integrations-webhook-received.v1.json) |
| `webhook.routed` | Event emitted after an inbound webhook has been translated to a domain event | — | ingest_id, system, source_event_type, domain_event_type, outbox_event_id, routed_at | [schema](contracts/events/integrations-webhook-routed.v1.json) |

## inventory

| Subject | Description | Consumers | Payload Fields | Schema |
|---------|-------------|-----------|----------------|--------|
| `inventory.events.inventory.adjusted` | — | — | — | — |
| `inventory.events.inventory.classification_assigned.v1` | — | — | — | — |
| `inventory.events.inventory.cycle_count_approved` | — | — | — | — |
| `inventory.events.inventory.cycle_count_submitted` | — | — | — | — |
| `inventory.events.inventory.expiry_alert.v1` | — | — | — | — |
| `inventory.events.inventory.expiry_set.v1` | — | — | — | — |
| `inventory.events.inventory.item_change_recorded` | — | — | — | — |
| `inventory.events.inventory.item_issued` | — | — | — | — |
| `inventory.events.inventory.item_received` | — | — | — | — |
| `inventory.events.inventory.item_reserved` | — | — | — | — |
| `inventory.events.inventory.item_revision_activated` | — | — | — | — |
| `inventory.events.inventory.item_revision_created` | — | — | — | — |
| `inventory.events.inventory.item_revision_policy_updated` | — | — | — | — |
| `inventory.events.inventory.label_generated.v1` | — | — | — | — |
| `inventory.events.inventory.lot_merged.v1` | — | — | — | — |
| `inventory.events.inventory.lot_split.v1` | — | — | — | — |
| `inventory.events.inventory.low_stock_triggered` | — | — | — | — |
| `inventory.events.inventory.make_buy_changed` | — | — | — | — |
| `inventory.events.inventory.reservation_fulfilled` | — | — | — | — |
| `inventory.events.inventory.reservation_released` | — | — | — | — |
| `inventory.events.inventory.status_changed` | — | — | — | — |
| `inventory.events.inventory.transfer_completed` | — | — | — | — |
| `inventory.events.inventory.valuation_run_completed` | — | — | — | — |
| `inventory.events.inventory.valuation_snapshot` | — | reporting | — | — |
| `inventory.events.inventory.valuation_snapshot_created` | — | — | — | — |
| `inventory.item_issued` | — | gl | — | — |
| `inventory.item_received` | — | ap, gl, quality-inspection | — | — |

## maintenance

| Subject | Description | Consumers | Payload Fields | Schema |
|---------|-------------|-----------|----------------|--------|
| `maintenance.asset.created` | — | — | — | — |
| `maintenance.asset.out_of_service_changed` | — | — | — | — |
| `maintenance.asset.updated` | — | — | — | — |
| `maintenance.calibration.completed` | — | — | — | — |
| `maintenance.calibration.created` | — | — | — | — |
| `maintenance.calibration.event_recorded` | — | — | — | — |
| `maintenance.calibration.status_changed` | — | — | — | — |
| `maintenance.downtime.recorded` | — | — | — | — |
| `maintenance.meter_reading.recorded` | Event emitted when a new meter reading is recorded for an asset | — | asset_id, meter_type_id, reading_value, recorded_at | [schema](contracts/events/maintenance-meter-reading-recorded.v1.json) |
| `maintenance.plan.assigned` | Event emitted when a maintenance plan is assigned to an asset | — | plan_id, asset_id, next_due_date, next_due_meter | [schema](contracts/events/maintenance-plan-assigned.v1.json) |
| `maintenance.plan.due` | Event emitted when a maintenance plan assignment becomes due (calendar or meter threshold reached) | — | assignment_id, plan_id, asset_id, due_kind, due_value | [schema](contracts/events/maintenance-plan-due.v1.json) |
| `maintenance.work_order.cancelled` | Event emitted when a work order is cancelled | — | wo_id, old_status, new_status | [schema](contracts/events/maintenance-work-order-cancelled.v1.json) |
| `maintenance.work_order.closed` | Event emitted when a work order is closed (cost locked, no further edits) | — | wo_id, asset_id | [schema](contracts/events/maintenance-work-order-closed.v1.json) |
| `maintenance.work_order.completed` | Event emitted when a work order is completed. Carries cost data for GL integration. | — | wo_id, asset_id, total_parts_minor, total_labor_minor, currency, downtime_minutes, … | [schema](contracts/events/maintenance-work-order-completed.v1.json) |
| `maintenance.work_order.created` | Event emitted when a work order is created | — | wo_id, wo_number, asset_id, wo_type, priority, plan_assignment_id | [schema](contracts/events/maintenance-work-order-created.v1.json) |
| `maintenance.work_order.overdue` | Event emitted when a work order's scheduled date has passed without completion | — | wo_id, asset_id, days_overdue, priority | [schema](contracts/events/maintenance-work-order-overdue.v1.json) |
| `maintenance.work_order.status_changed` | Event emitted when a work order transitions between statuses | — | wo_id, old_status, new_status | [schema](contracts/events/maintenance-work-order-status-changed.v1.json) |

## notifications

| Subject | Description | Consumers | Payload Fields | Schema |
|---------|-------------|-----------|----------------|--------|
| `notifications.close_calendar.reminder` | — | — | calendar_entry_id, tenant_id, period_id, owner_role, reminder_type, expected_close_date, … | [schema](contracts/events/notifications-close-calendar-reminder.v1.json) |
| `notifications.delivery.failed` | Event emitted when notification delivery fails | — | notification_id, channel, to, template_id, template_key, status, … | [schema](contracts/events/notifications-delivery-failed.v1.json) |
| `notifications.delivery.succeeded` | Event emitted when notification delivery succeeds | — | notification_id, channel, to, template_id, template_key, status, … | [schema](contracts/events/notifications-delivery-succeeded.v1.json) |
| `notifications.dlq.abandoned` | — | — | notification_id, action, previous_status, new_status | [schema](contracts/events/notifications-dlq-abandoned.v1.json) |
| `notifications.dlq.replayed` | — | — | notification_id, action, previous_status, new_status | [schema](contracts/events/notifications-dlq-replayed.v1.json) |
| `notifications.inbox.message_created` | — | — | inbox_message_id, user_id, notification_id, title | [schema](contracts/events/notifications-inbox-message-created.v1.json) |
| `notifications.inbox.message_dismissed` | — | — | inbox_message_id, user_id | [schema](contracts/events/notifications-inbox-message-dismissed.v1.json) |
| `notifications.inbox.message_read` | — | — | inbox_message_id, user_id | [schema](contracts/events/notifications-inbox-message-read.v1.json) |
| `notifications.inbox.message_undismissed` | — | — | inbox_message_id, user_id | [schema](contracts/events/notifications-inbox-message-undismissed.v1.json) |
| `notifications.inbox.message_unread` | — | — | inbox_message_id, user_id | [schema](contracts/events/notifications-inbox-message-unread.v1.json) |
| `notifications.low_stock.alert.created` | — | — | notification_id, channel, to, template_id, status, provider_message_id, … | [schema](contracts/events/notifications-low-stock-alert-created.v1.json) |

## numbering

| Subject | Description | Consumers | Payload Fields | Schema |
|---------|-------------|-----------|----------------|--------|
| `number.allocated` | Event emitted when a sequential number is allocated for a tenant+entity pair | — | tenant_id, entity, number_value, idempotency_key | [schema](contracts/events/numbering-number-allocated.v1.json) |
| `number.confirmed` | Event emitted when a gap-free reserved number is confirmed by the caller | — | tenant_id, entity, number_value, idempotency_key | [schema](contracts/events/numbering-number-confirmed.v1.json) |
| `policy.updated` | Event emitted when a numbering policy is created or updated for a tenant+entity pair | — | tenant_id, entity, pattern, prefix, padding, version | [schema](contracts/events/numbering-policy-updated.v1.json) |

## party

| Subject | Description | Consumers | Payload Fields | Schema |
|---------|-------------|-----------|----------------|--------|
| `party.contact_role.created` | — | — | — | — |
| `party.contact_role.updated` | — | — | — | — |
| `party.created` | Event emitted when a new party (company or individual) is created | — | party_id, app_id, party_type, display_name, email, created_at | [schema](contracts/events/party-created.v1.json) |
| `party.credit_terms.created` | — | — | — | — |
| `party.credit_terms.updated` | — | — | — | — |
| `party.deactivated` | Event emitted when a party is deactivated (soft deleted) | — | party_id, app_id, deactivated_by, deactivated_at | [schema](contracts/events/party-deactivated.v1.json) |
| `party.events.contact.created` | — | — | — | — |
| `party.events.contact.deactivated` | — | — | — | — |
| `party.events.contact.primary_set` | — | — | — | — |
| `party.events.contact.updated` | — | — | — | — |
| `party.events.tags.updated` | — | — | — | — |
| `party.reactivated` | — | — | — | — |
| `party.scorecard.created` | — | — | — | — |
| `party.scorecard.updated` | — | — | — | — |
| `party.updated` | Event emitted when a party's base fields are updated | — | party_id, app_id, display_name, email, updated_by, updated_at | [schema](contracts/events/party-updated.v1.json) |
| `party.vendor_qualification.created` | — | — | — | — |
| `party.vendor_qualification.updated` | — | — | — | — |

## payments

| Subject | Description | Consumers | Payload Fields | Schema |
|---------|-------------|-----------|----------------|--------|
| `payments.events.payment.failed` | — | notifications | — | — |
| `payments.events.payment.succeeded` | — | ar, notifications, reporting, treasury | — | — |
| `payments.events.payments.payment.failed` | Event emitted when a payment fails | — | payment_id, invoice_id, ar_customer_id, amount_minor, currency, failure_code, … | [schema](contracts/events/payments-payment-failed.v1.json) |
| `payments.events.payments.payment.succeeded` | Event emitted when a payment succeeds | — | payment_id, invoice_id, ar_customer_id, amount_minor, currency, processor_payment_id, … | [schema](contracts/events/payments-payment-succeeded.v1.json) |
| `payments.events.payments.refund.failed` | Event emitted when a refund fails | — | refund_id, payment_id, invoice_id, ar_customer_id, amount_minor, currency, … | [schema](contracts/events/payments-refund-failed.v1.json) |
| `payments.events.payments.refund.succeeded` | Event emitted when a refund succeeds | — | refund_id, payment_id, invoice_id, ar_customer_id, amount_minor, currency, … | [schema](contracts/events/payments-refund-succeeded.v1.json) |

## pdf-editor

| Subject | Description | Consumers | Payload Fields | Schema |
|---------|-------------|-----------|----------------|--------|
| `pdf.form.generated` | Event emitted when a filled PDF is generated from a submitted form | — | tenant_id, submission_id, template_id | [schema](contracts/events/pdf-editor-form-generated.v1.json) |
| `pdf.form.submitted` | Event emitted when a form submission is validated and finalized | — | tenant_id, submission_id, template_id, submitted_by | [schema](contracts/events/pdf-editor-form-submitted.v1.json) |

## production

| Subject | Description | Consumers | Payload Fields | Schema |
|---------|-------------|-----------|----------------|--------|
| `production.component_issue.requested` | — | inventory | — | — |
| `production.component_issued` | — | — | — | — |
| `production.downtime.ended` | — | maintenance | — | — |
| `production.downtime.started` | — | maintenance | — | — |
| `production.fg_receipt.requested` | — | inventory, quality-inspection | — | — |
| `production.fg_received` | — | — | — | — |
| `production.operation_completed` | — | quality-inspection | — | — |
| `production.operation_started` | — | — | — | — |
| `production.routing_created` | — | — | — | — |
| `production.routing_released` | — | — | — | — |
| `production.routing_updated` | — | — | — | — |
| `production.time_entry_created` | — | — | — | — |
| `production.time_entry_stopped` | — | — | — | — |
| `production.work_order_closed` | — | — | — | — |
| `production.work_order_created` | — | — | — | — |
| `production.work_order_released` | — | — | — | — |
| `production.workcenter_created` | — | — | — | — |
| `production.workcenter_deactivated` | — | — | — | — |
| `production.workcenter_updated` | — | — | — | — |

## sales

| Subject | Description | Consumers | Payload Fields | Schema |
|---------|-------------|-----------|----------------|--------|
| `sales.so.released` | — | shipping-receiving | — | — |

## shipping-receiving

| Subject | Description | Consumers | Payload Fields | Schema |
|---------|-------------|-----------|----------------|--------|
| `shipping_receiving.inbound_closed` | — | — | — | — |
| `shipping_receiving.outbound_delivered` | — | — | — | — |
| `shipping_receiving.outbound_shipped` | — | — | — | — |
| `shipping_receiving.shipment_created` | — | — | — | — |
| `shipping_receiving.shipment_status_changed` | — | — | — | — |
| `sr.carrier_request.created` | — | — | — | — |
| `sr.receipt_routed_to_inspection.v1` | — | — | — | — |
| `sr.receipt_routed_to_stock.v1` | — | — | — | — |

## smoke-test

| Subject | Description | Consumers | Payload Fields | Schema |
|---------|-------------|-----------|----------------|--------|
| `smoke_test.item_created` | — | smoke-test | — | — |

## subscriptions

| Subject | Description | Consumers | Payload Fields | Schema |
|---------|-------------|-----------|----------------|--------|
| `subscriptions.events.subscriptions.billrun.executed` | Event emitted when a billing cycle (bill run) is executed | — | bill_run_id, execution_date, subscriptions_processed, invoices_created, failures, execution_time, … | [schema](contracts/events/subscriptions-billrun-executed.v1.json) |
| `subscriptions.events.subscriptions.created` | Event emitted when a new subscription is created | — | subscription_id, ar_customer_id, plan_id, schedule, price_minor, currency, … | [schema](contracts/events/subscriptions-created.v1.json) |
| `subscriptions.events.subscriptions.paused` | Event emitted when a subscription is paused | — | subscription_id, ar_customer_id, paused_at, status, previous_status, reason | [schema](contracts/events/subscriptions-paused.v1.json) |
| `subscriptions.events.subscriptions.resumed` | Event emitted when a paused subscription is resumed | — | subscription_id, ar_customer_id, resumed_at, next_bill_date, status, previous_status | [schema](contracts/events/subscriptions-resumed.v1.json) |

## timekeeping

| Subject | Description | Consumers | Payload Fields | Schema |
|---------|-------------|-----------|----------------|--------|
| `clock_session.clocked_in` | — | — | — | — |
| `clock_session.clocked_out` | — | — | — | — |
| `export_run.completed` | — | — | — | — |
| `timekeeping.billable_time` | — | — | — | — |
| `timekeeping.labor_cost` | — | gl | — | — |
| `timesheet.approved` | Emitted when a reviewer approves a submitted timesheet. Approval locks the period, preventing further entry modifications. | — | approval_id, app_id, employee_id, period_start, period_end, reviewer_id, … | [schema](contracts/events/timekeeping-timesheet-approved.v1.json) |
| `timesheet.recalled` | Emitted when an employee recalls a submitted timesheet before it is reviewed. Transitions status from submitted back to draft. | — | approval_id, app_id, employee_id, period_start, period_end | [schema](contracts/events/timekeeping-timesheet-recalled.v1.json) |
| `timesheet.rejected` | Emitted when a reviewer rejects a submitted timesheet. Employee can correct entries and resubmit. | — | approval_id, app_id, employee_id, period_start, period_end, reviewer_id, … | [schema](contracts/events/timekeeping-timesheet-rejected.v1.json) |
| `timesheet.submitted` | Emitted when an employee submits a timesheet for a period. Transitions approval status from draft/rejected to submitted. | — | approval_id, app_id, employee_id, period_start, period_end, total_minutes | [schema](contracts/events/timekeeping-timesheet-submitted.v1.json) |
| `timesheet_entry.corrected` | Emitted when a timesheet entry is corrected. Append-only: the original row is marked is_current=FALSE and a new correction row is inserted with incremented version. | — | entry_id, app_id, employee_id, work_date, old_minutes, new_minutes, … | [schema](contracts/events/timekeeping-entry-corrected.v1.json) |
| `timesheet_entry.created` | Emitted when a new timesheet entry is created. Follows Guard-Mutation-Outbox atomicity. The entry_id identifies the logical entry; version starts at 1. | — | entry_id, app_id, employee_id, work_date, minutes, version | [schema](contracts/events/timekeeping-entry-created.v1.json) |
| `timesheet_entry.voided` | Emitted when a timesheet entry is voided (cancelled). A void row with minutes=0 is appended and the previous version is marked is_current=FALSE. | — | entry_id, app_id, employee_id, work_date, voided_minutes, version | [schema](contracts/events/timekeeping-entry-voided.v1.json) |

## treasury

| Subject | Description | Consumers | Payload Fields | Schema |
|---------|-------------|-----------|----------------|--------|
| `treasury.events.bank_account.created` | — | — | — | — |
| `treasury.events.bank_account.deactivated` | — | — | — | — |
| `treasury.events.bank_account.updated` | — | — | — | — |
| `treasury.events.bank_statement.imported` | — | — | — | — |
| `treasury.events.recon.auto_matched` | — | — | — | — |
| `treasury.events.recon.gl_linked` | — | — | — | — |
| `treasury.events.recon.manual_matched` | — | — | — | — |

## ttp

| Subject | Description | Consumers | Payload Fields | Schema |
|---------|-------------|-----------|----------------|--------|
| `ttp.billing_run.completed` | — | — | — | — |
| `ttp.billing_run.created` | — | — | — | — |
| `ttp.billing_run.failed` | — | — | — | — |
| `ttp.party.invoiced` | — | — | — | — |

## unknown

| Subject | Description | Consumers | Payload Fields | Schema |
|---------|-------------|-----------|----------------|--------|
| `docmgmt.attachment.created` | — | ap | — | — |
| `tax.committed` | — | gl | — | — |
| `tax.voided` | — | gl | — | — |
| `test.ingest.bus.events` | — | reporting | — | — |

## workflow

| Subject | Description | Consumers | Payload Fields | Schema |
|---------|-------------|-----------|----------------|--------|
| `definition.created` | Event emitted when a new workflow definition is registered | — | definition_id, tenant_id, name, version, initial_step_id, step_count | [schema](contracts/events/workflow-definition-created.v1.json) |
| `delegation.created` | Event emitted when an actor delegates their workflow authority to another actor | — | delegation_id, tenant_id, delegator_id, delegatee_id, definition_id, entity_type, … | [schema](contracts/events/workflow-delegation-created.v1.json) |
| `delegation.revoked` | Event emitted when a delegation of workflow authority is revoked | — | delegation_id, tenant_id, delegator_id, delegatee_id, revoked_by, revoke_reason | [schema](contracts/events/workflow-delegation-revoked.v1.json) |
| `escalation.fired` | Event emitted when an escalation timer fires due to step timeout | — | timer_id, instance_id, tenant_id, rule_id, step_id, escalation_count, … | [schema](contracts/events/workflow-escalation-fired.v1.json) |
| `hold.applied` | Event emitted when a hold is applied to an entity | — | hold_id, tenant_id, entity_type, entity_id, hold_type, reason, … | [schema](contracts/events/workflow-hold-applied.v1.json) |
| `hold.released` | Event emitted when a hold is released from an entity | — | hold_id, tenant_id, entity_type, entity_id, hold_type, released_by, … | [schema](contracts/events/workflow-hold-released.v1.json) |
| `instance.advanced` | Event emitted when a workflow instance advances to a new step | — | instance_id, tenant_id, transition_id, from_step_id, to_step_id, action | [schema](contracts/events/workflow-instance-advanced.v1.json) |
| `instance.cancelled` | Event emitted when a workflow instance is cancelled before completion | — | instance_id, tenant_id, step_at_cancellation | [schema](contracts/events/workflow-instance-cancelled.v1.json) |
| `instance.completed` | Event emitted when a workflow instance reaches its terminal completed state | — | instance_id, tenant_id, final_step_id | [schema](contracts/events/workflow-instance-completed.v1.json) |
| `instance.started` | Event emitted when a new workflow instance is started | — | instance_id, tenant_id, definition_id, entity_type, entity_id, initial_step_id | [schema](contracts/events/workflow-instance-started.v1.json) |
| `step.decision_recorded` | Event emitted when an actor records a decision at a parallel routing step | — | instance_id, tenant_id, step_id, actor_id, decision, current_count, … | [schema](contracts/events/workflow-step-decision-recorded.v1.json) |
| `step.parallel_threshold_met` | Event emitted when a parallel step reaches its required decision threshold and auto-advances | — | instance_id, tenant_id, step_id, decision_count, threshold, target_step | [schema](contracts/events/workflow-step-parallel-threshold-met.v1.json) |
| `workflow.events.definition.created` | — | — | — | — |
| `workflow.events.delegation.created` | — | — | — | — |
| `workflow.events.delegation.revoked` | — | — | — | — |
| `workflow.events.escalation.fired` | — | — | — | — |
| `workflow.events.hold.applied` | — | — | — | — |
| `workflow.events.hold.released` | — | — | — | — |
| `workflow.events.instance.advanced` | — | — | — | — |
| `workflow.events.instance.cancelled` | — | — | — | — |
| `workflow.events.instance.completed` | — | — | — | — |
| `workflow.events.instance.started` | — | — | — | — |
| `workflow.events.step.decision_recorded` | — | — | — | — |

## workforce-competence

| Subject | Description | Consumers | Payload Fields | Schema |
|---------|-------------|-----------|----------------|--------|
| `workforce_competence.acceptance_authority_granted` | — | — | — | — |
| `workforce_competence.acceptance_authority_revoked` | — | — | — | — |
| `workforce_competence.artifact_registered` | — | — | — | — |
| `workforce_competence.competence_assigned` | — | — | — | — |

---

## How to add a new event

1. Implement the outbox entry in your module (Guard → Mutation → Outbox pattern).
2. Add a JSON Schema contract to `contracts/events/<module>-<event>.v1.json`.
3. Run `python3 tools/event-catalog/generate.py` to regenerate the catalog.
4. Commit both the contract file and the updated catalog.

CI will fail if the catalog is out of date with the committed contracts.
