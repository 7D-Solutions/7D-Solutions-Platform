# Timekeeping Module

Tracks employee time entries, project/task assignments, approval workflows, cost allocations, billing rates, and payroll/GL export integrations.

## Architecture

- **Language**: Rust
- **Framework**: Axum
- **Database**: PostgreSQL
- **Event Bus**: NATS (outbox pattern)
- **Default Port**: 8097

## Key Endpoints

### Employees
- `POST /api/timekeeping/employees` — create employee
- `GET  /api/timekeeping/employees` — list employees
- `GET  /api/timekeeping/employees/{id}` — get employee
- `PUT  /api/timekeeping/employees/{id}` — update employee

### Projects & Tasks
- `POST /api/timekeeping/projects` — create project
- `GET  /api/timekeeping/projects` — list projects
- `POST /api/timekeeping/tasks` — create task
- `GET  /api/timekeeping/projects/{project_id}/tasks` — list tasks

### Time Entries
- `POST /api/timekeeping/entries` — create entry
- `POST /api/timekeeping/entries/correct` — correct entry
- `POST /api/timekeeping/entries/void` — void entry
- `GET  /api/timekeeping/entries` — list entries
- `GET  /api/timekeeping/entries/{entry_id}/history` — entry audit history

### Approvals
- `POST /api/timekeeping/approvals/submit` — submit for approval
- `POST /api/timekeeping/approvals/approve` — approve timesheet
- `POST /api/timekeeping/approvals/reject` — reject timesheet
- `POST /api/timekeeping/approvals/recall` — recall submission
- `GET  /api/timekeeping/approvals` — list approvals
- `GET  /api/timekeeping/approvals/pending` — list pending approvals

### Allocations & Rollups
- `POST /api/timekeeping/allocations` — create allocation
- `GET  /api/timekeeping/allocations` — list allocations
- `GET  /api/timekeeping/rollups/by-project` — rollup by project
- `GET  /api/timekeeping/rollups/by-employee` — rollup by employee
- `GET  /api/timekeeping/rollups/by-task/{project_id}` — rollup by task

### Billing & Export
- `POST /api/timekeeping/rates` — create billing rate
- `GET  /api/timekeeping/rates` — list billing rates
- `POST /api/timekeeping/billing-runs` — create billing run
- `POST /api/timekeeping/exports` — create export
- `GET  /api/timekeeping/exports` — list exports

### Ops
- `GET /api/health`, `GET /api/ready`, `GET /api/version`

## Database Tables

- `tk_employees` — employee records
- `tk_projects` / `tk_tasks` — project and task hierarchy
- `tk_timesheet_entries` — time entry records with correction/void support
- `tk_approval_requests` — approval workflow state machine
- `tk_allocations` — cost allocation rules
- `tk_exports` — payroll/GL export runs
- `tk_billing_rates` — per-employee or per-project billing rates
- `events_outbox` / `processed_events` — outbox pattern tables

## Events Emitted

- `timekeeping.entry_created` — time entry recorded
- `timekeeping.entry_corrected` — time entry corrected
- `timekeeping.entry_voided` — time entry voided
- `timekeeping.timesheet_submitted` — timesheet submitted for approval
- `timekeeping.timesheet_approved` — timesheet approved
- `timekeeping.timesheet_rejected` — timesheet rejected
- `timekeeping.timesheet_recalled` — timesheet recalled
- `timekeeping.export_completed` — export run finished
- `timekeeping.labor_cost` — GL posting for labor costs
- `timekeeping.billable_time` — AR export for billable time

## Configuration

| Variable | Default | Description |
|---|---|---|
| `DATABASE_URL` | _(required)_ | PostgreSQL connection string |
| `BUS_TYPE` | `inmemory` | Event bus: `inmemory` or `nats` |
| `NATS_URL` | `nats://localhost:4222` | NATS server URL |
| `HOST` | `0.0.0.0` | Bind address |
| `PORT` | `8097` | HTTP port |
| `CORS_ORIGINS` | `*` | Comma-separated allowed origins |
