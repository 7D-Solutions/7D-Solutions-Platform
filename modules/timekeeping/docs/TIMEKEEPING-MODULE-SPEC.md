# Timekeeping Module — Product Vision & Technical Specification

**7D Solutions Platform**
**Status:** Vision Document + Technical Specification (v0.1.0)
**Last updated:** 2026-02-24

---

## Revision History

| Rev | Date | Changed By | Summary |
|-----|------|-----------|---------|
| 1.0 | 2026-02-24 | Platform Orchestrator | Initial vision doc — business problem, domain model, schema, events, API, invariants, integration points, decision log. Documented from source code at v0.1.0. |
| 1.1 | 2026-02-24 | Platform Orchestrator | Review: fix 3 inaccuracies — invariant #2 overstated no-UPDATE rule, structural decision #4 overclaimed app_id on every table/index, Changed By used agent name. |

---

## The Business Problem

Every service business — consulting firms, agencies, law offices, IT contractors, maintenance companies — runs on time. Employees log hours against projects. Managers approve timesheets. Payroll needs accurate hours. Clients need accurate invoices. Finance needs labor cost visibility for GL posting.

The typical workflow involves spreadsheets, email-based approvals, and manual data re-entry into payroll and billing systems. Hours are mis-recorded, approvals are forgotten, and the gap between "hours worked" and "hours billed" grows silently. By the time finance notices, weeks of revenue have leaked.

Worse, corrections are destructive: someone edits a cell in a spreadsheet and the original record is gone. There is no audit trail, no way to prove what was originally submitted, and no way to reconstruct history when a billing dispute arises.

---

## What the Module Does

The Timekeeping module is the **authoritative system for employee time recording, approval, cost allocation, and payroll/billing export** across any project-based or service-based organization. It is **industry-agnostic**: the same data model handles a consulting engagement, a software development project, a maintenance contract, or internal overhead tracking.

It answers six questions:
1. **Who works here?** — An employee directory with codes, departments, hourly rates, and external payroll system IDs.
2. **What are they working on?** — A project and task catalog with billable/non-billable classification and GL account references.
3. **How much time did they spend?** — Append-only timesheet entries with full version history. Corrections and voids never destroy the original record.
4. **Is the time approved?** — A per-employee, per-period approval workflow with submit, approve, reject, and recall states.
5. **How should time be allocated?** — Resource allocations (planned minutes per week) and actual-time rollups by project, employee, and task.
6. **Where does the data go?** — Deterministic payroll/billing exports with content hashing, GL labor cost accrual postings, AR billable time exports, and billing runs that generate AR invoice line items.

---

## Who Uses This

The module is a platform service consumed by any vertical application that tracks employee time. It does not have its own frontend — it exposes an API that frontends consume.

### Employee / Timesheet User
- Logs time entries against projects and tasks (with date, minutes, and description)
- Submits time periods for manager approval
- Recalls submitted timesheets before they are reviewed
- Views their own entry history and correction trail

### Manager / Approver
- Reviews submitted timesheets for their team
- Approves or rejects time periods with notes
- Approval locks the period — no further edits to entries in that date range

### Payroll Administrator
- Runs payroll exports for approved periods
- Receives deterministic CSV and JSON artifacts
- Re-runs are idempotent — same data produces the same content hash

### Finance / Controller
- Views labor cost rollups by project, employee, and task
- Triggers GL labor cost accrual postings (DR Labor Expense / CR Accrued Labor)
- Triggers AR billable time exports for client invoicing
- Creates billing runs to aggregate billable entries into AR invoices

### System (Integrations)
- GL consumer subscribes to `timekeeping.labor_cost` events for journal entry creation
- AR consumer subscribes to `timekeeping.billable_time` events for invoice line items
- External payroll systems receive exported data via CSV/JSON artifacts

---

## Design Principles

### Append-Only Timesheet Entries
Entries are never updated or deleted. Every change — corrections, voids — inserts a new row with the same `entry_id` and an incremented `version`. Only the latest version (`is_current = TRUE`) counts toward totals. This gives a complete, tamper-evident audit trail. The original submission is always recoverable.

### Approval Locks Periods
Once a manager approves a time period, all entries within that date range are frozen. The entry guards check `tk_approval_requests.status = 'approved'` before allowing any mutation. This prevents post-approval changes that could invalidate payroll or billing data.

