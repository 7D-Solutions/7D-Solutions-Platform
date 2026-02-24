# Maintenance Module — Scope, Boundaries, Contracts (v0.1.x)

**7D Solutions Platform**
**Status:** Specification Document (Baseline Architecture)
**Date:** 2026-02-24
**Module Version:** 0.1.x

---

## 1. Mission & Non-Goals

### Mission
The Maintenance module is the **authoritative system for preventive, corrective, and inspection maintenance** across any type of maintainable asset — vehicles, machinery, equipment, facilities, or any other physical thing that requires upkeep. It tracks what needs to be maintained, when, what work was done, what it cost, and what is overdue. The module is **asset-type agnostic**: the same data model handles a fleet truck, an HVAC unit, a CNC machine, or a building elevator.

### Non-Goals
Maintenance does **NOT**:
- Own the financial asset register or depreciation schedules (delegated to Fixed-Assets module)
- Execute inventory transactions for spare parts (delegated to Inventory module via HTTP commands)
- Post journal entries to the general ledger (emits cost events for GL consumer to post)
- Send notifications directly (emits events for Notifications module to deliver)
- Manage technician HR records, payroll, or certifications (out of scope for v1)
- Provide IoT/sensor ingestion or predictive analytics (deferred to v2)
- Track warranties, compliance certifications, or regulatory inspections (deferred to v2)
- Model parent/child asset hierarchies (deferred to v2)

---

## 2. Domain Authority

Maintenance is the **source of truth** for:

| Domain Entity | Maintenance Authority |
|---------------|----------------------|
| **Maintainable Assets** | Lightweight asset register: tag, name, type, location, serial number, department, status. Optional link to Fixed-Assets for capitalized items. |
| **Meter Types** | Definitions of what is measured per tenant (odometer, engine hours, cycles, etc.) with unit labels and rollover values. |
| **Meter Readings** | Timestamped readings per asset per meter type. Monotonicity enforced with rollover detection. |
| **Maintenance Plans** | Recurring maintenance templates: schedule type (calendar, meter, or both), intervals, priority, estimated cost, task checklists. |
| **Plan Assignments** | Links between plans and specific assets. Tracks last completed date/reading and next due date/meter for scheduling. |
| **Work Orders** | Individual units of maintenance work. Full lifecycle from draft through completion and close. Carries type (preventive/corrective/inspection), priority, assignment, checklist, and downtime. |
| **Work Order Parts** | Parts consumed on a work order: description, quantity, unit cost. Optionally linked to Inventory SKU. |
| **Work Order Labor** | Labor entries: technician reference, hours, rate, description. |
| **Cost Accumulation** | Total parts + labor cost per work order, rolled up to asset lifetime maintenance cost. |

Maintenance is **NOT** authoritative for:
- Asset acquisition cost, depreciation, or net book value (Fixed-Assets module owns this)
- Spare parts stock levels, lot/serial tracking, or reorder points (Inventory module owns this)
- GL expense account balances or journal entries (GL module owns this)
- Customer billing for maintenance services (AR module would own this if implemented)

---

## 3. Data Ownership

### 3.1 Tables Owned by Maintenance

All tables use `tenant_id` for multi-tenant isolation. Every query **MUST** filter by `tenant_id`.

