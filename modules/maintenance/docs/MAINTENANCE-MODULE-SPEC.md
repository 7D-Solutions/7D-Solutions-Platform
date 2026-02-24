# Maintenance Module — Product Vision & Technical Specification

**7D Solutions Platform**
**Status:** Vision Document + Technical Specification (v0.1.x)
**Last updated:** 2026-02-24

---

## Revision History

| Rev | Date | Changed By | Summary |
|-----|------|-----------|---------|
| 1.0 | 2026-02-24 | Platform Orchestrator | Initial technical spec — schema, state machine, events, API, invariants, integration points |
| 2.0 | 2026-02-24 | Platform Orchestrator | Elevated to full vision doc — added business problem, user stories, structural decisions, decision log, open questions, MVP scope, technology summary. Technical contract preserved from rev 1.0. |

---

## The Business Problem

Every organization that owns physical things — vehicles, machines, HVAC units, elevators, generators, forklifts — has the same problem: **maintenance is invisible until something breaks.**

Oil changes get missed because nobody tracked the mileage. Filters go unchanged because the schedule was in someone's head. A $200 preventive service gets skipped and turns into a $15,000 engine rebuild. Equipment goes down, operations stop, and the only record of what happened is a paper logbook that nobody reads.

The businesses that do track maintenance use spreadsheets, whiteboards, or legacy CMMS software that costs six figures and takes months to deploy. Small and mid-size operations — the ones that need it most — either can't afford it or can't justify the overhead.

---

## What the Module Does

The Maintenance module is the **authoritative system for preventive, corrective, and inspection maintenance** across any type of maintainable asset. It is **asset-type agnostic**: the same data model handles a fleet truck, an HVAC unit, a CNC machine, or a building elevator. No industry-specific logic is hardcoded.

It answers five questions:
1. **What do we have?** — A lightweight asset register with tag, type, location, department, serial number, and status.
2. **When does it need service?** — Maintenance plans that trigger on calendar intervals, meter readings (odometer, engine hours, cycles), or whichever comes first.
3. **What work needs to be done?** — Work orders with full lifecycle tracking from draft through completion and cost lockdown.
4. **What did it cost?** — Parts and labor tracked per work order, rolled up to asset lifetime cost.
5. **What's overdue?** — A scheduler that detects due plans and overdue work orders and emits events for notification and escalation.

---

## Who Uses This

The module is a platform service consumed by any vertical application that manages physical assets. It does not have its own frontend — it exposes an API that frontends consume.

### Maintenance Manager / Fleet Manager
- Registers assets and defines meter types (odometer, engine hours, etc.)
- Creates maintenance plans with schedules and task checklists
- Assigns plans to specific assets
- Reviews overdue work orders and cost reports
- Closes completed work orders to lock cost

### Technician / Mechanic
- Receives assigned work orders
- Records meter readings (odometer at service time)
- Logs parts consumed and labor hours
- Follows task checklists
- Marks work orders as completed with downtime recorded

### Operations / Finance
- Reviews maintenance costs per asset and across the fleet
- Correlates maintenance spend with asset book value (via Fixed-Assets link)
- Receives cost events for GL posting
- Tracks downtime impact

### System (Scheduler)
- Evaluates plan assignments every 60 seconds
- Detects calendar-due and meter-due conditions
- Optionally auto-creates work orders when plans become due
- Detects overdue work orders and emits escalation events

---

## Design Principles

### Asset-Type Agnostic
The module never assumes what kind of thing is being maintained. The `asset_type` field (`vehicle|machinery|equipment|facility|other`) is for filtering and reporting — it does not change behavior. A vehicle oil change and a building elevator inspection follow the same plan → assignment → work order → close flow. Type-specific data goes in `metadata` (JSONB), not in dedicated columns.

### Standalone First, Integrate Later
Every integration is optional. The module boots and runs without Fixed-Assets, Inventory, GL, or Notifications. Parts can be tracked with manual descriptions and costs (standalone mode) even if Inventory is unreachable. This is not a degraded mode — it is a valid operating mode for tenants that don't use those modules.

### Cost Visibility Without GL Coupling
Maintenance tracks costs (parts + labor per work order, lifetime per asset) in its own tables. It emits cost data on events for GL to consume. It never calls GL, never stores GL account codes, never knows about journal entries. The cost data in Maintenance is operationally useful on its own — GL posting is a downstream concern.

### No Silent Failures
Every state change writes an event to the outbox atomically. If the event didn't get written, the state change didn't happen. The scheduler uses deterministic idempotency keys so it can safely re-evaluate without duplicate emissions.

---

## MVP Scope (v0.1.x)