### Integer Minutes, No Floating-Point
All time durations are stored as integer minutes. This eliminates floating-point rounding errors in aggregation. Hours are computed only at the presentation layer (minutes / 60.0) and never stored. Monetary amounts use integer minor units (cents).

### Deterministic Exports
Export runs produce CSV and JSON artifacts from the same approved data. A SHA-256 content hash guarantees that re-running the same export with unchanged data yields the same result. This is the idempotency mechanism for payroll integration.

### Standalone First, Integrate Later
The module functions without GL, AR, or any external payroll system. Integration events are emitted for downstream consumers, but the module never calls those systems synchronously. Billing rates and billing runs work independently of the AR module — they produce data that AR can consume, but don't require AR to be running.

### No Silent Failures
Every state-changing mutation writes an event to the outbox atomically. If the event didn't get written, the state change didn't happen. Idempotency keys prevent duplicate processing of the same API request.

---

## MVP Scope (v0.1.x)

### In Scope
- Employee directory (CRUD, external payroll ID mapping, hourly rates, departments)
- Project and task catalog (CRUD, billable flag, GL account reference)
- Timesheet entries: create, correct, void — all append-only with version history
- Overlap detection: same employee + date + project + task cannot have duplicate active entries
- Period lock enforcement: approved periods reject entry mutations
- Maximum single-entry duration guard (24h = 1440 minutes)
- Approval workflow: submit, approve, reject, recall — with audit trail
- Approval actions table for full status transition history
- Resource allocations: planned minutes per week per employee per project/task
- Actual-time rollups by project, employee, and task
- Export runs: deterministic CSV + JSON from approved entries with content hashing
- Idempotent export re-runs via content hash matching
- Billing rates: named hourly rates per tenant
- Billing runs: aggregate billable entries into AR-ready line items with no-double-billing invariant
- GL integration: labor cost accrual postings with deterministic posting IDs (UUID v5)
- AR integration: billable time export with deterministic export IDs (UUID v5)
- HTTP-level API idempotency keys
- Transactional outbox for all domain events
- Prometheus metrics (entries created, approvals processed, exports, SLO histograms)
- Admin endpoints for projection status, consistency checks
- Healthz, ready, version endpoints

### Explicitly Out of Scope for v1
- Overtime rules and compliance (labor law varies by jurisdiction)
- PTO / leave management
- Clock-in / clock-out (punch clock) mode
- Geolocation or GPS tracking of work time
- Mobile offline entry with sync
- Shift scheduling and assignment
- Multi-currency within a single billing run
- Active payroll system push (module produces exports; payroll systems pull)
- Frontend UI (consumed via API by vertical apps or TCP)

---

## Technology Summary

| Layer | Technology | Notes |
|-------|-----------|-------|
| Language | Rust | Platform standard |
| HTTP framework | Axum | Port 8097 (default) |
| Database | PostgreSQL | Dedicated database, SQLx for queries and migrations |
| Event bus | NATS | Via platform `event-bus` crate; InMemory fallback for dev |
| Auth | JWT via platform `security` crate | Tenant-scoped, permission-based (`timekeeping.mutate`) |
| Outbox | Platform outbox pattern | Same as all other modules |
| Projections | Platform `projections` crate | Admin projection status + consistency check |
| Metrics | Prometheus | `/metrics` endpoint with SLO-grade histograms |
| Crate | `timekeeping` | Single crate, standard module layout |

---

## Structural Decisions (The "Walls")

These are decisions that are cheap to make correctly now and very expensive to retrofit later.

### 1. Append-only entries — never update, never delete
Timesheet entries follow an event-sourced-like pattern: every change is a new row. The `entry_id` groups all versions of a logical entry. `is_current` marks the active version. Corrections inherit the original `work_date` and `employee_id`. Voids set `minutes = 0` with `entry_type = 'void'`. This design guarantees a complete audit trail and makes reconciliation disputes trivially provable.

### 2. Approval status is the period lock mechanism
Rather than a separate lock table, the approval workflow's `status = 'approved'` on `tk_approval_requests` is the lock. Entry guards query this table before allowing mutations. This means the lock is always consistent with the business action (approval) and cannot drift.

