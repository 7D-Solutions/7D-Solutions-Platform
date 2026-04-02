# Event Catalog

> **Generated from source code.** Do not edit manually — regenerate with:
> ```bash
> ./scripts/generate-event-catalog.sh
> ```

This catalog lists every event published across the platform, organized by
source module. Each entry shows the event type, the NATS subject it publishes
to, known consumers, and the source file containing the payload definition.

## ap

| Event Type | NATS Subject | Consumers | Source |
|-----------|-------------|-----------|--------|
| `ap.payment_executed` | `ap.events.ap.payment_executed` | treasury | `modules/ap/src/events/payment.rs` |
| `ap.payment_run_created` | `ap.events.ap.payment_run_created` | — | `modules/ap/src/events/payment.rs` |
| `ap.payment_terms_created` | `ap.events.ap.payment_terms_created` | — | `modules/ap/src/events/payment_terms.rs` |
| `ap.po_approved` | `ap.events.ap.po_approved` | shipping-receiving | `modules/ap/src/events/po.rs` |
| `ap.po_closed` | `ap.events.ap.po_closed` | — | `modules/ap/src/events/po.rs` |
| `ap.po_created` | `ap.events.ap.po_created` | — | `modules/ap/src/events/po.rs` |
| `ap.po_line_received_linked` | `ap.events.ap.po_line_received_linked` | — | `modules/ap/src/events/po.rs` |
| `ap.vendor_bill_approved` | `ap.events.ap.vendor_bill_approved` | fixed-assets | `modules/ap/src/events/bill.rs` |
| `ap.vendor_bill_created` | `ap.events.ap.vendor_bill_created` | — | `modules/ap/src/events/bill.rs` |
| `ap.vendor_bill_matched` | `ap.events.ap.vendor_bill_matched` | — | `modules/ap/src/events/bill.rs` |
| `ap.vendor_bill_voided` | `ap.events.ap.vendor_bill_voided` | — | `modules/ap/src/events/bill.rs` |
| `ap.vendor_created` | `ap.events.ap.vendor_created` | — | `modules/ap/src/events/vendor.rs` |
| `ap.vendor_updated` | `ap.events.ap.vendor_updated` | — | `modules/ap/src/events/vendor.rs` |

## ar

| Event Type | NATS Subject | Consumers | Source |
|-----------|-------------|-----------|--------|
| `ar.ar_aging_updated` | `ar.events.ar.ar_aging_updated` | — | `modules/ar/src/events/contracts/aging_dunning.rs` |
| `ar.credit_memo_approved` | `ar.events.ar.credit_memo_approved` | — | `modules/ar/src/events/contracts/credit_writeoff.rs` |
| `ar.credit_memo_created` | `ar.events.ar.credit_memo_created` | — | `modules/ar/src/events/contracts/credit_writeoff.rs` |
| `ar.credit_note_issued` | `ar.events.ar.credit_note_issued` | gl | `modules/ar/src/events/contracts/credit_writeoff.rs` |
| `ar.dunning_state_changed` | `ar.events.ar.dunning_state_changed` | — | `modules/ar/src/events/contracts/aging_dunning.rs` |
| `ar.invoice_opened` | `ar.events.ar.invoice_opened` | notifications | `modules/ar/src/events/contracts/invoice_lifecycle.rs` |
| `ar.invoice_paid` | `ar.events.ar.invoice_paid` | — | `modules/ar/src/events/contracts/invoice_lifecycle.rs` |
| `ar.invoice_settled_fx` | `ar.events.ar.invoice_settled_fx` | gl | `modules/ar/src/events/contracts/tax_fx.rs` |
| `ar.invoice_suspended` | `ar.events.ar.invoice_suspended` | subscriptions | `modules/ar/src/events/contracts/aging_dunning.rs` |
| `ar.invoice_written_off` | `ar.events.ar.invoice_written_off` | gl | `modules/ar/src/events/contracts/credit_writeoff.rs` |
| `ar.milestone_invoice_created` | `ar.events.ar.milestone_invoice_created` | — | `modules/ar/src/events/contracts/progress_billing.rs` |
| `ar.payment_allocated` | `ar.events.ar.payment_allocated` | — | `modules/ar/src/events/contracts/recon_allocation.rs` |
| `ar.recon_exception_raised` | `ar.events.ar.recon_exception_raised` | — | `modules/ar/src/events/contracts/recon_allocation.rs` |
| `ar.recon_match_applied` | `ar.events.ar.recon_match_applied` | — | `modules/ar/src/events/contracts/recon_allocation.rs` |
| `ar.recon_run_started` | `ar.events.ar.recon_run_started` | — | `modules/ar/src/events/contracts/recon_allocation.rs` |
| `ar.usage_captured` | `ar.events.ar.usage_captured` | — | `modules/ar/src/events/contracts/usage.rs` |
| `ar.usage_invoiced` | `ar.events.ar.usage_invoiced` | — | `modules/ar/src/events/contracts/usage.rs` |
| `gl.posting.requested` | `gl.events.posting.requested` | gl | `modules/ar/src/domain/invoices/service.rs` |
| `payment.collection.requested` | `ar.events.payment.collection.requested` | payments | `modules/ar/src/domain/invoices/service.rs` |
| `tax.committed` | `ar.events.tax.committed` | gl | `modules/ar/src/events/contracts/tax_fx.rs` |
| `tax.quoted` | `ar.events.tax.quoted` | — | `modules/ar/src/events/contracts/tax_fx.rs` |
| `tax.voided` | `ar.events.tax.voided` | gl | `modules/ar/src/events/contracts/tax_fx.rs` |

