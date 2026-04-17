# Platform Extensions — Additions to Existing Modules (draft v0.1)

**7D Solutions Platform**
**Status:** Draft Specification — bd-ixnbs migration
**Date:** 2026-04-16
**Related plan:** `docs/plans/bd-ixnbs-fireproof-platform-migration.md`

This document covers the migration work that extends existing platform modules rather than creating new ones. Each section describes the additive surface — new tables, new endpoints, new events — without repeating the full module spec. Each extension is its own small bead during implementation.

---

## 1. BOM Extension — MRP Explosion

**Existing module:** `modules/bom/`
**Source of migration:** Fireproof ERP `mrp/` module (~small computation engine)
**Why this is an extension, not a new module:** MRP is fundamentally a BOM explosion with on-hand subtraction and scrap-factor application. BOM already owns the explosion computation; adding net-requirements output keeps related logic colocated.

### New tables

| Table | Purpose | Key fields |
|-------|---------|-----------|
| **mrp_snapshots** | Immutable record of each MRP run | `id`, `tenant_id`, `bom_id` (ref → BOM), `demand_quantity`, `effectivity_date`, `on_hand_snapshot` (JSONB — captures on-hand as of run time), `created_at`, `created_by` |
| **mrp_requirement_lines** | Per-component net requirement output | `id` (bigint), `snapshot_id`, `level` (int — BOM depth), `parent_part_id`, `component_item_id`, `gross_quantity`, `scrap_factor`, `scrap_adjusted_quantity`, `on_hand_quantity`, `net_quantity`, `uom`, `revision_id`, `revision_label` |

### New endpoints
- `POST /api/bom/mrp/explode` — Run MRP: body `{bom_id, demand_quantity, effectivity_date, on_hand: [{item_id, quantity}]}`. Returns `{snapshot: MrpSnapshot, lines: [MrpRequirementLine]}`. Writes a snapshot row for auditability.
- `GET /api/bom/mrp/snapshots/:id` — Retrieve a historical snapshot with its lines.
- `GET /api/bom/mrp/snapshots` — List snapshots (filters: bom_id, created_after, created_by).

### New events
- `bom.mrp.exploded.v1` — Emitted on each successful explosion: `snapshot_id`, `bom_id`, `demand_quantity`, `line_count`, `net_shortage_count` (how many components net > 0).

### Notes
- The `on_hand` input is **caller-supplied** (not queried from Inventory automatically). Keeps the computation deterministic and auditable. Callers that want fresh on-hand query Inventory first and pass the snapshot.
- Quantities: Fireproof uses `f64`; platform keeps `f64` here since MRP is a computation over quantities (not monetary amounts — no precision concern that integer cents solves). Scrap factors are fractional.
- Lead-time-based time-phased MRP is **out of scope for v0.1.** Current explosion is single-point (demand at time T, requirements computed now). Time-phased MRP (scheduling requirements across a horizon based on lead times) is a future bead if demand emerges.

### Migration notes
- Fireproof's `mrp/` module retires; Fireproof wires to `platform_client_bom::MrpClient` (new sub-client under BOM).
- Sample data only — no ETL.

---

## 2. Inventory Extension — Barcode Resolution Service