### 3. Duration stored as integer minutes
No floating-point anywhere in the storage layer. `minutes` is `INT` with `CHECK (minutes >= 0)`. Hours are computed only for display and export (`minutes / 60.0`). This prevents the classic 0.1 + 0.2 != 0.3 rounding error that plagues spreadsheet-based timekeeping.

### 4. Tenant isolation via app_id on domain tables
Standard platform multi-tenant pattern. All domain tables (`tk_employees`, `tk_projects`, `tk_tasks`, `tk_timesheet_entries`, `tk_approval_requests`, `tk_allocations`, `tk_export_runs`, `tk_billing_rates`, `tk_billing_runs`, `tk_idempotency_keys`) have `app_id` as a non-nullable field. Child/junction tables (`tk_approval_actions`, `tk_billing_run_entries`) inherit tenant scope through their FK to a parent domain table. Infrastructure tables (`events_outbox`, `processed_events`) follow the platform outbox schema and do not carry `app_id`. Every query on domain tables filters by `app_id`.

### 5. Billing runs enforce no-double-billing
Each billing run links to entries via `tk_billing_run_entries`. The billable entry query explicitly excludes entries that already appear in any billing run (`NOT EXISTS (SELECT 1 FROM tk_billing_run_entries ...)`). This is the architectural guarantee against invoicing the same hours twice.

### 6. Deterministic IDs for integration events
GL labor cost postings use UUID v5 derived from `(app_id, employee_id, project_id, period_start, period_end)`. AR billable time exports use UUID v5 derived from `(app_id, period_start, period_end)`. This means re-running the same period produces the same event ID, which downstream consumers' `processed_events` tables use as the idempotency key.

### 7. All integrations are one-way or event-driven
Timekeeping never makes synchronous HTTP calls to GL, AR, or payroll systems. GL and AR subscribe to outbox events. Payroll receives exported CSV/JSON artifacts. Billing runs produce data for AR but do not call AR. This means the module has zero runtime dependencies on other services.

### 8. No mocking in tests
Integration tests hit real Postgres, real NATS. This is a platform-wide standard.

---

## Domain Authority

Timekeeping is the **source of truth** for:

| Domain Entity | Timekeeping Authority |
|---------------|----------------------|
| **Employees** | Employee directory for time tracking: code, name, email, department, external payroll ID, hourly rate, currency, active status. |
| **Projects** | Project catalog: code, name, description, billable flag, GL account reference, active status. |
| **Tasks** | Task breakdown within projects: code, name, active status. Scoped to a parent project. |
| **Timesheet Entries** | Append-only time records: employee, project, task, work date, minutes, description. Full version history with corrections and voids. |
| **Approval Requests** | Per-employee, per-period approval status: draft, submitted, approved, rejected. Approval locks the period. |
| **Approval Actions** | Audit trail of approval state transitions: who did what, when, with what notes. |
| **Allocations** | Planned minutes per week per employee per project/task, with effective date ranges. |
| **Export Runs** | Payroll/billing export records: type, period, status, record count, content hash, artifacts. |
| **Billing Rates** | Named hourly rates per tenant for billable time. |
| **Billing Runs** | Aggregated billing records: customer, period, amount, AR invoice reference, entry links. |

Timekeeping is **NOT** authoritative for:
- GL account balances or journal entries (GL module owns this)
- AR invoices, payments, or customer balances (AR module owns this)
- Employee HR records, payroll processing, or tax calculations (external payroll systems own this)
- Project budgets or financial forecasting (not implemented)

---

## Data Ownership

### Tables Owned by Timekeeping

All domain tables use `app_id` for multi-tenant isolation. Every query on domain tables **MUST** filter by `app_id`. Child tables (`tk_approval_actions`, `tk_billing_run_entries`) inherit scope via FK. Infrastructure tables (`events_outbox`, `processed_events`) follow the platform outbox schema.