### In Scope
- Maintainable asset register (CRUD, filterable by type/status/department)
- Per-tenant meter type definitions (odometer, hours, cycles, etc.)
- Meter reading recording with monotonicity enforcement and rollover detection
- Maintenance plans: calendar-based, meter-based, or both
- Plan-to-asset assignment with due tracking (next due date / next due meter)
- Work orders: full lifecycle with state machine (draft through closed)
- Work order parts and labor tracking (standalone mode — manual entry)
- Cost accumulation per work order and per asset
- Configurable approval gate (per tenant, defaults off)
- Background scheduler: evaluate due plans, detect overdue work orders
- Auto-create work orders from due plans (per tenant, defaults off)
- 7 domain events emitted via outbox (see Events Produced)
- Integration seams: Fixed-Assets ref, Inventory part_ref, GL cost payload, Notification subjects
- OpenAPI contract

### Explicitly Out of Scope for v1
- Parent/child asset hierarchies (engine belongs to vehicle)
- Warranty tracking and coverage rules
- IoT sensor ingestion and predictive maintenance
- Technician scheduling (availability, skills, workload balancing)
- Compliance and regulatory inspection tracking
- Active inventory integration (HTTP commands to reserve/issue parts)
- GL consumer (platform-side NATS subscriber that posts journal entries)
- AR integration (billable maintenance generating invoice line items)
- Mobile / field service app
- Frontend UI (consumed via API by vertical apps or TCP)

---

## Technology Summary

| Layer | Technology | Notes |
|-------|-----------|-------|
| Language | Rust | Platform standard |
| HTTP framework | Axum | Port TBD (assigned at integration time) |
| Database | PostgreSQL | Dedicated database, SQLx for queries and migrations |
| Event bus | NATS | Via platform `event-bus` crate |
| Auth | JWT via platform `security` crate | Tenant-scoped, role-based |
| Outbox | Platform outbox pattern | Same as all other modules |
| Metrics | Prometheus | `/metrics` endpoint |
| Crate | `maintenance-rs` | Single crate, standard module layout |

---

## Structural Decisions (The "Walls")

These are decisions that are cheap to make correctly now and very expensive to retrofit later. Each one was explicitly locked during planning.

### 1. Own lightweight asset register — do not depend on Fixed-Assets
Maintenance owns `maintainable_assets` with its own tag, name, type, location, status. It optionally links to Fixed-Assets via `fixed_asset_ref` for capitalized items, but this reference is set at registration time and never queried at runtime. Maintenance never calls Fixed-Assets. This means the module works for assets that aren't capitalized (spare equipment, facility infrastructure, tools) and doesn't break if Fixed-Assets is down.

### 2. Meter types are tenant-defined, not hardcoded
Each tenant defines their own meter types (odometer, engine hours, cycles, pressure readings, whatever they measure). The module doesn't know what "odometer" means — it just enforces monotonicity on reading values and evaluates due conditions against intervals. This keeps the module industry-agnostic.

### 3. Dual scheduling: calendar + meter, whichever comes first
Plans can be calendar-only (every 90 days), meter-only (every 5,000 miles), or both (whichever triggers first). The "both" mode is critical — a truck that sits idle for 6 months still needs an oil change even if the odometer hasn't moved. Both conditions are evaluated independently every scheduler tick.

### 4. Work order state machine is the single path for status changes
No direct SQL updates to `work_orders.status`. Every transition goes through the domain state machine which validates the from→to pair and enforces guards (e.g., completed requires `completed_at` and `downtime_minutes`). This prevents invalid states and ensures every transition emits the correct event.

### 5. Cost is immutable after close
Once a work order reaches `closed`, parts, labor, downtime, and all fields are frozen. No edits. This is the point at which cost data becomes reliable for GL posting and reporting. The completed→closed transition exists specifically to give users a window to correct mistakes before locking.

### 6. All integrations are one-way or event-driven
Maintenance never makes synchronous HTTP calls to other modules at runtime (v1). Fixed-Assets ref is set at registration. Inventory part_ref is informational. GL and Notifications subscribe to events. This means the module has zero runtime dependencies on other services.

### 7. Tenant isolation via tenant_id on every table
Standard platform multi-tenant pattern. Every table has `tenant_id` as a non-nullable field. Every index has `tenant_id` as the leading column. Every query filters by `tenant_id`. No exceptions.

### 8. No mocking in tests
Integration tests hit real Postgres, real NATS. Tests that mock the database or event bus test nothing useful. This is a platform-wide standard.

---

## Open Questions (Resolve Before Workers Start)

*None currently. All planning questions were resolved during the Grok → ChatGPT → Grok review cycle on 2026-02-24.*

*If questions emerge during implementation, workers should send mail to the orchestrator rather than making assumptions. The orchestrator will update this section with resolutions.*

---

## Domain Authority

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

## Data Ownership

### Tables Owned by Maintenance

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

