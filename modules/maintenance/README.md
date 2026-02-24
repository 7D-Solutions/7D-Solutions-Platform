# Maintenance Module

Fleet/facility maintenance management: assets, meter readings, preventive plans, work orders, and overdue detection.

## NATS Event Subjects

All events are wrapped in an `EventEnvelope` (see `platform/event-bus`) with these guaranteed fields:

| Field | Description |
|-------|-------------|
| `event_id` | UUID, unique per event |
| `event_type` | Matches the NATS subject |
| `tenant_id` | Tenant that owns the aggregate |
| `occurred_at` | RFC 3339 timestamp |
| `source_module` | Always `"maintenance"` |
| `source_version` | Cargo.toml version at build time |
| `payload` | Domain-specific data (see below) |

### Stable Subjects

Defined in `src/events/subjects.rs`. Changing a subject is a **breaking change**.

| Subject | Trigger |
|---------|---------|
| `maintenance.work_order.created` | New work order created |
| `maintenance.work_order.status_changed` | Any status transition (non-completed) |
| `maintenance.work_order.completed` | WO transitioned to `completed` (includes cost totals) |
| `maintenance.work_order.closed` | WO transitioned to `closed` |
| `maintenance.work_order.cancelled` | WO cancelled |
| `maintenance.work_order.overdue` | Overdue detection tick finds past-due WO |
| `maintenance.meter_reading.recorded` | New meter reading recorded |
| `maintenance.plan.due` | Scheduler tick detects plan assignment is due |
| `maintenance.plan.assigned` | Plan assigned to an asset |

### Wildcard Subscriptions

```
maintenance.work_order.>   # all WO lifecycle events
maintenance.>              # all maintenance events
```