**Existing module:** `modules/inventory/`
**Source of migration:** the barcode-resolution portion of Fireproof's `sfdc/` module (the rest — kiosks, sessions, kiosk-driven labor — stays in Fireproof as shop-specific hardware workflow)
**Why this is an Inventory extension:** Inventory already owns the primary entities referenced by shop-floor barcodes (items, lots, serials). Making barcode resolution a cross-cutting service on Inventory lets any module (Production, Shipping-Receiving, Quality-Inspection, Fireproof's local kiosk UI) call a single endpoint to turn a raw scanned string into a typed entity reference.

### New tables

| Table | Purpose | Key fields |
|-------|---------|-----------|
| **barcode_format_rules** | Tenant-configured parsing rules | `id`, `tenant_id`, `rule_name`, `pattern_regex`, `entity_type_when_matched` (canonical: work_order/operation/item/lot/serial/badge/other), `capture_group_index` (which regex group yields the entity ID/reference), `priority`, `active`, `created_at`, `updated_at`, `updated_by` |

### New endpoints
- `POST /api/inventory/barcode/resolve` — Body: `{barcode_raw}`. Response: `{resolved: boolean, entity_type, entity_id (UUID) OR entity_ref (string for non-UUID refs like WO numbers), matched_rule_id, resolution_error?}`. When `entity_type` is Inventory-owned (lot/serial/item), returns full UUID; for cross-module entities (work_order, operation), returns the string reference and the caller queries that module for details.
- `POST /api/inventory/barcode/resolve/batch` — Array body for bulk resolution.
- `POST /api/inventory/barcode/rules/test` — Test a barcode against current rules, returns which rule matched and parsed result.
- `GET /api/inventory/barcode/rules` — List tenant rules.
- `POST /api/inventory/barcode/rules` — Add rule.
- `PUT /api/inventory/barcode/rules/:id` — Update.
- `POST /api/inventory/barcode/rules/:id/deactivate` — Deactivate (preserves history).

### New events
- `inventory.barcode.resolved.v1` — For observability / audit: `barcode_raw`, `entity_type`, `resolved`, `matched_rule_id`, `resolved_by`. Optional subscription; platform does not require consumers.

### Notes
- Rules evaluated in priority order (lowest priority number first); first match wins.
- For Inventory-native entity types (item, lot, serial), the service both parses AND resolves to a verified existing record. For cross-module types (work_order, operation), the service parses and returns the reference; caller validates existence via Production's API.
- Callers include: Fireproof's local kiosk UI (scans at terminals), Shipping-Receiving (scans on packing slips), Quality-Inspection (scans to pull inspection plan), Production (scans for manual entries).

### Migration notes
- Fireproof's `BarcodeResolution` utility + SFDC's per-scan parsing logic migrates here.
- Fireproof's kiosk / session / labor-at-kiosk code stays local — it's the UX wrapper around the platform resolution service.

---

## 3. Production Extension — Manufacturing Costing

**Existing module:** `modules/production/`
**Source of migration:** Fireproof ERP `manufacturing_costing/` module (~800 LOC)
**Why this is an extension:** Production already owns work orders, operations, time entries, and workcenter cost rates. Manufacturing costing is cost accumulation on WOs using those existing inputs plus material issues and overhead. The composition engine belongs with the data it composes.

### New tables

| Table | Purpose | Key fields |
|-------|---------|-----------|
| **work_order_cost_postings** | Individual cost postings attributed to a WO | `id`, `tenant_id`, `work_order_id`, `operation_id` (nullable), `posting_category` (canonical: labor/material/overhead/outside_processing/scrap/other), `amount_cents`, `quantity` (nullable), `source_event_id` (nullable — link to the event that produced this posting), `posted_at`, `posted_by` |
| **work_order_cost_summaries** | Rolled-up cost per WO (derived; refreshed on posting) | `work_order_id` (PK), `tenant_id`, `total_cost_cents`, `labor_cost_cents`, `material_cost_cents`, `overhead_cost_cents`, `outside_processing_cost_cents`, `scrap_cost_cents`, `other_cost_cents`, `posting_count`, `last_updated_at` |

### New endpoints
- `POST /api/production/work-orders/:id/cost-postings` — Record a cost posting (typically called by event handlers, not directly by users)
- `GET /api/production/work-orders/:id/cost-summary` — Retrieve rolled-up cost
- `GET /api/production/work-orders/:id/cost-postings` — List individual postings
- `POST /api/production/work-orders/:id/close` — Close WO, finalize cost summary; emits WO closed event with final cost snapshot (this likely already exists on Production; extension adds cost-snapshot payload)

### New events
- `production.cost.posted.v1` — Individual posting recorded: `work_order_id`, `operation_id`, `posting_category`, `amount_cents`, `posted_at`
- `production.work_order.cost_finalized.v1` — On WO close, emits final cost summary for GL posting downstream: `work_order_id`, `total_cost_cents`, category breakdown, `closed_at`

### Consumed events (cost-posting triggers)
- `production.time_entry.approved.v1` → Production computes labor cost (duration × operator rate × workcenter cost rate) and posts
- `inventory.lot.issued.v1` → Material cost posting based on lot valuation
- `outside_processing.order.closed.v1` → OSP cost posting from the OP's actual_cost_cents
- Overhead: configurable allocation rules per tenant (time-based or material-based); deferred to v0.2

### Migration notes
- Fireproof's manufacturing_costing module retires; Fireproof wires to Production's cost endpoints.
- Fireproof's posting category values map 1:1 to canonical.
- GL posting events (from work_order.cost_finalized.v1) integrate with existing GL posting flow, similar to AR's pattern.

---

## 4. BOM Extension — Kit Readiness

**Existing module:** `modules/bom/`
**Source of migration:** Fireproof ERP `kit_readiness/` module (~890 LOC)
**Why this is an extension:** Kit readiness = BOM explosion × Inventory availability check. It's a computation, not a persistent state. The computation logic belongs with BOM since BOM explosion is the core operation.

### No new persistent tables required
Kit readiness is a compute endpoint that returns a result; it doesn't persist state (beyond optionally logging a snapshot similar to MRP).

Optionally for auditability:
| Table | Purpose |
|-------|---------|
| **kit_readiness_snapshots** | Record of each readiness check: `id`, `tenant_id`, `bom_id`, `required_quantity`, `check_date`, `overall_status` (canonical: ready/partial/not_ready), `issue_summary` (JSONB — per-component status), `created_at`, `created_by` |

### New endpoints
- `POST /api/bom/kit-readiness/check` — Check readiness: body `{bom_id, required_quantity, check_date, policy: { allow_expired, scrap_factor_application, …}}`. Returns per-component readiness (component_id, required_qty, on_hand_qty, expired_qty, available_qty, status: ready/short/expired/quarantined). Logs snapshot.
- `GET /api/bom/kit-readiness/snapshots/:id` — Retrieve historical snapshot

### New events
- `bom.kit_readiness.checked.v1` — Check performed: `snapshot_id`, `bom_id`, `overall_status`

### Notes
- Pulls on-hand from Inventory (uses Inventory's availability query) rather than taking it as input like MRP does. Kit readiness is an operational check ("am I ready to start this WO now?"), so fresh data is the right default.
- Policy knobs per tenant (allow expired, scrap factor application) configurable later; for v0.1, sensible defaults.

### Migration notes
- Fireproof's kit_readiness module retires; Fireproof wires to BOM's kit-readiness endpoint.

---

## 5. Workforce-Competence Extension — Training Delivery

**Existing module:** `platform/workforce-competence/` (already at port 8121 per catalog)
**Source of migration:** Fireproof ERP `hr_training/` module
**Why this is an extension:** Workforce-Competence already tracks competence artifacts (certifications, training records as artifact categories), assignments (operator → artifact with validity window), and acceptance authorities. What's missing is the training delivery layer — the act of planning a training event, assigning operators to it, and recording their completion.

### New tables

| Table | Purpose | Key fields |
|-------|---------|-----------|
| **training_plans** | A planned training event | `id`, `tenant_id`, `plan_code`, `title`, `description`, `artifact_code` (FK → competence_artifacts — what qualification this training confers), `duration_minutes`, `instructor_id` (nullable), `material_refs` (array of doc-mgmt refs), `location`, `scheduled_at` (nullable — training may be on-demand), `active`, `created_at`, `updated_at`, `updated_by` |
| **training_assignments** | Operator assigned to a training plan | `id`, `tenant_id`, `plan_id`, `operator_id`, `assigned_by`, `assigned_at`, `status` (canonical: assigned/scheduled/in_progress/completed/cancelled/no_show), `scheduled_at` (nullable — specific date for this operator), `notes`, `updated_at` |
| **training_completions** | Record of completion; triggers competence_assignment creation | `id`, `tenant_id`, `assignment_id`, `operator_id`, `plan_id`, `completed_at`, `verified_by`, `outcome` (canonical: passed/failed/incomplete), `notes`, `resulting_competence_assignment_id` (nullable — set when passing creates a competence_assignment via API call), `created_at` |

### New endpoints
- `POST /api/workforce-competence/training-plans` — Create plan
- `PUT /api/workforce-competence/training-plans/:id` — Update
- `GET /api/workforce-competence/training-plans/:id` — Retrieve
- `GET /api/workforce-competence/training-plans` — List
- `POST /api/workforce-competence/training-assignments` — Assign operators to a plan
- `POST /api/workforce-competence/training-assignments/:id/transition` — Advance assignment status
- `POST /api/workforce-competence/training-completions` — Record completion; if outcome=passed, auto-creates a competence_assignment via the existing assignment endpoint
- `GET /api/workforce-competence/training-assignments` — List
- `GET /api/workforce-competence/training-completions` — List

### New events
- `workforce_competence.training.planned.v1`
- `workforce_competence.training.assigned.v1`
- `workforce_competence.training.completed.v1` — includes outcome and whether a competence_assignment was created

### Migration notes
- Fireproof's hr_training module migrates: TrainingPlan → training_plans, TrainingAssignment → training_assignments, TrainingCompletion → training_completions.
- Fireproof's Qualification and EmployeeQualification tables **do not migrate** — they duplicate platform's existing competence_artifacts and assignments. Fireproof rewires existing data to platform's existing tables.
- Fireproof's QualificationTrainingRequirement (links a qualification to required trainings) becomes a new link table `competence_artifact_training_requirements` in workforce-competence, or can be modeled as an array column on `training_plans.required_for_artifact_codes`. Decision in implementation bead.

---

## 6. AP Extension — Supplier Eligibility & Qualification

**Existing module:** `modules/ap/`
**Source of migration:** Fireproof ERP `procurement/` module (the SupplierEligibility portion only — POs and Receipts are already covered by AP and Shipping-Receiving, not migrating)
**Why this is an extension:** AP already owns vendors. Supplier qualification/eligibility is a small addition to vendor management — a qualification state that gates PO creation.

### New tables or existing-table additions

Add to existing `vendors` table:
- `qualification_status` (canonical: unqualified/pending_review/qualified/restricted/disqualified)
- `qualification_notes`
- `qualified_by` (nullable)
- `qualified_at` (nullable)
- `preferred_vendor` (boolean, default false)

And new table:
| Table | Purpose | Key fields |
|-------|---------|-----------|
| **vendor_qualification_events** | Audit log of qualification state changes | `id`, `tenant_id`, `vendor_id`, `from_status`, `to_status`, `reason`, `changed_by`, `changed_at` |

### New endpoints
- `POST /api/ap/vendors/:id/qualify` — Set qualification status (body: target canonical status, reason, notes)
- `POST /api/ap/vendors/:id/mark-preferred` / `POST /api/ap/vendors/:id/unmark-preferred`
- `GET /api/ap/vendors/:id/qualification-history` — Audit trail
- Existing `GET /api/ap/vendors` adds filter param: `qualification_status`, `preferred_only`

### New events
- `ap.vendor.qualified.v1` — status transition (typically on first entering qualified)
- `ap.vendor.disqualified.v1` — status transition to disqualified
- `ap.vendor.qualification_changed.v1` — any status transition (includes from/to)

### Enforcement
- PO creation endpoint (`POST /api/ap/pos`) refuses when `vendor.qualification_status in (unqualified, disqualified)`. Adds role-gated override (`ap:po:create_without_qualification`) for exceptional cases, logged.
- `restricted` status allows PO creation but flags it for review.

### Migration notes
- Fireproof's procurement module retires. Sample data on supplier eligibility migrates to AP's new vendor qualification columns.
- Fireproof's own POs + Receipts in `procurement/` were duplicates of AP's POs + Shipping-Receiving's receipts — they retire entirely. Fireproof rewires all supplier-facing flows to use AP + Shipping-Receiving.

---

## Summary of extension work

Six extensions across five existing modules:

| Module | Extension | Est. scope |
|--------|-----------|-----------|
| BOM | MRP explosion | 2 new tables, 3 endpoints, 1 event |
| BOM | Kit readiness | 0-1 new table, 2 endpoints, 1 event |
| Inventory | Barcode resolution service | 1 new table, 6 endpoints, 1 event |
| Production | Manufacturing costing | 2 new tables, 4 endpoints, 2 events, 3 consumed events |
| Workforce-Competence | Training delivery | 3 new tables, 8 endpoints, 3 events |
| AP | Supplier eligibility | Additive columns on vendors + 1 audit table, 3 endpoints, 3 events |

Each extension is an independent bead during implementation. Fireproof's corresponding source modules retire once the extension lands and Fireproof wires to the platform client.
