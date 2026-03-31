# Idempotency Audit Report — bd-xs0ry

**Date:** 2026-03-31
**Auditor:** MaroonHarbor

## Summary

Audited all 25 modules for idempotency across HTTP write endpoints, event consumers,
and outbox publishers. 17 modules are well-protected. Production and Payments have
P1 gaps that could cause duplicate side effects on retry.

## Module-by-Module Report

| Module | Operation | Idempotent | Mechanism |
|--------|-----------|:----------:|-----------|
| **ap** | Match engine INSERT | YES | ON CONFLICT (bill_line_id) DO NOTHING |
| **ap** | Receipts link INSERT | YES | ON CONFLICT (po_line_id, receipt_id) DO NOTHING |
| **ap** | Payment run allocation | YES | ON CONFLICT (allocation_id) DO NOTHING |
| **ap** | Consumer: inventory_item_received | YES | processed_events table |
| **ar** | Event consumer | YES | processed_events table (event_id dedup) |
| **ar** | Outbox enqueue | YES | ON CONFLICT (event_id) DO NOTHING |
| **ar** | Finalization replay | YES | already_processed detection |
| **bom** | Numbering sequence | YES | ON CONFLICT (tenant_id, entity) DO UPDATE |
| **consolidation** | Eliminations insert | YES | ON CONFLICT (group_id, period_id, idempotency_key) DO NOTHING |
| **customer-portal** | Status update | YES | ON CONFLICT (tenant_id, party_id, document_id) DO NOTHING |
| **customer-portal** | Auth operation | YES | ON CONFLICT (tenant_id, operation, idempotency_key) DO NOTHING |
| **fixed-assets** | AP bill consumer | YES | UNIQUE (tenant_id, bill_id, line_id) constraint |
| **fixed-assets** | Outbox enqueue | YES | ON CONFLICT (event_id) DO NOTHING |
| **gl** | All event consumers | YES | processed_events table via processed_repo |
| **gl** | Accruals / accruals reversal | YES | processed_events check before posting |
| **gl** | FX revaluation | YES | ON CONFLICT (event_id) DO NOTHING |
| **gl** | Balance updater | YES | Upsert via tx_upsert_rollup |
| **gl** | Period close snapshot | YES | ON CONFLICT (tenant_id, period_id, currency) DO NOTHING |
| **integrations** | Webhook ingest | YES | ON CONFLICT DO NOTHING (dedup constraint) |
| **integrations** | External refs create | YES | Upsert on (app_id, system, external_id) |
| **integrations** | Outbox relay | YES | ON CONFLICT (event_id) DO UPDATE |
| **integrations** | QBO normalizer | YES | Per-event ON CONFLICT DO NOTHING |
| **inventory** | Receipt (HTTP + consumer) | YES | inv_idempotency_keys table (key + request hash) |
| **inventory** | Issue (HTTP + consumer) | YES | inv_idempotency_keys table (key + request hash) |
| **inventory** | Cycle count submit | YES | inv_idempotency_keys table |
| **inventory** | Labels generate | YES | inv_idempotency_keys table |
| **inventory** | Classifications assign | YES | inv_idempotency_keys table |
| **inventory** | Lot upsert | YES | ON CONFLICT (tenant_id, item_id, lot_code) DO UPDATE |
| **inventory** | On-hand projection | YES | Upsert ON CONFLICT |
| **inventory** | Component issue consumer | YES | Derives idem key from event_id → inv_idempotency_keys |
| **inventory** | FG receipt consumer | YES | Derives idem key from event_id → inv_idempotency_keys |
| **maintenance** | Overdue outbox | YES | ON CONFLICT (event_id) DO NOTHING |
| **maintenance** | Production bridges | YES | ON CONFLICT (event_id) DO NOTHING + projection upsert |
| **maintenance** | Tenant config | YES | Upsert ON CONFLICT |
| **notifications** | Broadcast create | YES | idempotency_key guard + ON CONFLICT |
| **notifications** | Inbox insert | YES | ON CONFLICT (notification_id, user_id) DO NOTHING |
| **notifications** | Event consumers | YES | processed_events + consume_event_idempotent wrapper |
| **notifications** | Scheduled dispatch | YES | idempotency_key per attempt |
| **party** | Event processing | YES | party_processed_events table |
| **payments** | Collection consumer | YES | payments_processed_events via process_idempotent |
| **payments** | Payment attempts | YES | Deterministic idempotency keys + UNIQUE constraint |
| **payments** | Webhook handler | YES | UNIQUE (app_id, payment_id, attempt_no) + SELECT FOR UPDATE |
| **payments** | Checkout present | YES | Idempotent state transition (already presented = no-op) |
| **payments** | **Checkout session create** | **NO** | **Plain INSERT, no idempotency key — double-submit creates duplicate Tilled payment intents** |
| **production** | Work order create | YES | correlation_id check returns existing WO |
| **production** | Work order release/close | YES | State machine transition (idempotent by nature) |
| **production** | Operations start/complete | YES | State machine transition (idempotent by nature) |
| **production** | **POST component-issues** | **NO** | **No HTTP idempotency key — double-submit → duplicate outbox events → double stock issue** |
| **production** | **POST fg-receipt** | **NO** | **No HTTP idempotency key — double-submit → duplicate outbox events → double FG receipt** |
| **production** | **POST workcenters** | **NO** | **No conflict detection — double-submit creates duplicate workcenter** |
| **production** | **POST time-entries/start** | **NO** | **No idempotency — double-click starts duplicate timers** |
| **production** | **POST time-entries/manual** | **NO** | **No idempotency — double-submit creates duplicate entries** |
| **production** | **POST downtime/start** | **NO** | **No idempotency — double-click starts duplicate downtime records** |
| **production** | **POST routings** | **NO** | **No conflict detection — double-submit creates duplicate routing** |
| **production** | **Outbox enqueue** | **NO** | **Plain INSERT without ON CONFLICT (event_id) DO NOTHING** |
| **quality-inspection** | Receipt event bridge | YES | quality_inspection_processed_events |
| **quality-inspection** | Production event bridge | YES | quality_inspection_processed_events |
| **shipping-receiving** | PO approved consumer | YES | sr_processed_events table |
| **shipping-receiving** | SO released consumer | YES | sr_processed_events table |
| **subscriptions** | Event consumer | YES | processed_events table |
| **timekeeping** | Approvals submit | YES | Upsert ON CONFLICT |
| **timekeeping** | Event emission | YES | ON CONFLICT (app_id, idempotency_key) DO NOTHING |
| **timekeeping** | GL integration | YES | Deterministic posting_id → GL processed_events |
| **treasury** | Import transactions | YES | ON CONFLICT (account_id, external_id) DO NOTHING |
| **treasury** | Account operations | YES | ON CONFLICT (app_id, idempotency_key) DO NOTHING |
| **treasury** | Txn insert | YES | ON CONFLICT (event_id) DO NOTHING |
| **workflow** | Instance start | YES | ON CONFLICT (app_id, idempotency_key) DO NOTHING |
| **workflow** | Advance/transition | YES | idempotency_key |
| **workflow** | Apply/release hold | YES | ON CONFLICT (app_id, idempotency_key) DO NOTHING |

