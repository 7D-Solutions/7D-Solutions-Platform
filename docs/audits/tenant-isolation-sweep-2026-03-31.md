# Tenant Isolation Sweep — 2026-03-31

**Bead:** bd-vnuvp
**Auditor:** SageDesert
**Scope:** All 25 modules — every SQL query in `modules/*/src/**/*.rs`
**Tenant columns:** `app_id` (party, timekeeping, treasury, integrations) and `tenant_id` (all others)

## Executive Summary

**85 SQL queries across 16 modules access tenant data without a tenant_id/app_id filter.**

Every SELECT, UPDATE, and DELETE on tenant data tables must include the tenant isolation column in its WHERE clause. Querying by primary key alone (e.g., `WHERE id = $1`) is insufficient — a valid UUID from another tenant would return cross-tenant data (IDOR vulnerability).

For an aerospace/defense customer, this is a P0 security requirement.

## Methodology

1. Grep all `SELECT...FROM`, `UPDATE...SET`, `DELETE...FROM` in `modules/*/src/**/*.rs` (excluding `bin/`)
2. Exclude system tables: `information_schema`, `pg_type`, `pg_indexes`, `_sqlx_migrations`, `pg_catalog`
3. For each query, check 25-line context for `app_id` or `tenant_id`
4. Classify results: INFRASTRUCTURE (safe), TENANT_DATA (violation), COMMENT (false positive)
5. Infrastructure exclusions: outbox relay, processed_events dedup, QBO API, ingestion checkpoints

## Module Summary

| Module | Total Queries | With Tenant Filter | Missing Filter | Status |
|--------|--------------|-------------------|----------------|--------|
| ap | 176 | 136 | **23** | FAIL |
| ar | 51 | 40 | **6** | FAIL |
| bom | 14 | 14 | 0 | PASS |
| consolidation | 28 | 24 | **4** | FAIL |
| customer-portal | 11 | 11 | 0 | PASS |
| fixed-assets | 56 | 52 | **3** | FAIL |
| gl | 41 | 34 | **3** | FAIL |
| integrations | 26 | 20 | 0 | PASS (QBO excluded) |
| inventory | 36 | 34 | **1** | FAIL |
| maintenance | 66 | 64 | 0 | PASS (outbox only) |
| notifications | 15 | 10 | **5** | FAIL |
| numbering | 7 | 6 | 0 | PASS (outbox only) |
| party | 18 | 18 | 0 | PASS |
| payments | 19 | 7 | **10** | FAIL |
| pdf-editor | 26 | 23 | **3** | FAIL |
| production | 36 | 34 | **2** | FAIL |
| reporting | 89 | 77 | **4** | FAIL |
| shipping-receiving | 38 | 36 | 0 | PASS (outbox/dedup) |
| subscriptions | 9 | 4 | **4** | FAIL |
| timekeeping | 31 | 30 | **1** | FAIL |
| treasury | 71 | 54 | **12** | FAIL |
| ttp | 11 | 10 | **1** | FAIL |
| workflow | 41 | 35 | **4** | FAIL |
| workforce-competence | 1 | 1 | 0 | PASS |

**Clean modules (9):** bom, customer-portal, integrations, maintenance, numbering, party, shipping-receiving, workforce-competence
**Failing modules (16):** ap, ar, consolidation, fixed-assets, gl, inventory, notifications, payments, pdf-editor, production, reporting, subscriptions, timekeeping, treasury, ttp, workflow

## Detailed Violations by Module

### AP (23 violations) — CRITICAL

The AP module has the most violations, concentrated in consumers, match engine, bill lifecycle, PO approval, allocations, and payment runs.