## bom

| Event Type | NATS Subject | Consumers | Source |
|-----------|-------------|-----------|--------|
| `bom.created` | `bom.events.bom.created` | — | `modules/bom/src/events/mod.rs` |
| `bom.effectivity_set` | `bom.events.bom.effectivity_set` | — | `modules/bom/src/events/mod.rs` |
| `bom.line_added` | `bom.events.bom.line_added` | — | `modules/bom/src/events/mod.rs` |
| `bom.line_removed` | `bom.events.bom.line_removed` | — | `modules/bom/src/events/mod.rs` |
| `bom.line_updated` | `bom.events.bom.line_updated` | — | `modules/bom/src/events/mod.rs` |
| `bom.revision_created` | `bom.events.bom.revision_created` | — | `modules/bom/src/events/mod.rs` |
| `bom.revision_released` | `bom.events.bom.revision_released` | — | `modules/bom/src/events/mod.rs` |
| `bom.revision_superseded` | `bom.events.bom.revision_superseded` | — | `modules/bom/src/events/mod.rs` |
| `eco.applied` | `bom.events.eco.applied` | — | `modules/bom/src/events/mod.rs` |
| `eco.approved` | `bom.events.eco.approved` | — | `modules/bom/src/events/mod.rs` |
| `eco.created` | `bom.events.eco.created` | — | `modules/bom/src/events/mod.rs` |
| `eco.rejected` | `bom.events.eco.rejected` | — | `modules/bom/src/events/mod.rs` |
| `eco.submitted` | `bom.events.eco.submitted` | — | `modules/bom/src/events/mod.rs` |

## gl

| Event Type | NATS Subject | Consumers | Source |
|-----------|-------------|-----------|--------|
| `fx.rate_updated` | `gl.events.fx.rate_updated` | — | `modules/gl/src/events/contracts/fx.rs` |
| `gl.accrual_created` | `gl.events.gl.accrual_created` | — | `modules/gl/src/events/contracts/accruals.rs` |
| `gl.accrual_reversed` | `gl.events.gl.accrual_reversed` | — | `modules/gl/src/events/contracts/accruals.rs` |
| `gl.export.completed` | `gl.events.gl.export.completed` | — | `modules/gl/src/exports/service.rs` |
| `gl.export.requested` | `gl.events.gl.export.requested` | — | `modules/gl/src/exports/service.rs` |
| `gl.fx_realized_posted` | `gl.events.gl.fx_realized_posted` | — | `modules/gl/src/events/contracts/fx.rs` |
| `gl.fx_revaluation_posted` | `gl.events.gl.fx_revaluation_posted` | — | `modules/gl/src/events/contracts/fx.rs` |
| `revrec.contract_created` | `gl.events.revrec.contract_created` | — | `modules/gl/src/revrec/contracts/mod.rs` |
| `revrec.contract_modified` | `gl.events.revrec.contract_modified` | — | `modules/gl/src/revrec/contracts/mod.rs` |
| `revrec.recognition_posted` | `gl.events.revrec.recognition_posted` | — | `modules/gl/src/revrec/contracts/mod.rs` |
| `revrec.schedule_created` | `gl.events.revrec.schedule_created` | — | `modules/gl/src/revrec/contracts/mod.rs` |