| Table | Purpose | Key Fields |
|-------|---------|------------|
| **tk_employees** | Employee directory for time tracking | `id`, `app_id`, `employee_code` (unique per app), `first_name`, `last_name`, `email`, `department`, `external_payroll_id`, `hourly_rate_minor`, `currency`, `active` |
| **tk_projects** | Project catalog | `id`, `app_id`, `project_code` (unique per app), `name`, `description`, `billable`, `gl_account_ref`, `active` |
| **tk_tasks** | Task breakdown under projects | `id`, `app_id`, `project_id` (FK), `task_code` (unique per project), `name`, `active` |
| **tk_timesheet_entries** | Append-only time records | `id` (BIGSERIAL), `entry_id` (logical UUID), `version`, `app_id`, `employee_id`, `project_id`, `task_id`, `work_date`, `minutes`, `description`, `entry_type` (original\|correction\|void), `is_current`, `billing_rate_id`, `billable`, `created_by` |
| **tk_approval_requests** | Per-employee, per-period approval status | `id`, `app_id`, `employee_id`, `period_start`, `period_end`, `status` (draft\|submitted\|approved\|rejected), `total_minutes`, `submitted_at`, `reviewed_at`, `reviewer_id`, `reviewer_notes` |
| **tk_approval_actions** | Audit trail for approval transitions | `id` (BIGSERIAL), `approval_id` (FK), `action`, `actor_id`, `notes` |
| **tk_allocations** | Planned resource allocations | `id`, `app_id`, `employee_id`, `project_id`, `task_id`, `allocated_minutes_per_week`, `effective_from`, `effective_to`, `active` |
| **tk_export_runs** | Payroll/billing export records | `id`, `app_id`, `export_type`, `period_start`, `period_end`, `status` (pending\|in_progress\|completed\|failed), `record_count`, `metadata` (JSONB), `content_hash`, `error_message` |
| **tk_billing_rates** | Named hourly billing rates | `id`, `app_id`, `name` (unique per app), `rate_cents_per_hour`, `is_active` |
| **tk_billing_runs** | Aggregated billing run records | `id`, `app_id`, `ar_customer_id`, `from_date`, `to_date`, `amount_cents`, `ar_invoice_id`, `idempotency_key` (unique), `status` |
| **tk_billing_run_entries** | Links billing runs to the entries they cover | `billing_run_id` (FK), `entry_id`, `amount_cents` — composite PK |
| **tk_idempotency_keys** | HTTP-level API idempotency | `app_id`, `idempotency_key` (unique per app), `request_hash`, `response_body`, `status_code`, `expires_at` |
| **events_outbox** | Transactional outbox for domain events | Standard platform schema |
| **processed_events** | Event deduplication for consumers | Standard platform schema |

**Monetary Precision:** All monetary amounts use **integer minor units** (e.g., `hourly_rate_minor` in cents, `amount_cents` in cents). Currency stored as 3-letter ISO 4217 code.

**Tenant Isolation:** All domain tables include `app_id` as a non-nullable field. Child/junction tables inherit tenant scope via FK. Infrastructure tables follow the platform outbox schema.

### Data NOT Owned by Timekeeping

Timekeeping **MUST NOT** store:
- GL account balances, journal entries, or chart of accounts (GL module)
- AR invoices, customer records, or payment history (AR module)
- Employee payroll details, tax withholdings, or benefits (external payroll systems)
- Project budgets, milestones, or Gantt schedules (not implemented)

---

## Approval State Machine

```
draft ──→ submitted ──→ approved
                   └──→ rejected

submitted ──→ draft (recall)
rejected  ──→ submitted (re-submit)
```

### Transition Rules

| From | Allowed To | Guard |
|------|-----------|-------|
| draft | submitted | Employee submits period for review; total_minutes computed from current entries |
| submitted | approved | Reviewer grants approval; period becomes locked |
| submitted | rejected | Reviewer rejects; employee can correct and re-submit |
| submitted | draft | Employee recalls before review (recall action) |
| rejected | submitted | Employee re-submits after corrections |
| approved | *(terminal for v1)* | No further transitions; period is locked |

### Period Lock Effect

When `status = 'approved'`:
- `create_entry` is rejected for work_dates within the period
- `correct_entry` is rejected for entries with work_dates within the period
- `void_entry` is rejected for entries with work_dates within the period

---

## Events Produced

All events use the platform `EventEnvelope` and are written to the module outbox atomically with the triggering mutation.