| File | Line | Query | Severity |
|------|------|-------|----------|
| consumers/inventory_item_received.rs | 87 | `SELECT vendor_id FROM purchase_orders WHERE po_id = $1` | HIGH |
| consumers/inventory_item_received.rs | 364 | `SELECT COUNT(*) FROM po_receipt_links WHERE po_id = $1 AND receipt_id = $2` | HIGH |
| consumers/inventory_item_received.rs | 393 | `SELECT COUNT(*) FROM po_receipt_links WHERE po_id = $1 AND receipt_id = $2` | HIGH |
| consumers/inventory_item_received.rs | 416 | `SELECT COUNT(*) FROM po_receipt_links` (no filter at all!) | CRITICAL |
| domain/receipts_link/service.rs | 66 | `SELECT COUNT(*) FROM po_receipt_links WHERE po_line_id = $1` | HIGH |
| domain/receipts_link/service.rs | 238 | `SELECT COUNT(*) FROM po_receipt_links ...` | HIGH |
| domain/receipts_link/service.rs | 269 | `SELECT COUNT(*) FROM po_receipt_links WHERE po_line_id = $1 AND receipt_id = $2` | HIGH |
| domain/match/engine.rs | 588 | `SELECT COUNT(*) FROM three_way_match WHERE bill_id = $1` | HIGH |
| domain/match/engine.rs | 597 | `SELECT status FROM vendor_bills WHERE bill_id = $1` | HIGH |
| domain/match/engine.rs | 672 | `SELECT COUNT(*) FROM three_way_match WHERE bill_id = $1` | HIGH |
| domain/po/approve.rs | 448 | `UPDATE purchase_orders SET status = 'cancelled' WHERE po_id = $1` | HIGH |
| domain/po/approve.rs | 498 | `SELECT COUNT(*) FROM po_status WHERE po_id = $1 AND status = 'approved'` | HIGH |
| domain/bills/service.rs | 610 | `UPDATE vendor_bills SET status = 'voided' WHERE bill_id = $1` | HIGH |
| domain/allocations/service.rs | 283,307,333 | `SELECT status FROM vendor_bills WHERE bill_id = $1` (3x) | HIGH |
| domain/allocations/service.rs | 381 | `SELECT COUNT(*) FROM ap_allocations WHERE bill_id = $1` | HIGH |
| domain/payment_runs/execute.rs | 519 | `UPDATE vendor_bills SET status = 'partially_paid' WHERE bill_id = $1` | HIGH |
| domain/payment_runs/execute.rs | 537 | `SELECT amount_minor FROM ap_allocations WHERE bill_id = $1 AND payment_run_id = $2` | HIGH |
| domain/payment_runs/builder.rs | 553 | `UPDATE vendor_bills SET status = 'partially_paid' WHERE bill_id = $1` | HIGH |
| domain/reports/metrics.rs | 42 | `SELECT COUNT(*) FROM payment_runs` (global count!) | MEDIUM |
| domain/reports/metrics.rs | 46 | `SELECT COUNT(*) FROM ap_allocations` (global count!) | MEDIUM |
| domain/reports/aging.rs | 347 | `SELECT total_minor FROM vendor_bills WHERE bill_id = $1` | HIGH |

### AR (6 violations)

State machine and write-off queries missing tenant_id.

| File | Line | Query | Severity |
|------|------|-------|----------|
| finalization.rs | 257 | `SELECT status FROM ar_invoices WHERE id = $1` | HIGH |
| write_offs.rs | 191 | `SELECT id FROM ar_invoice_write_offs WHERE write_off_id = $1` | HIGH |
| write_offs.rs | 291 | `UPDATE ar_invoice_write_offs SET outbox_event_id = $1 WHERE id = $2` | HIGH |
| lifecycle.rs | 126 | `SELECT status FROM ar_invoices WHERE id = $1` | HIGH |
| lifecycle.rs | 217,244,271,298 | `UPDATE ar_invoices SET status = $1 WHERE id = $2` (4x) | HIGH |

### Consolidation (4 violations)

Group entity and rule queries.

| File | Line | Query | Severity |
|------|------|-------|----------|
| domain/config/service.rs | 186 | `SELECT * FROM csl_group_entities WHERE group_id = $1` | HIGH |
| domain/config/service_rules.rs | 62 | `SELECT * FROM csl_elimination_rules WHERE group_id = $1` | HIGH |
| domain/config/service_rules.rs | 217 | `DELETE FROM csl_fx_policies WHERE id = $1` | HIGH |
| domain/engine/compute.rs | 293 | `DELETE FROM csl_trial_balance_cache WHERE group_id = $1 AND as_of = $2` | HIGH |

### Fixed-assets (3 violations)

| File | Line | Query | Severity |
|------|------|-------|----------|
| consumers/ap_bill_approved.rs | 288 | `SELECT COUNT(*) FROM fa_ap_capitalizations ...` | HIGH |
| domain/disposals/service.rs | 450 | `SELECT status FROM fa_assets WHERE id = $1` | HIGH |
| domain/capitalize/service.rs | 428 | `SELECT source_ref FROM fa_ap_capitalizations ...` | HIGH |

### GL (3 violations)

Revenue recognition existence checks.

| File | Line | Query | Severity |
|------|------|-------|----------|
| repos/revrec_schedule_repo.rs | 49 | `SELECT EXISTS(... revrec_schedules WHERE schedule_id = $1)` | HIGH |
| repos/revrec_contract_repo.rs | 20 | `SELECT EXISTS(... revrec_contracts WHERE contract_id = $1)` | HIGH |
| repos/revrec_amendment_repo.rs | 172 | `SELECT EXISTS(... revrec_schedules WHERE schedule_id = $1)` | HIGH |

