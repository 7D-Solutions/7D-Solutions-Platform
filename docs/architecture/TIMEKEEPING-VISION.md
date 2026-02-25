# Timekeeping Module — Vision & Roadmap

**Version**: 0.1.0
**Last Updated**: 2026-02-25
**Status**: Active

---

## 1. Mission

Timekeeping is the **time and labor authority** — it captures employee time entries, manages approval workflows, allocates costs to projects/jobs, and generates payroll/billing exports. Timekeeping answers "who worked on what, for how long, and at what rate?"

### Non-Goals

Timekeeping does **NOT**:
- Own employee identity (Party Master owns canonical identity)
- Own payroll processing (export data consumed by external payroll systems)
- Own project financial budgets (future module or product-layer concern)
- Post GL entries directly (future: via `gl.posting.requested`)

---

## 2. Domain Authority

| Domain Entity | Timekeeping Authority |
|---|---|
| **Employees** | Employee master for time tracking context |
| **Projects** | Project/job definitions for time allocation |
| **Timesheet Entries** | Individual time records with hours, date, project, billing rate |
| **Approvals** | Timesheet approval workflow state |
| **Allocations** | Cost allocation to projects/jobs |
| **Exports** | Payroll and billing export batches (content-hash deduped) |
| **Billing Rates** | Per-employee or per-project billing rates |

---

## 3. Data Ownership

| Table | Purpose |
|---|---|
| `employees` | Employee records for time tracking |
| `projects` | Project/job definitions |
| `timesheet_entries` | Time records with hours, date, project, rates |
| `approvals` | Approval status per entry or batch |
| `allocations` | Cost allocation records |
| `exports` | Export batch records with content hash |
| `billing_rates` | Rate definitions per employee/project |
| `events_outbox` | Module outbox for NATS |
| `processed_events` | Consumer idempotency |

---

## 4. Events

**Produces:**
- `timekeeping.entry.*` (planned) — timesheet submission/approval events
- `timekeeping.approval.*` (planned) — approval workflow events

**Consumes:**
- None currently

---

## 5. Key Invariants

1. Approved entries are immutable (must void and re-enter)
2. Export batches are content-hash deduplicated
3. Billing rates apply in priority order: entry override > project rate > employee rate
4. Tenant isolation on every table and query

---

## 6. Integration Map

- **Party** → employee identity may reference party_id (future)
- **GL** → future: labor cost posting via `gl.posting.requested`
- **AR** → future: billable time → invoice line items
- **Maintenance** → future: labor hours on work orders

---

## 7. Roadmap

### v0.1.0 (current)
- Employee and project management
- Timesheet entry CRUD
- Approval workflow (submit → approve → reject)
- Cost allocation to projects
- Payroll/billing export generation
- Billing rate management

### v1.0.0 (proven)
- Mobile time entry
- GPS/geofence clock-in/clock-out
- Overtime calculation rules
- GL labor cost posting
- AR billable time integration
- Scheduling and shift management