## integrations

| Event Type | NATS Subject | Consumers | Source |
|-----------|-------------|-----------|--------|
| `edi_transaction.created` | `integrations.events.edi_transaction.created` | — | `modules/integrations/src/events/edi_transaction_created.rs` |
| `edi_transaction.status_changed` | `integrations.events.edi_transaction.status_changed` | — | `modules/integrations/src/events/edi_transaction_status_changed.rs` |
| `external_ref.created` | `integrations.events.external_ref.created` | — | `modules/integrations/src/events/external_ref_created.rs` |
| `external_ref.deleted` | `integrations.events.external_ref.deleted` | — | `modules/integrations/src/events/external_ref_deleted.rs` |
| `external_ref.updated` | `integrations.events.external_ref.updated` | — | `modules/integrations/src/events/external_ref_updated.rs` |
| `file_job.created` | `integrations.events.file_job.created` | — | `modules/integrations/src/events/file_job_created.rs` |
| `file_job.status_changed` | `integrations.events.file_job.status_changed` | — | `modules/integrations/src/events/file_job_status_changed.rs` |
| `outbound_webhook.created` | `integrations.events.outbound_webhook.created` | — | `modules/integrations/src/events/outbound_webhook_created.rs` |
| `outbound_webhook.deleted` | `integrations.events.outbound_webhook.deleted` | — | `modules/integrations/src/events/outbound_webhook_deleted.rs` |
| `outbound_webhook.updated` | `integrations.events.outbound_webhook.updated` | — | `modules/integrations/src/events/outbound_webhook_updated.rs` |
| `qbo.entity.synced` | `integrations.events.qbo.entity.synced` | — | `modules/integrations/src/domain/qbo/cdc.rs` |
| `webhook.received` | `integrations.events.webhook.received` | — | `modules/integrations/src/events/webhook_received.rs` |
| `webhook.routed` | `integrations.events.webhook.routed` | — | `modules/integrations/src/events/webhook_routed.rs` |

## inventory

| Event Type | NATS Subject | Consumers | Source |
|-----------|-------------|-----------|--------|
| `inventory.adjusted` | `inventory.events.inventory.adjusted` | — | `modules/inventory/src/events/contracts.rs` |
| `inventory.classification_assigned.v1` | `inventory.events.inventory.classification_assigned.v1` | — | `modules/inventory/src/events/classification_assigned.rs` |
| `inventory.cycle_count_approved` | `inventory.events.inventory.cycle_count_approved` | — | `modules/inventory/src/events/cycle_count_approved.rs` |
| `inventory.cycle_count_submitted` | `inventory.events.inventory.cycle_count_submitted` | — | `modules/inventory/src/events/cycle_count_submitted.rs` |
| `inventory.expiry_alert.v1` | `inventory.events.inventory.expiry_alert.v1` | — | `modules/inventory/src/events/expiry_alert.rs` |
| `inventory.expiry_set.v1` | `inventory.events.inventory.expiry_set.v1` | — | `modules/inventory/src/events/expiry_set.rs` |
| `inventory.item_change_recorded` | `inventory.events.inventory.item_change_recorded` | — | `modules/inventory/src/events/item_change_recorded.rs` |
| `inventory.item_issued` | `inventory.events.inventory.item_issued` | gl | `modules/inventory/src/events/contracts.rs` |
| `inventory.item_received` | `inventory.events.inventory.item_received` | ap, gl, quality-inspection | `modules/inventory/src/events/contracts.rs` |
| `inventory.item_revision_activated` | `inventory.events.inventory.item_revision_activated` | — | `modules/inventory/src/events/revision_activated.rs` |
| `inventory.item_revision_created` | `inventory.events.inventory.item_revision_created` | — | `modules/inventory/src/events/revision_created.rs` |
| `inventory.item_revision_policy_updated` | `inventory.events.inventory.item_revision_policy_updated` | — | `modules/inventory/src/events/revision_policy_updated.rs` |
| `inventory.label_generated.v1` | `inventory.events.inventory.label_generated.v1` | — | `modules/inventory/src/events/label_generated.rs` |
| `inventory.lot_merged.v1` | `inventory.events.inventory.lot_merged.v1` | — | `modules/inventory/src/events/lot_merged.rs` |
| `inventory.lot_split.v1` | `inventory.events.inventory.lot_split.v1` | — | `modules/inventory/src/events/lot_split.rs` |
| `inventory.low_stock_triggered` | `inventory.events.inventory.low_stock_triggered` | — | `modules/inventory/src/events/low_stock_triggered.rs` |
| `inventory.make_buy_changed` | `inventory.events.inventory.make_buy_changed` | — | `modules/inventory/src/events/make_buy_changed.rs` |
| `inventory.status_changed` | `inventory.events.inventory.status_changed` | — | `modules/inventory/src/events/status_changed.rs` |
| `inventory.transfer_completed` | `inventory.events.inventory.transfer_completed` | — | `modules/inventory/src/events/contracts.rs` |
| `inventory.valuation_run_completed` | `inventory.events.inventory.valuation_run_completed` | — | `modules/inventory/src/events/valuation_run_completed.rs` |
| `inventory.valuation_snapshot_created` | `inventory.events.inventory.valuation_snapshot_created` | — | `modules/inventory/src/events/valuation_snapshot_created.rs` |