| Table | Purpose | Key Fields |
|-------|---------|------------|
| **maintainable_assets** | Lightweight asset register | `id`, `tenant_id`, `asset_tag`, `name`, `description`, `asset_type` (vehicle\|machinery\|equipment\|facility\|other), `location`, `department`, `responsible_person`, `serial_number`, `fixed_asset_ref` (nullable UUID), `status` (active\|inactive\|retired), `metadata` (JSONB) |
| **meter_types** | Per-tenant meter definitions | `id`, `tenant_id`, `name`, `unit_label`, `rollover_value` (nullable) |
| **meter_readings** | Timestamped readings per asset per meter | `id`, `tenant_id`, `asset_id`, `meter_type_id`, `reading_value` (BIGINT), `recorded_at`, `recorded_by` |
| **maintenance_plans** | Recurring schedule templates | `id`, `tenant_id`, `name`, `description`, `asset_type_filter` (nullable), `schedule_type` (calendar\|meter\|both), `calendar_interval_days`, `meter_type_id`, `meter_interval` (BIGINT), `priority` (low\|medium\|high\|critical), `estimated_duration_minutes`, `estimated_cost_minor`, `task_checklist` (JSONB), `is_active` |
| **maintenance_plan_assignments** | Plan-to-asset links with due tracking | `id`, `plan_id`, `asset_id`, `last_completed_at`, `last_meter_reading`, `next_due_date`, `next_due_meter`, `state` (active\|paused\|completed) |
| **work_orders** | Individual maintenance tasks | `id`, `tenant_id`, `asset_id`, `plan_assignment_id` (nullable), `wo_number`, `title`, `description`, `wo_type` (preventive\|corrective\|inspection), `priority`, `status`, `assigned_to`, `scheduled_date`, `started_at`, `completed_at`, `closed_at`, `checklist` (JSONB), `downtime_minutes`, `notes` |
| **wo_counters** | Tenant-scoped WO number sequence | `tenant_id` (PK), `next_number` (BIGINT) |
| **work_order_parts** | Parts consumed on a work order | `id`, `work_order_id`, `part_description`, `part_ref` (nullable), `quantity`, `unit_cost_minor`, `currency`, `inventory_issue_ref` (nullable) |
| **work_order_labor** | Labor entries per work order | `id`, `work_order_id`, `technician_ref`, `hours_decimal`, `rate_minor`, `currency`, `description` |
| **events_outbox** | Standard platform outbox | Module-owned, same schema as other modules |
| **processed_events** | Event deduplication | Module-owned, same schema as other modules |

**Monetary Precision:** All monetary amounts use **integer minor units** (e.g., `unit_cost_minor` in cents). Currency stored as 3-letter ISO 4217 code.

**Tenant Isolation:** Every table includes `tenant_id` as a non-nullable field. All indexes include `tenant_id` as the leading column.

### 3.2 Data NOT Owned by Maintenance

Maintenance **MUST NOT** store:
- Fixed asset financial data (acquisition cost, accumulated depreciation, net book value)
- Inventory stock quantities, lot codes, or serial number assignments
- GL account codes or journal entry details
- Customer billing records or invoice references
- Technician HR data, certifications, or pay rates (only opaque `technician_ref` and per-WO labor rate)

---

## 4. Work Order State Machine

```
draft ──→ awaiting_approval ──→ scheduled ──→ in_progress ──→ completed ──→ closed
                                                    │
                                                on_hold ──→ in_progress (resume)

Any state ──→ cancelled (terminal)
```

### Transition Rules

| From | Allowed To | Guard |
|------|-----------|-------|
| draft | awaiting_approval, scheduled, cancelled | If tenant requires approvals → must go through awaiting_approval |
| awaiting_approval | scheduled, cancelled | Approval granted |
| scheduled | in_progress, cancelled | — |
| in_progress | on_hold, completed, cancelled | completed requires `completed_at` and `downtime_minutes` |
| on_hold | in_progress, cancelled | Resume from hold |
| completed | closed | closed locks all edits (cost finalized) |
| closed | *(terminal)* | No further transitions |
| cancelled | *(terminal)* | No further transitions |

### Configurable Behavior (Per Tenant)

| Setting | Default | Effect |
|---------|---------|--------|
| `approvals_required` | false | If true, work orders must pass through `awaiting_approval` |
| `auto_create_on_due` | false | If true, scheduler auto-creates work orders when plans become due |

---

## 5. Meter Reading Invariants

1. **Monotonicity:** A new reading must be >= the previous maximum reading for the same `(tenant_id, asset_id, meter_type_id)`.
2. **Rollover Exception:** If `meter_types.rollover_value` is set and the new reading is less than the previous reading, it is accepted **only if** the previous reading is within 10% of the rollover value and the new reading is within 10% of zero. This handles odometer wraps (e.g., 999,999 → 00,012).
3. **Out-of-Order Timestamps:** Readings with `recorded_at` earlier than existing readings are accepted (backdating is valid), but validation is always against the highest `reading_value`, not the latest timestamp.
4. **Trigger Re-evaluation:** Any new reading insertion triggers a re-evaluation of meter-based plan assignments for that asset (via the scheduler or inline check).

---

## 6. Scheduler

A background task polls every 60 seconds (configurable via `MAINTENANCE_SCHED_INTERVAL_SECS`).

### Each Tick Evaluates

