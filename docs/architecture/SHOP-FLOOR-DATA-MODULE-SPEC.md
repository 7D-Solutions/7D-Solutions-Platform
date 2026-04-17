# Shop-Floor-Data Module — Scope, Boundaries, Contracts (draft v0.1)

> **NOT PLATFORM SCOPE (2026-04-16, revised).** User ruling: kiosks, operator sessions, and kiosk-driven labor capture are shop-specific hardware + bespoke operator workflow — same reasoning that kept machine-comm in Fireproof. The only truly cross-cutting piece, barcode resolution, moves to the Inventory extension (see `PLATFORM-EXTENSIONS-SPEC.md`). This spec is retained for historical reference only.

**7D Solutions Platform**
**Status:** Retired draft — not being built as a new platform module
**Date:** 2026-04-16
**Proposed module name:** `shop-floor-data`
**Source of migration:** Fireproof ERP — `sfdc/` module (~2,140 LOC)
**Cross-vertical applicability:** Fireproof + HuberPower (any manufacturer with shop-floor terminals and barcode-driven production data capture)

---

## 1. Mission & Non-Goals

### Mission
Shop-Floor-Data is the **authoritative system for raw data captured from human operators interacting with the shop floor**: kiosks/terminals, operator sessions, barcode scans that establish work order and operation context, and labor time records generated from those sessions.