### Inventory (1 violation)

| File | Line | Query | Severity |
|------|------|-------|----------|
| domain/fulfill_service.rs | 163 | `SELECT EXISTS(... inventory_reservations WHERE reverses_reservation_id = $1)` | MEDIUM |

### Notifications (5 violations)

| File | Line | Query | Severity |
|------|------|-------|----------|
| consumers/close_calendar.rs | 142 | `SELECT 1 FROM close_calendar_reminders_sent ...` | HIGH |
| escalation/repo.rs | 145,165 | `SELECT id FROM escalation_sends ...` (2x) | HIGH |
| scheduled/repo.rs | 144 | `UPDATE scheduled_notifications SET status = 'sent' WHERE id = $1` | HIGH |
| scheduled/repo.rs | 179 | `UPDATE scheduled_notifications SET status = 'failed' WHERE id = $1` | HIGH |

### Payments (10 violations) — CRITICAL

All payment attempt lifecycle queries miss tenant_id.

| File | Line | Query | Severity |
|------|------|-------|----------|
| webhook_handler.rs | 164 | `SELECT webhook_event_id FROM payment_attempts WHERE id = $1 FOR UPDATE` | HIGH |
| webhook_handler.rs | 208 | `SELECT status::text FROM payment_attempts WHERE id = $1` | HIGH |
| webhook_handler.rs | 270 | `UPDATE payment_attempts SET status ... WHERE id = $3` | HIGH |
| reconciliation.rs | 167 | `SELECT status::text, processor_payment_id FROM payment_attempts WHERE id = $1 FOR UPDATE` | HIGH |
| reconciliation.rs | 329 | `SELECT completed_at FROM payment_attempts WHERE id = $1` | HIGH |
| lifecycle.rs | 134 | `SELECT status::text FROM payment_attempts WHERE id = $1` | HIGH |
| lifecycle.rs | 300 | `UPDATE payment_attempts SET status ... WHERE id = $2` | HIGH |
| lifecycle.rs | 333 | `UPDATE payment_attempts SET status ... WHERE id = $2` | HIGH |
| lifecycle.rs | 376 | `UPDATE payment_attempts SET status ... WHERE id = $2` | HIGH |
| lifecycle.rs | 413 | `UPDATE payment_attempts SET status ... WHERE id = $2` | HIGH |

### PDF-editor (3 violations)

| File | Line | Query | Severity |
|------|------|-------|----------|
| domain/forms/repo.rs | 158 | `SELECT MAX(display_order) FROM form_fields WHERE template_id = $1` | HIGH |
| domain/forms/repo.rs | 215 | `SELECT * FROM form_fields ...` | HIGH |
| domain/forms/repo.rs | 320 | `SELECT id FROM form_fields WHERE template_id = $1 ORDER BY display_order` | HIGH |

### Production (2 violations)

| File | Line | Query | Severity |
|------|------|-------|----------|
| domain/routings/repo.rs | 422 | `SELECT * FROM routing_steps WHERE routing_template_id = $1` | HIGH |
| domain/operations.rs | 129 | `SELECT COUNT(*) FROM operations WHERE work_order_id = $1` | HIGH |

### Reporting (4 violations)

| File | Line | Query | Severity |
|------|------|-------|----------|
| metrics.rs | 269 | `SELECT COUNT(*) FROM rpt_kpi_cache` (global count) | MEDIUM |
| metrics.rs | 290 | `SELECT COUNT(*) FROM {table}` (global count per cache table) | MEDIUM |
| domain/statements/cashflow.rs | 314,345 | Upsert without tenant in ON CONFLICT clause | MEDIUM |

### Subscriptions (4 violations)

| File | Line | Query | Severity |
|------|------|-------|----------|
| lifecycle/transitions.rs | 165 | `SELECT status FROM subscriptions WHERE id = $1` | HIGH |
| lifecycle/transitions.rs | 183 | `UPDATE subscriptions SET status = $1 WHERE id = $2` | HIGH |
| lifecycle/transitions.rs | 205 | `SELECT status FROM subscriptions WHERE id = $1` | HIGH |
| lifecycle/transitions.rs | 226 | `UPDATE subscriptions SET status = $1 WHERE id = $2` | HIGH |

### Timekeeping (1 violation)

| File | Line | Query | Severity |
|------|------|-------|----------|
| domain/billing/service.rs | 271 | `SELECT entry_id, amount_cents FROM tk_billing_run_entries WHERE billing_run_id = $1` | HIGH |

### Treasury (12 violations)