1. **Calendar-based plans:** `maintenance_plan_assignments` where `next_due_date <= now` and `state = active` → emit `maintenance.plan.due`.
2. **Meter-based plans:** For each active assignment with a `meter_type_id`, fetch the latest `meter_readings.reading_value` for that asset+meter. If `reading_value >= next_due_meter` → emit `maintenance.plan.due`.
3. **Both (whichever first):** If `schedule_type = both`, either condition triggers due.
4. **Overdue work orders:** Work orders with `status IN (scheduled, in_progress, on_hold)` and `scheduled_date < today` → emit `maintenance.work_order.overdue`.

### Idempotency

Each event emission uses a deterministic idempotency key:
- Plan due: `(tenant_id, assignment_id, due_kind, due_date_bucket)` — prevents re-emission on subsequent ticks
- Overdue: `(tenant_id, wo_id, overdue_day)` — one overdue event per day per work order

### Auto-Create Work Orders

When `auto_create_on_due = true` and a plan.due event is generated, the scheduler also creates a work order in the same transaction. Initial status respects `approvals_required`.

---

## 7. Events Produced

All events use the platform `EventEnvelope` and are written to the module outbox atomically with the triggering mutation.

| Event | Trigger | Key Payload Fields |
|-------|---------|-------------------|
| `maintenance.work_order.created` | Work order created | `wo_id`, `wo_number`, `asset_id`, `wo_type`, `priority`, `plan_assignment_id` |
| `maintenance.work_order.status_changed` | Any status transition | `wo_id`, `old_status`, `new_status` |
| `maintenance.work_order.completed` | Work order completed | `wo_id`, `asset_id`, `total_parts_minor`, `total_labor_minor`, `currency`, `downtime_minutes`, `fixed_asset_ref` |
| `maintenance.work_order.closed` | Work order closed (cost locked) | `wo_id`, `asset_id` |
| `maintenance.work_order.overdue` | Scheduled date passed | `wo_id`, `asset_id`, `days_overdue`, `priority` |
| `maintenance.meter_reading.recorded` | New meter reading | `asset_id`, `meter_type_id`, `reading_value`, `recorded_at` |
| `maintenance.plan.due` | Maintenance plan becomes due | `assignment_id`, `plan_id`, `asset_id`, `due_kind` (calendar\|meter), `due_value` |

---

## 8. Events Consumed

| Event | Source | Action |
|-------|--------|--------|
| *(None in v1)* | — | Maintenance is event-producing only in v1. Future: consume `fixed_assets.asset.created` to auto-register maintainable assets. |

---

## 9. Integration Points

### 9.1 Fixed-Assets (Optional, Read-Only)

`maintainable_assets.fixed_asset_ref` optionally references a fixed-assets UUID. When present, the `maintenance.work_order.completed` event includes `fixed_asset_ref` so downstream consumers (GL, reporting) can correlate maintenance costs to the capitalized asset. **Maintenance never calls Fixed-Assets at runtime** — the reference is set at asset registration time.

### 9.2 Inventory (Optional, HTTP Command)

Parts on a work order can optionally reference an Inventory SKU via `part_ref`. In v1, this is informational only (standalone mode). In a future bead, the work order completion flow will issue an HTTP command to Inventory to reserve/issue parts. **Degradation:** If Inventory is unavailable, the work order still completes — parts are tracked in standalone mode with manual quantities and costs.

### 9.3 GL (Event-Driven, One-Way)

`maintenance.work_order.completed` carries `total_parts_minor`, `total_labor_minor`, `currency`, and `fixed_asset_ref`. A GL consumer (future bead, not part of v1 maintenance module) subscribes and posts:
- DR Maintenance Expense (or asset-specific maintenance sub-account)
- CR Parts/Labor Accrual (or AP if through vendor)

**Maintenance never calls GL.** GL subscribes to the event.

### 9.4 Notifications (Event-Driven, One-Way)

The Notifications module subscribes to:
- `maintenance.plan.due` → sends maintenance-due reminders
- `maintenance.work_order.overdue` → sends escalation alerts
- `maintenance.work_order.completed` → sends completion confirmations

**Maintenance never calls Notifications.** Notifications subscribes to the events.

### 9.5 AR (Future, Not in v1)

For service businesses that bill customers for maintenance work, a future integration would allow completed work orders to generate AR invoice line items. Not designed or implemented in v1.

---

## 10. Invariants