## P1 Gaps Requiring Child Beads

### 1. Production: HTTP endpoint idempotency (HIGH RISK)

**Affected endpoints:**
- `POST /api/production/work-orders/:id/component-issues` — double-submit → double stock depletion
- `POST /api/production/work-orders/:id/fg-receipt` — double-submit → double FG receipt (inventory inflation)
- `POST /api/production/workcenters` — double-submit → duplicate workcenter
- `POST /api/production/time-entries/start` — double-click → duplicate running timer
- `POST /api/production/time-entries/manual` — double-submit → duplicate time entry
- `POST /api/production/downtime/start` — double-click → duplicate downtime record
- `POST /api/production/routings` — double-submit → duplicate routing

**Root cause:** These endpoints accept a request, generate a fresh event_id, and enqueue
to the outbox. A second identical HTTP call generates a second event_id and a second outbox
entry. The downstream inventory consumers ARE idempotent per event_id, but since each HTTP
call produces a distinct event_id, both are processed.

**Fix:** Add an `idempotency_key` field to each request type. Before enqueueing, check if the
key already exists in a `production_idempotency_keys` table. Return the cached result on replay.

### 2. Production: Outbox missing ON CONFLICT (LOW RISK)

**Affected:** `production_outbox` INSERT in `domain/outbox.rs`.

**Root cause:** Plain INSERT without ON CONFLICT (event_id) DO NOTHING. Low risk because
the event_id is generated in-code just before the insert, and the whole operation runs in
a transaction. If the transaction committed, the insert won't retry. If it didn't commit,
the outbox entry doesn't exist.

**Fix:** Add `ON CONFLICT (event_id) DO NOTHING` for defensive safety.

### 3. Payments: Checkout session creation (HIGH RISK)

**Affected:** `POST /api/payments/checkout-sessions`

**Root cause:** Each call creates a new Tilled payment intent AND a new checkout_sessions row.
No idempotency key prevents double-submit. A retry or double-click creates duplicate payment
intents with real money impact.

**Fix:** Accept an `idempotency_key` (or use `invoice_id + tenant_id` as natural key). Check
for existing session before calling Tilled. Return existing session on replay.

## Modules Not Applicable

These modules either have no write operations or no event consumers:
- **numbering**: Read-only sequences (consumed by other modules)
- **pdf-editor**: Document generation (no state-changing events)
- **reporting**: Read-only metrics and exports
- **workforce-competence**: Read-only queries
