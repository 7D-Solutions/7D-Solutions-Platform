# Maintenance Module — Vision & Roadmap

**Version**: 0.1.0
**Last Updated**: 2026-02-25
**Status**: Active

---

## 1. Mission

Maintenance manages **work orders and preventive maintenance** for physical assets and facilities. It tracks corrective and preventive maintenance tasks, records meter readings for condition-based triggers, captures parts and labor costs, and posts costs to GL. Maintenance answers "what needs maintenance, what's overdue, and what did it cost?"

### Non-Goals

Maintenance does **NOT**:
- Own asset financial data (depreciation/disposal owned by Fixed Assets)
- Own inventory/parts stock levels (owned by Inventory)
- Own employee identity (owned by Party/Timekeeping)
- Post GL entries directly (uses `gl.posting.requested`)

---

## 2. Domain Authority

| Domain Entity | Maintenance Authority |
|---|---|
| **Work Orders** | Corrective and preventive maintenance tasks with status lifecycle |
| **Preventive Maintenance Plans** | Scheduled (time-based) and meter-based maintenance triggers |
| **Meter Readings** | Equipment meter tracking for condition-based maintenance |
| **Parts and Labor** | Cost tracking per work order (materials + labor hours) |
| **Tenant Config** | Per-tenant maintenance settings (overdue thresholds, etc.) |

---

## 3. Data Ownership

| Table | Purpose |
|---|---|
| `work_orders` | Work order headers with status and assignment |
| `wo_lines` / parts_and_labor | Cost line items per work order |
| `pm_plans` | Preventive maintenance plan definitions |
| `meter_readings` | Equipment meter values with timestamps |
| `maintenance_tenant_config` | Per-tenant maintenance configuration |
| `events_outbox` | Module outbox for NATS |
| `processed_events` | Consumer idempotency |

---

## 4. Events

**Produces:**
- `maintenance.work_order.created` — new work order opened
- `maintenance.work_order.status_changed` — WO status transition
- `maintenance.work_order.completed` — WO work finished (triggers GL cost posting)
- `maintenance.work_order.closed` — WO fully closed
- `maintenance.work_order.cancelled` — WO cancelled
- `maintenance.work_order.overdue` — WO past due date
- `maintenance.meter_reading.recorded` — new meter reading captured
- `maintenance.plan.due` — PM plan triggered (time or meter threshold)
- `maintenance.plan.assigned` — PM plan auto-generated a work order

**Planned (not yet implemented):**
- `gl.posting.requested` — cost posting for completed work orders (deferred to GL integration bead)

**Consumes:**
- None currently

---

## 5. Key Invariants

1. Completed work orders are immutable (cost locked)
2. Meter readings are append-only (no edits to historical values)
3. PM plans generate work orders idempotently (no duplicate generation per trigger)
4. Overdue detection runs on scheduler tick, not on read
5. Tenant isolation on every table and query

---

## 6. Integration Map

- **GL** → `maintenance.work_order.completed` carries cost data; GL posting integration planned (not yet implemented)
- **Inventory** → future: parts consumption from inventory stock
- **Fixed Assets** → future: link work orders to fixed asset records
- **Timekeeping** → future: labor hours from timesheet entries

---

## 7. Roadmap

### v0.1.0 (current)
- Work order CRUD with status lifecycle
- Preventive maintenance plan management (time + meter based)
- Meter reading recording and threshold detection
- Parts and labor cost capture per work order
- Overdue detection with scheduler
- Tenant configuration
- GL cost posting for completed work orders
- Event emission for all lifecycle transitions

### v1.0.0 (proven)
- Inventory parts consumption integration
- Fixed asset linkage
- Mobile work order management
- Photo/document attachment to work orders
- SLA tracking and compliance reporting
- Predictive maintenance analytics