## notifications

| Event Type | NATS Subject | Consumers | Source |
|-----------|-------------|-----------|--------|
| `notifications.delivery.succeeded` | `notifications.events.notifications.delivery.succeeded` | — | `modules/notifications/src/handlers.rs` |
| `notifications.escalation.fired` | `notifications.events.notifications.escalation.fired` | — | `modules/notifications/src/escalation/repo.rs` |
| `notifications.events.broadcast.created` | `notifications.events.notifications.events.broadcast.created` | — | `modules/notifications/src/broadcast/repo.rs` |
| `notifications.events.broadcast.delivered` | `notifications.events.notifications.events.broadcast.delivered` | — | `modules/notifications/src/broadcast/repo.rs` |
| `notifications.events.delivery.attempted` | `notifications.events.notifications.events.delivery.attempted` | — | `modules/notifications/src/http/sends.rs` |
| `notifications.events.delivery.succeeded` | `notifications.events.notifications.events.delivery.succeeded` | — | `modules/notifications/src/http/sends.rs` |
| `notifications.events.dlq.abandoned` | `notifications.events.notifications.events.dlq.abandoned` | — | `modules/notifications/src/http/dlq.rs` |
| `notifications.events.dlq.replayed` | `notifications.events.notifications.events.dlq.replayed` | — | `modules/notifications/src/bin/dlq_replay_drill.rs` |
| `notifications.events.inbox.message_created` | `notifications.events.notifications.events.inbox.message_created` | — | `modules/notifications/src/inbox/repo.rs` |
| `notifications.events.inbox.message_dismissed` | `notifications.events.notifications.events.inbox.message_dismissed` | — | `modules/notifications/src/inbox/repo.rs` |
| `notifications.events.inbox.message_read` | `notifications.events.notifications.events.inbox.message_read` | — | `modules/notifications/src/inbox/repo.rs` |
| `notifications.events.inbox.message_undismissed` | `notifications.events.notifications.events.inbox.message_undismissed` | — | `modules/notifications/src/inbox/repo.rs` |
| `notifications.events.inbox.message_unread` | `notifications.events.notifications.events.inbox.message_unread` | — | `modules/notifications/src/inbox/repo.rs` |
| `notifications.events.template.published` | `notifications.events.notifications.events.template.published` | — | `modules/notifications/src/http/templates.rs` |
| `notifications.low_stock.alert.created` | `notifications.events.notifications.low_stock.alert.created` | — | `modules/notifications/src/handlers.rs` |

## party