1. **Tenant isolation is unbreakable.** Every query filters by `tenant_id`. No cross-tenant data leakage.
2. **Work order state transitions are guarded.** No direct status updates — all transitions go through the domain state machine validator.
3. **Outbox atomicity.** Every state-changing mutation writes its event to the outbox in the same database transaction. No silent event loss.
4. **Meter monotonicity.** Readings cannot decrease except for documented rollover cases.
5. **Cost immutability after close.** Once a work order is `closed`, parts, labor, and all fields are frozen. No edits.
6. **WO number uniqueness.** `wo_number` is sequential per tenant, generated via `SELECT FOR UPDATE` on `wo_counters`. Race-safe.
7. **No forced dependencies.** The module boots and functions without Fixed-Assets, Inventory, GL, or Notifications running. Every integration degrades gracefully.
8. **Scheduler idempotency.** Due and overdue events are emitted at most once per triggering condition per evaluation period.

---

## 11. API Surface (Summary)

Full OpenAPI contract: `contracts/maintenance/maintenance-v0.1.0.yaml`

### Assets
- `POST /api/maintenance/assets` — Create maintainable asset
- `GET /api/maintenance/assets` — List assets (tenant-scoped, filterable by type/status)
- `GET /api/maintenance/assets/{id}` — Get asset detail
- `PATCH /api/maintenance/assets/{id}` — Update asset

### Meter Types & Readings
- `POST /api/maintenance/meter-types` — Define a meter type
- `GET /api/maintenance/meter-types` — List meter types
- `POST /api/maintenance/assets/{id}/readings` — Record a meter reading
- `GET /api/maintenance/assets/{id}/readings` — List readings for asset

### Maintenance Plans & Assignments
- `POST /api/maintenance/plans` — Create maintenance plan
- `GET /api/maintenance/plans` — List plans
- `PATCH /api/maintenance/plans/{id}` — Update plan
- `POST /api/maintenance/plans/{id}/assign` — Assign plan to asset
- `GET /api/maintenance/assets/{id}/assignments` — List assignments for asset

### Work Orders
- `POST /api/maintenance/work-orders` — Create work order (ad-hoc or from plan)
- `GET /api/maintenance/work-orders` — List work orders (filterable by status/priority/asset)
- `GET /api/maintenance/work-orders/{id}` — Get work order detail
- `POST /api/maintenance/work-orders/{id}/transition` — Transition status
- `POST /api/maintenance/work-orders/{id}/complete` — Complete work order
- `POST /api/maintenance/work-orders/{id}/close` — Close work order (lock cost)
- `POST /api/maintenance/work-orders/{id}/cancel` — Cancel work order

### Parts & Labor
- `POST /api/maintenance/work-orders/{id}/parts` — Add part
- `GET /api/maintenance/work-orders/{id}/parts` — List parts
- `DELETE /api/maintenance/work-orders/{id}/parts/{part_id}` — Remove part
- `POST /api/maintenance/work-orders/{id}/labor` — Add labor entry
- `GET /api/maintenance/work-orders/{id}/labor` — List labor entries
- `DELETE /api/maintenance/work-orders/{id}/labor/{labor_id}` — Remove labor entry

### Operational
- `GET /api/ready` — Readiness check
- `GET /metrics` — Prometheus metrics

---

## 12. v2 Roadmap (Deferred)

These capabilities are explicitly out of scope for v1 but anticipated:

| Feature | Rationale for Deferral |
|---------|----------------------|
| **Asset Hierarchy** | Parent/child relationships (engine → vehicle, compressor → HVAC). Requires recursive queries and UI complexity. |
| **Warranty Tracking** | Warranty terms, expiry dates, coverage rules per asset. Needs provider/vendor integration. |
| **Predictive Maintenance** | IoT sensor ingestion, anomaly detection, ML-based failure prediction. Requires streaming infrastructure. |
| **Technician Scheduling** | Availability calendars, skill matching, workload balancing. Needs its own domain model. |
| **Compliance & Certifications** | Regulatory inspection tracking, certification expiry, audit evidence. Varies heavily by industry. |
| **Inventory Integration (Active)** | HTTP commands to Inventory for parts reservation/issue on WO. Requires degradation testing. |
| **GL Consumer** | Platform-side NATS consumer that posts maintenance cost journal entries. Not part of maintenance module itself. |
| **AR Integration** | Billable maintenance: completed WOs generate AR invoice line items for service businesses. |
| **Mobile / Field Service** | Offline-capable mobile app for technicians to update WOs, record readings, capture photos. |