| Event | Trigger | Key Payload Fields |
|-------|---------|-------------------|
| `timesheet_entry.created` | New time entry created | `entry_id`, `app_id`, `employee_id`, `work_date`, `minutes`, `version` |
| `timesheet_entry.corrected` | Existing entry corrected | `entry_id`, `app_id`, `employee_id`, `work_date`, `old_minutes`, `new_minutes`, `version` |
| `timesheet_entry.voided` | Entry voided (cancelled) | `entry_id`, `app_id`, `employee_id`, `work_date`, `voided_minutes`, `version` |
| `timesheet.submitted` | Timesheet submitted for approval | `approval_id`, `app_id`, `employee_id`, `period_start`, `period_end`, `total_minutes` |
| `timesheet.approved` | Timesheet approved by reviewer | `approval_id`, `app_id`, `employee_id`, `period_start`, `period_end`, `reviewer_id`, `total_minutes` |
| `timesheet.rejected` | Timesheet rejected by reviewer | `approval_id`, `app_id`, `employee_id`, `period_start`, `period_end`, `reviewer_id`, `notes` |
| `timesheet.recalled` | Timesheet recalled by employee | `approval_id`, `app_id`, `employee_id`, `period_start`, `period_end` |
| `export_run.completed` | Export run generated | `run_id`, `app_id`, `export_type`, `period_start`, `period_end`, `record_count`, `content_hash` |
| `timekeeping.labor_cost` | GL labor cost accrual generated | `posting_id`, `app_id`, `employee_id`, `employee_name`, `project_id`, `period_start`, `period_end`, `total_minutes`, `hourly_rate_minor`, `currency`, `total_cost_minor`, `posting_date` |
| `timekeeping.billable_time` | AR billable time exported | `export_id`, `app_id`, `period_start`, `period_end`, `lines[]`, `total_amount_minor`, `currency` |

---

## Events Consumed

| Event | Source | Action |
|-------|--------|--------|
| *(None in v1)* | — | Timekeeping is event-producing only in v1. Future: consume employee lifecycle events from an HR/identity module to auto-register employees. |

---

## Integration Points

### GL (Event-Driven, One-Way)

`timekeeping.labor_cost` carries the full accrual posting payload: employee, project, period, total minutes, hourly rate, computed cost in minor units, and a deterministic `posting_id`. A GL consumer subscribes and posts:
- DR Labor Expense (or project-specific labor sub-account)
- CR Accrued Labor (liability account)

The `posting_id` is UUID v5, deterministic on `(app_id, employee_id, project_id, period_start, period_end)`. Re-running the same period produces the same ID, which the GL consumer's `processed_events` table uses for deduplication.

**Timekeeping never calls GL.** GL subscribes to the event.

### AR (Event-Driven + Billing Runs)

Two integration paths exist:

1. **Event-driven:** `timekeeping.billable_time` carries approved billable time grouped by employee and project, with line items and amounts. AR subscribes to create draft invoices.

2. **Billing runs:** `POST /api/timekeeping/billing-runs` aggregates unbilled billable entries for a customer and period, records the run, and returns line items. The caller (TCP or an AR integration layer) uses these to create an AR invoice, then calls `set_invoice_id` to link the run to the invoice. The no-double-billing invariant ensures entries are included in at most one billing run.

**Timekeeping never calls AR directly.** The billing run produces data; AR creation is the caller's responsibility.

### External Payroll Systems (Export Artifacts)

Export runs produce deterministic CSV and JSON artifacts from approved entries. These are designed for payroll system import:
- CSV with employee ID, project, task, hours, description
- JSON with metadata (app, period, record count, totals) and line items

The `external_payroll_id` field on `tk_employees` maps timekeeping employees to external payroll system identifiers (ADP, Gusto, etc.).

**Timekeeping produces exports.** Payroll systems pull the data.

### Projects → GL Account Reference (Soft Reference)

`tk_projects.gl_account_ref` stores a GL account code string. This is a soft reference — not enforced via FK. The GL integration uses this to determine which expense account receives the labor cost posting.

---

## Invariants