### Data NOT Owned by Maintenance

Maintenance **MUST NOT** store:
- Fixed asset financial data (acquisition cost, accumulated depreciation, net book value)
- Inventory stock quantities, lot codes, or serial number assignments
- GL account codes or journal entry details
- Customer billing records or invoice references
- Technician HR data, certifications, or pay rates (only opaque `technician_ref` and per-WO labor rate)

---

## Work Order State Machine

```
draft ──→ awaiting_approval ──→ scheduled ──→ in_progress ──→ completed ──→ closed
                                                    |
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

## Meter Reading Invariants

1. **Monotonicity:** A new reading must be >= the previous maximum reading for the same `(tenant_id, asset_id, meter_type_id)`.
2. **Rollover Exception:** If `meter_types.rollover_value` is set and the new reading is less than the previous reading, it is accepted **only if** the previous reading is within 10% of the rollover value and the new reading is within 10% of zero. This handles odometer wraps (e.g., 999,999 → 00,012).
3. **Out-of-Order Timestamps:** Readings with `recorded_at` earlier than existing readings are accepted (backdating is valid), but validation is always against the highest `reading_value`, not the latest timestamp.
4. **Trigger Re-evaluation:** Any new reading insertion triggers a re-evaluation of meter-based plan assignments for that asset (via the scheduler or inline check).

---

## Scheduler

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

## Events Produced

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

## Events Consumed

| Event | Source | Action |
|-------|--------|--------|
| *(None in v1)* | — | Maintenance is event-producing only in v1. Future: consume `fixed_assets.asset.created` to auto-register maintainable assets. |

---

## Integration Points

### Fixed-Assets (Optional, Read-Only)

`maintainable_assets.fixed_asset_ref` optionally references a fixed-assets UUID. When present, the `maintenance.work_order.completed` event includes `fixed_asset_ref` so downstream consumers (GL, reporting) can correlate maintenance costs to the capitalized asset. **Maintenance never calls Fixed-Assets at runtime** — the reference is set at asset registration time.

### Inventory (Optional, HTTP Command)

Parts on a work order can optionally reference an Inventory SKU via `part_ref`. In v1, this is informational only (standalone mode). In a future bead, the work order completion flow will issue an HTTP command to Inventory to reserve/issue parts. **Degradation:** If Inventory is unavailable, the work order still completes — parts are tracked in standalone mode with manual quantities and costs.

### GL (Event-Driven, One-Way)

`maintenance.work_order.completed` carries `total_parts_minor`, `total_labor_minor`, `currency`, and `fixed_asset_ref`. A GL consumer (future bead, not part of v1 maintenance module) subscribes and posts:
- DR Maintenance Expense (or asset-specific maintenance sub-account)
- CR Parts/Labor Accrual (or AP if through vendor)

**Maintenance never calls GL.** GL subscribes to the event.

### Notifications (Event-Driven, One-Way)

The Notifications module subscribes to:
- `maintenance.plan.due` → sends maintenance-due reminders
- `maintenance.work_order.overdue` → sends escalation alerts
- `maintenance.work_order.completed` → sends completion confirmations

**Maintenance never calls Notifications.** Notifications subscribes to the events.

### AR (Future, Not in v1)

For service businesses that bill customers for maintenance work, a future integration would allow completed work orders to generate AR invoice line items. Not designed or implemented in v1.

---

## Invariants

1. **Tenant isolation is unbreakable.** Every query filters by `tenant_id`. No cross-tenant data leakage.
2. **Work order state transitions are guarded.** No direct status updates — all transitions go through the domain state machine validator.
3. **Outbox atomicity.** Every state-changing mutation writes its event to the outbox in the same database transaction. No silent event loss.
4. **Meter monotonicity.** Readings cannot decrease except for documented rollover cases.
5. **Cost immutability after close.** Once a work order is `closed`, parts, labor, and all fields are frozen. No edits.
6. **WO number uniqueness.** `wo_number` is sequential per tenant, generated via `SELECT FOR UPDATE` on `wo_counters`. Race-safe.
7. **No forced dependencies.** The module boots and functions without Fixed-Assets, Inventory, GL, or Notifications running. Every integration degrades gracefully.
8. **Scheduler idempotency.** Due and overdue events are emitted at most once per triggering condition per evaluation period.

---

## API Surface (Summary)

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

## v1 Bead Plan

**13 beads** planned and loaded into the bead pool.

| Bead | Title | Dependencies |
|------|-------|-------------|
| bd-1wmv | Scaffold (crate, Docker, /api/ready, outbox skeleton) | None |
| bd-19zc | OpenAPI contract | None |
| bd-11l8 | DB schema (all tables, enums, indexes) | bd-1wmv |
| bd-37lz | Domain types + work order state machine | None |
| bd-1lmd | Assets + meter types/readings CRUD with rollover | bd-11l8 |
| bd-1dcy | Plans + assignments, compute next due | bd-1lmd |
| bd-3nvm | Work orders CRUD with Guard-Mutation-Outbox | bd-37lz, bd-1dcy |
| bd-260x | Parts & labor subresources (standalone mode) | bd-3nvm |
| bd-1wuh | Scheduler tick: evaluate due plans, emit events | bd-1dcy, bd-1lmd, bd-1wmv |
| bd-22f2 | Auto-create work orders from due plans + approval gate | bd-1wuh, bd-3nvm |
| bd-2x15 | Overdue detection: emit overdue events | bd-1wuh, bd-3nvm |
| bd-1lxj | GL integration seam: cost payload in completed event | bd-260x, bd-3nvm |
| bd-16az | Notification integration: stable NATS subjects | bd-1wuh, bd-2x15 |

---

## v2 Roadmap (Deferred)

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

---

## ChatGPT Planning Conversation

Active conversation used for Maintenance planning:
`https://chatgpt.com/g/g-p-698c7e2090308191ba6e6eac93e3cc59-rust-postgres-modules/c/6999f1f7-4014-8325-9176-82016f9594d3`