| File | Line | Query | Severity |
|------|------|-------|----------|
| domain/import/service.rs | 210 | `SELECT id FROM treasury_bank_statements WHERE account_id = $1 AND statement_hash = $2` | HIGH |
| domain/import/service.rs | 220 | `SELECT currency FROM treasury_bank_accounts WHERE id = $1` | HIGH |
| domain/import/service.rs | 332 | `SELECT COUNT(*) FROM treasury_bank_statements WHERE id = $1` | HIGH |
| domain/import/service.rs | 341 | `SELECT COUNT(*) FROM treasury_bank_transactions WHERE statement_id = $1` | HIGH |
| domain/import/service.rs | 516,564 | `SELECT amount_minor FROM treasury_bank_transactions ...` (2x) | HIGH |
| domain/recon/service.rs | 367 | `SELECT id FROM treasury_recon_matches ...` | HIGH |
| domain/recon/service.rs | 396 | `SELECT bank_transaction_id FROM treasury_recon_matches WHERE id = $1` | HIGH |
| domain/recon/service.rs | 404 | `UPDATE treasury_bank_transactions SET status = 'unmatched' WHERE id = $1` | HIGH |
| domain/recon/service.rs | 420 | `UPDATE treasury_bank_transactions SET status = 'matched' WHERE id = $1` | HIGH |
| domain/recon/metrics.rs | 23,29,36 | Global counts on treasury_recon_matches and treasury_bank_transactions | MEDIUM |

### TTP (1 violation)

| File | Line | Query | Severity |
|------|------|-------|----------|
| domain/billing_db.rs | 48 | `SELECT party_id FROM ttp_billing_run_items WHERE run_id = $1 AND status = 'invoiced'` | HIGH |

### Workflow (4 violations)

| File | Line | Query | Severity |
|------|------|-------|----------|
| domain/escalation.rs | 340 | `SELECT * FROM workflow_escalation_timers ...` | HIGH |
| domain/escalation.rs | 396 | `SELECT * FROM workflow_escalation_timers ...` | HIGH |
| domain/escalation.rs | 414 | `SELECT * FROM workflow_escalation_rules WHERE id = $1` | HIGH |
| domain/escalation.rs | 423 | `SELECT COUNT(*) FROM workflow_escalation_timers ...` | HIGH |

## Existing Tenant Boundary Tests

| Module | Has Test | File |
|--------|---------|------|
| ap | YES | tests/tenant_boundary_test.rs |
| gl | YES | tests/tenant_boundary_concurrency_test.rs |
| inventory | YES | tests/tenant_boundary_test.rs |
| maintenance | YES | tests/tenant_boundary_test.rs |
| notifications | YES | tests/tenant_boundary.rs |
| party | YES | tests/hardening_integration.rs |
| pdf-editor | YES | tests/tenant_boundary_test.rs |
| shipping-receiving | YES | tests/tenant_isolation_e2e.rs |
| ar | NO | — |
| consolidation | NO | — |
| fixed-assets | NO | — |
| payments | NO | — |
| production | NO | — |
| reporting | NO | — |
| subscriptions | NO | — |
| timekeeping | NO | — |
| treasury | NO | — |
| ttp | NO | — |
| workflow | NO | — |

## Infrastructure Queries (Excluded — Safe)

The following query patterns are intentionally unscoped and do not represent tenant isolation violations:

- **Outbox relay:** `SELECT/UPDATE events_outbox WHERE published_at IS NULL / event_id = $1` — publishes ALL events for ALL tenants
- **Idempotency dedup:** `SELECT EXISTS(... processed_events WHERE event_id = $1)` — globally unique event IDs
- **QBO API:** `SELECT * FROM Invoice` — QuickBooks Online query language, not our DB
- **Ingestion checkpoints:** `rpt_ingestion_checkpoints` — system-level ETL bookkeeping
- **Failed events:** `failed_events WHERE event_id` — event bus infrastructure

## Risk Assessment

**Attack vector:** An authenticated user with a valid session could craft API requests using UUIDs belonging to another tenant. Without tenant_id in the WHERE clause, these queries would happily return or modify cross-tenant data.

**Impact:** Data breach across tenant boundaries in an aerospace/defense ERP. This is a regulatory and contractual violation.

**Likelihood:** MEDIUM — requires knowledge of another tenant's UUIDs, but UUIDs may be exposed in logs, URLs, or API responses.

**Overall risk:** P0 — must be fixed before customer go-live.

## Recommended Fix

For each violation:
1. Add `AND tenant_id = $N` (or `AND app_id = $N`) to the WHERE clause
2. Thread the tenant context parameter through from the calling HTTP handler
3. Add a tenant isolation integration test: insert as tenant-A, query as tenant-B, expect empty/404

Child beads created for each affected module.