1. **Tenant isolation is unbreakable.** Every query filters by `app_id`. No cross-tenant data leakage.
2. **Entries are append-only (data).** Entry data (minutes, description, type) is never modified. Corrections and voids insert new rows with incremented versions. The only UPDATE is flipping `is_current = FALSE` on superseded versions within the same transaction.
3. **Only one current version per entry.** `is_current = TRUE` is set only on the latest version. Previous versions are flipped to `FALSE` atomically within the same transaction.
4. **Approved periods are locked.** Entry guards reject create, correct, and void operations for work_dates within an approved period.
5. **Outbox atomicity.** Every state-changing mutation writes its event to the outbox in the same database transaction.
6. **Overlap prevention.** Only one active (non-void) entry per `(app_id, employee_id, work_date, project_id, task_id)` combination.
7. **Maximum entry duration.** A single entry cannot exceed 1440 minutes (24 hours).
8. **Non-negative duration.** Entry minutes must be >= 0 (DB CHECK constraint + domain guard).
9. **Employee code uniqueness.** `employee_code` is unique per `app_id`.
10. **Project code uniqueness.** `project_code` is unique per `app_id`.
11. **Task code uniqueness.** `task_code` is unique per `project_id`.
12. **No double billing.** Entries appear in at most one billing run, enforced by `NOT EXISTS` on `tk_billing_run_entries`.
13. **Deterministic export hashing.** Same input data always produces the same content hash, enabling idempotent re-runs.
14. **Deterministic integration IDs.** GL posting IDs and AR export IDs use UUID v5 — same inputs always produce the same IDs.
15. **No forced dependencies.** The module boots and functions without GL, AR, or external payroll systems running.

---

## API Surface (Summary)

### Employees
- `POST /api/timekeeping/employees` — Create employee
- `GET /api/timekeeping/employees` — List employees (tenant-scoped, filterable by active)
- `GET /api/timekeeping/employees/{id}` — Get employee detail
- `PUT /api/timekeeping/employees/{id}` — Update employee
- `DELETE /api/timekeeping/employees/{id}` — Deactivate employee (soft delete)

### Projects & Tasks
- `POST /api/timekeeping/projects` — Create project
- `GET /api/timekeeping/projects` — List projects
- `GET /api/timekeeping/projects/{id}` — Get project detail
- `PUT /api/timekeeping/projects/{id}` — Update project
- `DELETE /api/timekeeping/projects/{id}` — Deactivate project
- `POST /api/timekeeping/tasks` — Create task under project
- `GET /api/timekeeping/projects/{project_id}/tasks` — List tasks for project
- `GET /api/timekeeping/tasks/{id}` — Get task detail
- `PUT /api/timekeeping/tasks/{id}` — Update task
- `DELETE /api/timekeeping/tasks/{id}` — Deactivate task

### Timesheet Entries
- `POST /api/timekeeping/entries` — Create time entry (original)
- `GET /api/timekeeping/entries` — List current entries (by employee, date range)
- `POST /api/timekeeping/entries/correct` — Correct an existing entry
- `POST /api/timekeeping/entries/void` — Void an entry (set minutes = 0)
- `GET /api/timekeeping/entries/{entry_id}/history` — Full version history for a logical entry

### Approvals
- `POST /api/timekeeping/approvals/submit` — Submit timesheet for approval
- `POST /api/timekeeping/approvals/approve` — Approve submitted timesheet
- `POST /api/timekeeping/approvals/reject` — Reject submitted timesheet
- `POST /api/timekeeping/approvals/recall` — Recall submitted timesheet
- `GET /api/timekeeping/approvals` — List approval requests (by employee, period)
- `GET /api/timekeeping/approvals/pending` — List pending reviews
- `GET /api/timekeeping/approvals/{id}` — Get approval detail
- `GET /api/timekeeping/approvals/{id}/actions` — Audit trail for an approval

### Allocations & Rollups
- `POST /api/timekeeping/allocations` — Create allocation
- `GET /api/timekeeping/allocations` — List allocations (filterable by employee, project)
- `GET /api/timekeeping/allocations/{id}` — Get allocation detail
- `PUT /api/timekeeping/allocations/{id}` — Update allocation
- `DELETE /api/timekeeping/allocations/{id}` — Deactivate allocation
- `GET /api/timekeeping/rollups/by-project` — Actual time by project
- `GET /api/timekeeping/rollups/by-employee` — Actual time by employee
- `GET /api/timekeeping/rollups/by-task/{project_id}` — Actual time by task

### Exports
- `POST /api/timekeeping/exports` — Create export run (CSV + JSON)
- `GET /api/timekeeping/exports` — List export runs
- `GET /api/timekeeping/exports/{id}` — Get export run detail

### Billing
- `POST /api/timekeeping/rates` — Create billing rate
- `GET /api/timekeeping/rates` — List active billing rates
- `POST /api/timekeeping/billing-runs` — Create billing run

### Admin
- `POST /api/timekeeping/admin/projection-status` — Query projection status (requires admin token)
- `POST /api/timekeeping/admin/consistency-check` — Run consistency check (requires admin token)
- `GET /api/timekeeping/admin/projections` — List projections (requires admin token)