| Event Type | NATS Subject | Consumers | Source |
|-----------|-------------|-----------|--------|
| `party.contact_role.created` | `party.events.party.contact_role.created` | — | `modules/party/src/events/vendor.rs` |
| `party.contact_role.updated` | `party.events.party.contact_role.updated` | — | `modules/party/src/events/vendor.rs` |
| `party.created` | `party.events.party.created` | — | `modules/party/src/events/party.rs` |
| `party.credit_terms.created` | `party.events.party.credit_terms.created` | — | `modules/party/src/events/vendor.rs` |
| `party.credit_terms.updated` | `party.events.party.credit_terms.updated` | — | `modules/party/src/events/vendor.rs` |
| `party.deactivated` | `party.events.party.deactivated` | — | `modules/party/src/events/party.rs` |
| `party.events.contact.created` | `party.events.party.events.contact.created` | — | `modules/party/src/events/contact.rs` |
| `party.events.contact.deactivated` | `party.events.party.events.contact.deactivated` | — | `modules/party/src/events/contact.rs` |
| `party.events.contact.primary_set` | `party.events.party.events.contact.primary_set` | — | `modules/party/src/events/contact.rs` |
| `party.events.contact.updated` | `party.events.party.events.contact.updated` | — | `modules/party/src/events/contact.rs` |
| `party.events.tags.updated` | `party.events.party.events.tags.updated` | — | `modules/party/src/events/contact.rs` |
| `party.scorecard.created` | `party.events.party.scorecard.created` | — | `modules/party/src/events/vendor.rs` |
| `party.scorecard.updated` | `party.events.party.scorecard.updated` | — | `modules/party/src/events/vendor.rs` |
| `party.updated` | `party.events.party.updated` | — | `modules/party/src/events/party.rs` |
| `party.vendor_qualification.created` | `party.events.party.vendor_qualification.created` | — | `modules/party/src/events/vendor.rs` |
| `party.vendor_qualification.updated` | `party.events.party.vendor_qualification.updated` | — | `modules/party/src/events/vendor.rs` |

## payments

| Event Type | NATS Subject | Consumers | Source |
|-----------|-------------|-----------|--------|
| `payment.failed` | `payments.events.payment.failed` | notifications | `modules/payments/src/handlers.rs` |
| `payment.succeeded` | `payments.events.payment.succeeded` | ar, treasury, notifications | `modules/payments/src/handlers.rs` |

## pdf-editor

| Event Type | NATS Subject | Consumers | Source |
|-----------|-------------|-----------|--------|
| `pdf.form.generated` | `pdf-editor.events.pdf.form.generated` | — | `modules/pdf-editor/src/http/generate.rs` |
| `pdf.form.submitted` | `pdf-editor.events.pdf.form.submitted` | — | `modules/pdf-editor/src/domain/submissions/repo.rs` |
| `pdf.image.uploaded` | `pdf-editor.events.pdf.image.uploaded` | — | `modules/pdf-editor/src/domain/images/repo.rs` |
| `pdf.table.rendered` | `pdf-editor.events.pdf.table.rendered` | — | `modules/pdf-editor/src/domain/tables/repo.rs` |

## production

| Event Type | NATS Subject | Consumers | Source |
|-----------|-------------|-----------|--------|
| `production.component_issue.requested` | `production.component_issue.requested` | inventory | `modules/production/src/events/mod.rs` |
| `production.component_issued` | `production.component_issued` | — | `modules/production/src/events/mod.rs` |
| `production.downtime.ended` | `production.downtime.ended` | maintenance | `modules/production/src/events/mod.rs` |
| `production.downtime.started` | `production.downtime.started` | maintenance | `modules/production/src/events/mod.rs` |
| `production.fg_receipt.requested` | `production.fg_receipt.requested` | inventory, quality-inspection | `modules/production/src/events/mod.rs` |
| `production.fg_received` | `production.fg_received` | — | `modules/production/src/events/mod.rs` |
| `production.operation_completed` | `production.operation_completed` | quality-inspection | `modules/production/src/events/mod.rs` |
| `production.operation_started` | `production.operation_started` | — | `modules/production/src/events/mod.rs` |
| `production.routing_created` | `production.routing_created` | — | `modules/production/src/events/mod.rs` |
| `production.routing_released` | `production.routing_released` | — | `modules/production/src/events/mod.rs` |
| `production.routing_updated` | `production.routing_updated` | — | `modules/production/src/events/mod.rs` |
| `production.time_entry_created` | `production.time_entry_created` | — | `modules/production/src/events/mod.rs` |
| `production.time_entry_stopped` | `production.time_entry_stopped` | — | `modules/production/src/events/mod.rs` |
| `production.work_order_closed` | `production.work_order_closed` | — | `modules/production/src/events/mod.rs` |
| `production.work_order_created` | `production.work_order_created` | — | `modules/production/src/events/mod.rs` |
| `production.work_order_released` | `production.work_order_released` | — | `modules/production/src/events/mod.rs` |
| `production.workcenter_created` | `production.workcenter_created` | — | `modules/production/src/events/mod.rs` |
| `production.workcenter_deactivated` | `production.workcenter_deactivated` | — | `modules/production/src/events/mod.rs` |
| `production.workcenter_updated` | `production.workcenter_updated` | — | `modules/production/src/events/mod.rs` |