> **Warning:** Platform Orchestrator agents may overwrite `.flywheel/chatgpt.json` with their own conversation URL. Always verify the URL before sending messages via the worker.

---

## Document Standards Reference

This document follows the revision and decision log standards defined at:
`docs/frontend/DOC-REVISION-STANDARDS.md`

---

## Decision Log

Every significant product, architecture, or standards decision is recorded here. Do not re-open a decision without adding a new row that supersedes the old one.

| Date | Decision | Rationale | Decided By |
|------|----------|-----------|-----------|
| 2026-02-24 | Own lightweight asset register — do not depend on Fixed-Assets at runtime | Module must work for non-capitalized assets (tools, facility infrastructure); no runtime dependency on another service | Platform Orchestrator + Grok |
| 2026-02-24 | Meter types are tenant-defined, not hardcoded | Different industries measure different things; module stays agnostic by letting tenants define what they track | Platform Orchestrator + Grok |
| 2026-02-24 | Dual scheduling: calendar + meter, whichever comes first | Idle assets still need calendar-based service; high-usage assets need meter-based service; "both" handles real-world fleet management correctly | Platform Orchestrator + ChatGPT |
| 2026-02-24 | Work order state machine is the single path for status changes | Prevents invalid states, ensures every transition emits the correct event; no direct SQL status updates allowed | Platform Orchestrator + ChatGPT |
| 2026-02-24 | Expanded WO states: added on_hold, awaiting_approval, closed | on_hold for waiting on parts; awaiting_approval as configurable gate; closed as cost-lock point separate from completed | Platform Orchestrator + Grok |
| 2026-02-24 | Cost is immutable after close | completed→closed transition gives users a correction window; after close, cost data is reliable for GL and reporting | Platform Orchestrator + ChatGPT |
| 2026-02-24 | All integrations are one-way or event-driven in v1 | Zero runtime dependencies on other modules; module boots and functions standalone | Platform Orchestrator + Grok |
| 2026-02-24 | Meter rollover detection with 10% threshold | Handles odometer wraps (999,999→00,012) without rejecting valid readings; threshold prevents false acceptance of erroneous readings | Platform Orchestrator + ChatGPT |
| 2026-02-24 | Validation against highest reading_value, not latest timestamp | Backdated readings are valid (data entry correction); monotonicity is about value ordering, not time ordering | Platform Orchestrator + ChatGPT |
| 2026-02-24 | Domain types bead (bd-37lz) has no schema dependency | Pure Rust types and state machine can be built and tested without a database; enables parallel work | Platform Orchestrator + Grok |
| 2026-02-24 | OpenAPI contract bead (bd-19zc) added as separate deliverable | Contract-first approach; workers implementing routes have a reference; caught as missing by Grok review | Platform Orchestrator + Grok |
| 2026-02-24 | Schema in single migration bead, multiple migration files | One bead for organizing schema work, but actual SQL split into logical migration files (not one giant file) | Platform Orchestrator + Grok |
| 2026-02-24 | No mocking in tests — integrated tests against real services | Platform-wide standard; mocked tests provide false confidence; all verification hits real Postgres and real NATS | Platform Orchestrator |
| 2026-02-24 | Tenant isolation via tenant_id on every table | Standard platform multi-tenant pattern; all indexes include tenant_id as leading column | Platform Orchestrator |
| 2026-02-24 | Standalone parts tracking is a valid operating mode, not degraded | Tenants that don't use Inventory still need to track parts consumed on work orders; manual entry with description + cost is the baseline | Platform Orchestrator + ChatGPT |