### Operational
- `GET /healthz` — Liveness probe
- `GET /api/health` — Health check (no external deps)
- `GET /api/ready` — Readiness probe (verifies DB connectivity)
- `GET /api/version` — Module identity and schema version
- `GET /metrics` — Prometheus metrics

---

## v2 Roadmap (Deferred)

These capabilities are explicitly out of scope for v1 but anticipated:

| Feature | Rationale for Deferral |
|---------|----------------------|
| **Overtime Rules** | Labor law compliance varies by jurisdiction (federal, state, union). Requires configurable rule engine. |
| **PTO / Leave Management** | Different accrual models, carryover rules, approval chains. Separate domain. |
| **Clock-In / Clock-Out** | Punch clock mode with start/stop timestamps. Different data model from duration-based entries. |
| **Geolocation Tracking** | GPS-verified work time. Privacy implications, mobile app dependency. |
| **Shift Scheduling** | Employee availability, shift templates, rotation rules. Separate domain with its own state machine. |
| **Multi-Currency Billing** | A single billing run currently assumes one currency. Multi-currency needs exchange rate handling. |
| **Active Payroll Push** | Currently exports are pull-based. Push integration requires per-system adapters (ADP, Gusto, etc.). |
| **Approval Delegation** | Designating a backup approver when a manager is unavailable. |
| **Budget Tracking** | Project budgets with actual-vs-planned alerts. Requires budget data model. |
| **Mobile Offline Mode** | Offline entry creation with conflict resolution on sync. |

---

## Decision Log

Every significant product, architecture, or standards decision is recorded here. Do not re-open a decision without adding a new row that supersedes the old one.

| Date | Decision | Rationale | Decided By |
|------|----------|-----------|-----------|
| 2026-02-18 | Append-only entries — never update, never delete | Complete audit trail for payroll and billing disputes; original submissions are always recoverable; avoids destructive edits | Platform Orchestrator |
| 2026-02-18 | Approval status is the period lock mechanism | Lock is always consistent with the business action; no separate lock table to drift; guards query approval_requests directly | Platform Orchestrator |
| 2026-02-18 | Duration stored as integer minutes, not floating-point | Eliminates rounding errors in aggregation; hours computed only at presentation layer; matches monetary minor-units pattern | Platform Orchestrator |
| 2026-02-18 | Tenant isolation via app_id | Standard platform multi-tenant pattern; all indexes have app_id as leading column | Platform Orchestrator |
| 2026-02-18 | Overlap detection: one active entry per (employee, date, project, task) | Prevents duplicate time recording; corrections use the correct/void mechanism, not duplicate entries | Platform Orchestrator |
| 2026-02-18 | Maximum entry duration: 1440 minutes (24 hours) | Physical constraint; no single entry can exceed a day; multi-day work uses separate entries per date | Platform Orchestrator |
| 2026-02-18 | GL account ref is a soft reference (string, not FK) | Timekeeping must not depend on GL at runtime; the reference is informational for downstream consumers | Platform Orchestrator |
| 2026-02-18 | Deterministic export hashing with SHA-256 | Enables idempotent re-runs; same data produces same hash; prevents duplicate payroll submissions | Platform Orchestrator |
| 2026-02-19 | UUID v5 for GL posting IDs and AR export IDs | Deterministic deduplication in downstream consumers; same period produces same event ID | Platform Orchestrator |
| 2026-02-19 | Billing runs use NOT EXISTS to prevent double billing | Entries linked to a billing run are excluded from future runs; architectural guarantee, not just application logic | Platform Orchestrator |
| 2026-02-19 | All integrations are one-way or event-driven | Zero runtime dependencies on GL, AR, or payroll systems; module boots and functions standalone | Platform Orchestrator |
| 2026-02-20 | Billing rate → entry linkage via billing_rate_id column | Entries reference billing rates directly; billing runs join through this to compute amounts | Platform Orchestrator |
| 2026-02-21 | Admin endpoints use projections crate | Consistent with platform admin pattern; projection status and consistency checks reusable across modules | Platform Orchestrator |
| 2026-02-21 | Permission guard: timekeeping.mutate for all write operations | Single permission covers all mutations; reads are unenforced at this stage | Platform Orchestrator |