## quality-inspection

| Event Type | NATS Subject | Consumers | Source |
|-----------|-------------|-----------|--------|
| `quality_inspection.accepted` | `quality-inspection.events.quality_inspection.accepted` | — | `modules/quality-inspection/src/events/mod.rs` |
| `quality_inspection.disposition_decided` | `quality-inspection.events.quality_inspection.disposition_decided` | — | `modules/quality-inspection/src/events/mod.rs` |
| `quality_inspection.held` | `quality-inspection.events.quality_inspection.held` | — | `modules/quality-inspection/src/events/mod.rs` |
| `quality_inspection.inspection_recorded` | `quality-inspection.events.quality_inspection.inspection_recorded` | — | `modules/quality-inspection/src/events/mod.rs` |
| `quality_inspection.plan_created` | `quality-inspection.events.quality_inspection.plan_created` | — | `modules/quality-inspection/src/events/mod.rs` |
| `quality_inspection.rejected` | `quality-inspection.events.quality_inspection.rejected` | — | `modules/quality-inspection/src/events/mod.rs` |
| `quality_inspection.released` | `quality-inspection.events.quality_inspection.released` | — | `modules/quality-inspection/src/events/mod.rs` |

## shipping-receiving

| Event Type | NATS Subject | Consumers | Source |
|-----------|-------------|-----------|--------|
| `shipping_receiving.inbound_closed` | `shipping_receiving.inbound_closed` | — | `modules/shipping-receiving/src/events/contracts.rs` |
| `shipping_receiving.outbound_delivered` | `shipping_receiving.outbound_delivered` | — | `modules/shipping-receiving/src/events/contracts.rs` |
| `shipping_receiving.outbound_shipped` | `shipping_receiving.outbound_shipped` | — | `modules/shipping-receiving/src/events/contracts.rs` |
| `shipping_receiving.shipment_created` | `shipping_receiving.shipment_created` | — | `modules/shipping-receiving/src/events/contracts.rs` |
| `shipping_receiving.shipment_status_changed` | `shipping_receiving.shipment_status_changed` | — | `modules/shipping-receiving/src/events/contracts.rs` |
| `sr.receipt_routed_to_inspection.v1` | `sr.receipt_routed_to_inspection.v1` | — | `modules/shipping-receiving/src/events/contracts.rs` |
| `sr.receipt_routed_to_stock.v1` | `sr.receipt_routed_to_stock.v1` | — | `modules/shipping-receiving/src/events/contracts.rs` |

## subscriptions

| Event Type | NATS Subject | Consumers | Source |
|-----------|-------------|-----------|--------|
| `billrun.completed` | `subscriptions.events.billrun.completed` | — | `modules/subscriptions/src/http/bill_run_service.rs` |
| `subscriptions.status.changed` | `subscriptions.events.subscriptions.status.changed` | — | `modules/subscriptions/src/lifecycle/transitions.rs` |

## treasury

| Event Type | NATS Subject | Consumers | Source |
|-----------|-------------|-----------|--------|
| `recon.auto_matched` | `treasury.events.recon.auto_matched` | — | `modules/treasury/src/domain/recon/service.rs` |
| `recon.gl_linked` | `treasury.events.recon.gl_linked` | — | `modules/treasury/src/domain/recon/gl_link.rs` |

## workforce-competence

| Event Type | NATS Subject | Consumers | Source |
|-----------|-------------|-----------|--------|
| `workforce_competence.acceptance_authority_granted` | `workforce_competence.events.workforce_competence.acceptance_authority_granted` | — | `modules/workforce-competence/src/events/mod.rs` |
| `workforce_competence.acceptance_authority_revoked` | `workforce_competence.events.workforce_competence.acceptance_authority_revoked` | — | `modules/workforce-competence/src/events/mod.rs` |
| `workforce_competence.artifact_registered` | `workforce_competence.events.workforce_competence.artifact_registered` | — | `modules/workforce-competence/src/events/mod.rs` |
| `workforce_competence.competence_assigned` | `workforce_competence.events.workforce_competence.competence_assigned` | — | `modules/workforce-competence/src/events/mod.rs` |

---

**Summary:** 171 events across 16 modules, 26 consumer subscriptions.

*Generated on 2026-04-02T16:53:46Z*