### Non-Goals
Shop-Floor-Data does **NOT**:
- Own machine telemetry or machine-to-machine integration (stays in Fireproof's `machine_comm/` — bespoke to the specific CNC machines in the shop)
- Own work orders or operations (Production owns — SFDC references `work_order_id`, `operation_id`)
- Own accounting-level labor time allocation for cost posting (delegated to Production's time entries — SFDC emits events; Production consumes and rolls up for costing)
- Perform authentication or identity management (delegated to platform Identity-Auth — SFDC validates sessions against platform JWTs / badge-number lookups)
- Own workcenter capacity or scheduling (Production owns)
- Handle formal signoff / attestation (delegated to Shop-Floor-Gates)

---

## 2. Domain Authority

Shop-Floor-Data is the **source of truth** for:

| Domain Entity | Authority |
|---------------|-----------|
| **Kiosks** | Physical floor terminal registration: name, location, optional workcenter binding, device identifier, active state |
| **Kiosk Sessions** | Operator authenticated session on a kiosk: operator_id, badge_number, start/end timestamps, end reason |
| **Operation Scans** | Raw barcode scan events during a session: barcode_raw, scan_type, resolved WO/op/part/lot/serial context, resolved flag |
| **Labor Records** | Clock-in/clock-out pairs linked to a session and (when applicable) a scan: operator, work_order, operation, duration, approval state |
| **Barcode Resolution** | Decoded meaning of a scanned barcode — what entity it represents (WO, operation, part, lot, serial, user badge) |

Shop-Floor-Data is **NOT** authoritative for:
- The work order or operation itself (Production owns)
- The operator's identity or credentials (Identity-Auth owns)
- Part or lot master data (Inventory owns)
- Accounting-grade rollups of labor to job cost (Production owns; SFDC feeds raw)

---

## 3. Data Ownership

All tables include `tenant_id`. Shared-DB model. High write-volume: indexes tuned for `(tenant_id, work_order_id)` and `(tenant_id, operator_id, clock_in_at)` access patterns.

| Table | Purpose | Key fields |
|-------|---------|-----------|
| **kiosks** | Physical floor terminals | `id`, `tenant_id`, `kiosk_name`, `location`, `work_center_id` (nullable — ref → Production workcenter), `device_identifier`, `is_active`, `created_at`, `updated_at` |
| **kiosk_sessions** | Operator login sessions | `id`, `tenant_id`, `kiosk_id`, `operator_id`, `badge_number`, `session_start`, `session_end`, `end_reason` (canonical: manual_logout/idle_timeout/kiosk_restart/admin_force/shift_change), `created_at` |
| **operation_scans** | Barcode scan events during sessions | `id`, `tenant_id`, `session_id`, `barcode_raw`, `scan_type` (canonical: work_order/operation/part/lot/serial/badge/other), `work_order_id`, `operation_id`, `part_number`, `part_revision`, `lot_number`, `serial_numbers` (array), `resolved` (boolean — true if resolution succeeded), `resolution_error` (text if resolved=false), `scanned_at`, `created_at` |
| **labor_records** | Clock-in/out labor entries | `id`, `tenant_id`, `labor_number`, `session_id` (nullable — manual labor entries may not have a session), `scan_id` (nullable), `work_order_id`, `operation_id`, `operator_id`, `badge_number`, `clock_in_at`, `clock_out_at`, `duration_minutes` (int — computed on clock-out), `approval_status` (canonical: pending/approved/rejected/auto_approved), `approved_by`, `approved_at`, `rejection_reason`, `notes`, `created_at`, `updated_at` |
| **barcode_format_rules** | Tenant-configured barcode parsing rules | `id`, `tenant_id`, `rule_name`, `pattern_regex`, `scan_type_when_matched` (canonical scan_type code), `priority`, `active`, `created_at`, `updated_at`, `updated_by` — determines how raw barcodes get classified |
| **session_end_reason_labels**, **scan_type_labels**, **approval_status_labels** | Tenant display labels over canonical enums | Standard label-table shape |

**Note on `barcode_format_rules`:** tenants have different barcode conventions (Code 128 with custom prefix, QR with embedded JSON, etc.). Platform provides pluggable parsing rules per tenant rather than a hardcoded format. Rules evaluated in priority order; first match wins.

**Note on data volume:** SFDC write volume is high (scan every few seconds per active kiosk, continuous labor ticks). Anticipate partitioning `operation_scans` and `labor_records` by month for retention management.

---

## 4. OpenAPI Surface

### 4.1 Kiosk Endpoints
- `POST /api/shop-floor-data/kiosks` — Register kiosk (admin)
- `PUT /api/shop-floor-data/kiosks/:id` — Update
- `POST /api/shop-floor-data/kiosks/:id/deactivate` — Deactivate
- `GET /api/shop-floor-data/kiosks/:id` — Retrieve
- `GET /api/shop-floor-data/kiosks` — List (filter: work_center_id, is_active)

### 4.2 Session Endpoints (called from kiosk client)
- `POST /api/shop-floor-data/sessions/start` — Start session (body: kiosk_id, operator_id or badge_number)
- `POST /api/shop-floor-data/sessions/:id/end` — End session (body: end_reason)
- `GET /api/shop-floor-data/sessions/:id` — Retrieve with all scans + labor records
- `GET /api/shop-floor-data/sessions` — List (filter: operator_id, kiosk_id, active, date ranges)

### 4.3 Scan Endpoints
- `POST /api/shop-floor-data/sessions/:session_id/scans` — Record a scan (body: barcode_raw); server resolves via barcode_format_rules, returns resolved fields + resolved flag
- `GET /api/shop-floor-data/scans/:id` — Retrieve
- `GET /api/shop-floor-data/scans` — List (filter: session_id, work_order_id, scanned_after)

### 4.4 Labor Endpoints
- `POST /api/shop-floor-data/labor/clock-in` — Start labor entry (body: session_id OR operator_id, work_order_id, operation_id)
- `POST /api/shop-floor-data/labor/:id/clock-out` — Close labor entry; computes duration
- `POST /api/shop-floor-data/labor/:id/approve` — Supervisor approval (role-gated)
- `POST /api/shop-floor-data/labor/:id/reject` — Supervisor rejection
- `POST /api/shop-floor-data/labor` — Manual labor entry (no kiosk session) — body includes clock_in_at + clock_out_at directly; auto-enters `pending` or `auto_approved` per tenant policy
- `GET /api/shop-floor-data/labor/:id` — Retrieve
- `GET /api/shop-floor-data/labor` — List (filters: operator_id, work_order_id, date ranges, approval_status)

### 4.5 Barcode Rule Endpoints
- `GET /api/shop-floor-data/barcode-rules` — List tenant rules
- `POST /api/shop-floor-data/barcode-rules` — Add rule
- `PUT /api/shop-floor-data/barcode-rules/:id` — Update
- `POST /api/shop-floor-data/barcode-rules/test` — Test a barcode against current rules (returns which rule matched, resolved scan_type, parsed fields)

### 4.6 Label Endpoints
- Standard per-canonical-field, matching earlier specs.

---

## 5. Events Produced & Consumed

Platform envelope. `source_module` = `"shop-floor-data"`. High event volume — consumers should use batch-friendly subscriptions.

### 5.1 Events Produced

| Event name | Trigger | Key payload |
|------------|---------|-------------|
| `shop_floor_data.session.started.v1` | Session start | `session_id`, `kiosk_id`, `operator_id`, `session_start` |
| `shop_floor_data.session.ended.v1` | Session end | `session_id`, `session_end`, `end_reason`, `duration_minutes` |
| `shop_floor_data.scan.recorded.v1` | Scan recorded and resolved | `scan_id`, `session_id`, `scan_type`, `work_order_id`, `operation_id`, `part_number`, `lot_number`, `resolved` |
| `shop_floor_data.scan.unresolved.v1` | Scan failed to resolve | `scan_id`, `barcode_raw`, `resolution_error` |
| `shop_floor_data.labor.clocked_in.v1` | Labor entry opened | `labor_id`, `operator_id`, `work_order_id`, `operation_id`, `clock_in_at` |
| `shop_floor_data.labor.clocked_out.v1` | Labor entry closed | `labor_id`, `operator_id`, `work_order_id`, `operation_id`, `clock_in_at`, `clock_out_at`, `duration_minutes` |
| `shop_floor_data.labor.approved.v1` | Labor approved | `labor_id`, `approved_by`, `approved_at` |
| `shop_floor_data.labor.rejected.v1` | Labor rejected | `labor_id`, `rejection_reason`, `rejected_by` |

### 5.2 Events Consumed

| Event name | Source | Behavior |
|------------|--------|----------|
| `production.work_order.cancelled.v1` | Production | Auto-close any open labor entries against that WO (mark end reason, clock-out at cancellation time), emit clocked_out |
| `production.operation.completed.v1` | Production | Auto-close open labor entries against that operation if policy is `close-labor-on-op-complete` (tenant-configurable; future) |
| `party.user.deactivated.v1` | Party/Identity-Auth | End any active sessions for that operator, auto-close labor entries |

---

## 6. State Machines

### 6.1 Kiosk Session
```
active ──┬──> ended (end_reason explains why)
```
Single transition. Sessions are not updated — only ended.

### 6.2 Operation Scan
Scans are single-event records. No state machine — `resolved` bool either true (resolution succeeded) or false (error captured).

### 6.3 Labor Record
```
open (clocked_in, not yet clocked_out) ──> closed (clocked_out) ──┬──> approved
                                                                   ├──> rejected
                                                                   └──> auto_approved (tenant policy)
```
Approval is post-clockout; an open labor record cannot be approved.

---

## 7. Security & Tenant Isolation

- Shared DB, row-level isolation by `tenant_id`.
- Kiosk authentication: kiosks authenticate via device-identifier + API key; sessions are started by operator badge scan; badge → operator_id resolution done against Identity-Auth.
- Role gates:
  - `shop_floor_data:kiosk:manage` (admin)
  - `shop_floor_data:labor:approve`, `shop_floor_data:labor:reject` (supervisor)
  - `shop_floor_data:barcode_rule:manage` (admin)
- Operators can see their own labor records without special roles; cross-operator visibility requires supervisor role.
- Data retention: raw scans + sessions may be aged (tenant-configurable retention) to manage DB size while preserving labor records indefinitely for audit.

---

## 8. Required Invariants

1. **Labor records reference a valid WO and operation.** FK-like to Production's work orders/operations.
2. **Clock-out requires open labor.** Cannot clock out a closed labor record; cannot clock out a labor record before clock-in.
3. **Duration computed on clock-out.** `duration_minutes = clock_out_at - clock_in_at`. Enforced at write time.
4. **One open labor per operator-operation.** An operator cannot have two open labor entries on the same `(operator_id, operation_id)` simultaneously. Attempt to clock-in while another is open auto-closes the prior with `end_reason = shift_change` (or per tenant policy).
5. **Session end closes all open labor.** Ending a session auto-closes any of that session's open labor records (same time as session_end).
6. **Scan resolution best-effort but logged.** A scan that doesn't resolve is still persisted with `resolved = false` + error message; downstream consumers can re-attempt resolution if rules change later.
7. **Approval append-only pattern for corrections.** Approve/reject is a state change on the labor record, but the transition is logged via event; cannot silently flip states.
8. **Barcode rules evaluated by priority.** Lowest-priority-number first; rule test endpoint returns the matched rule.
9. **Tenant isolation cross-table.** All joins share `tenant_id`.
10. **Canonical values in events.** Tenants can relabel but events carry canonical codes.

---

## 9. Cross-module integration notes

- **Production:** biggest integration partner. SFDC emits labor clock-in/out events; Production consumes to roll up labor hours per WO/operation for costing. Production may also expose its workcenter list for kiosk `work_center_id` binding.
- **Shop-Floor-Gates:** gates references the same WO/operation IDs. If an operation has active holds, the kiosk UI may surface that on scan (UI-side check via Gates API), but enforcement of "cannot start operation with hold" lives in Production.
- **Identity-Auth:** badge-to-operator resolution. Session start calls Identity-Auth's lookup.
- **Inventory:** scans referencing lot/serial numbers resolve through Inventory's lot queries for validation.
- **Workforce-Competence:** labor records reference operator; Workforce-Competence exposes "is operator qualified for this operation" checks. Optionally integrated: on clock-in, kiosk UI checks operator qualification and warns/blocks if not qualified. Enforcement is vertical policy; platform exposes the check, doesn't force use.

---

## 10. Open questions

- **Overlap with Production Time Entries.** Production currently has a "Time Entries" endpoint group. SFDC's labor records may duplicate. Resolution: SFDC owns raw clock events; Production's time entries become a derived view or event-sourced projection of SFDC labor. This is a consolidation task during implementation — may require a Production-side refactor to remove its own time-entry table in favor of SFDC-sourced data.
- **Manual labor entries.** Workers who don't use a kiosk (remote, off-shift) may submit labor via an office app. Current design: `POST /labor` without session_id works, records direct clock_in/clock_out. Some verticals may require all labor to flow through kiosks; others don't. Recommend: allow manual, role-gate with a different permission (`shop_floor_data:labor:manual_entry`).
- **Offline kiosk support.** Kiosks in poor-connectivity areas buffer locally and sync when online. Current design: out of scope for platform; kiosk clients handle buffering and batch-post when connected. Platform accepts batch submissions via array body on POST endpoints.
- **Approval policy config.** `auto_approved` outcome requires per-tenant policy (e.g. "auto-approve if duration ≤ 8 hours and operator is qualified"). Recommend: seed a `labor_approval_policy` table in v0.2 — for v0.1, default everything to `pending` and let supervisors approve manually.
- **Partition strategy.** High-volume tables (scans, labor) may need time-based partitioning. Defer decision — sample data doesn't require it; revisit before first real tenant goes live.
- **Kiosk-to-supervisor mapping.** Some verticals want supervisor notifications on specific kiosks (only the shift supervisor for their station). Recommend: out of scope for v0.1 — handle via Notifications subscription configuration.

---

## 11. Migration notes (from Fireproof)

- Fireproof's `sfdc/` module (~2,140 LOC) migrates directly. Entity shapes preserved; minor renames for platform consistency.
- Fireproof's `BarcodeResolution` (a utility type, not a persistent record) becomes the server-side logic invoked by the scan endpoint. Tenant-specific resolution rules move to `barcode_format_rules`.
- Fireproof's `LaborRecord.labor_number` becomes `labor_number` — allocated via platform Numbering module.
- Sample data — drop Fireproof tables, create platform schema, Fireproof rewires to typed client (`platform_client_shop_floor_data::*`).
- Production's time-entry table may need a migration path to source from SFDC events instead of storing independently. Out of scope for the initial shop-floor-data bead; handled as a follow-up Production refactor bead.
