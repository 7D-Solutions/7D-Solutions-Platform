# Corrective-Action Module — Scope, Boundaries, Contracts (draft v0.1)

> **NOT PLATFORM SCOPE (2026-04-16).** User ruled the full QMS cluster stays in Fireproof because other verticals (HuberPower, TrashTech, RanchOrbit) will not use the ISO 9001 / AS9100 formal workflow. This draft is retained for historical reference only. See `docs/plans/bd-ixnbs-fireproof-platform-migration.md`.

**7D Solutions Platform**
**Status:** Retired draft — not being built on platform
**Date:** 2026-04-16
**Proposed module name:** `corrective-action`
**Source of migration:** Fireproof ERP (`capa/` module)
**Pairs with:** `nonconformance` (subscribes to its events)

---

## 1. Mission & Non-Goals

### Mission
The Corrective-Action module is the **authoritative system for Corrective and Preventive Action (CAPA) lifecycles**: capturing root cause analysis, planning/implementing fixes, verifying implementation, and reviewing effectiveness over time. It owns the record of how we respond to problems (reactive) and how we prevent potential problems (proactive).

### Non-Goals
Corrective-Action does **NOT**:
- Own nonconformance records themselves (delegated to Nonconformance module — CAPA references NCR IDs, doesn't duplicate NCR data)
- Own audit findings (delegated to a future Internal-Audit module inside `quality-governance` cluster)
- Own risk register entries (delegated to Risk-Register inside `quality-governance`)
- Enforce AS9100-specific effectiveness timing windows or documentation requirements (Fireproof overlay)
- Send effectiveness-review reminders (delegated to Notifications module; Corrective-Action emits "review due" events, Notifications fires the email/SMS)
- Post to GL (no financial side effects at CAPA level)

---

## 2. Domain Authority

Corrective-Action is the **source of truth** for:

| Domain Entity | Authority |
|---------------|-----------|
| **CAPA records** | CAPA lifecycle (open → investigation → planning → implementation → verification → effectiveness_review → closed), type (corrective/preventive), title, description, owner, assignee |
| **Root Cause Analysis** | Narrative RCA text, optional structured-RCA fields (5-why, fishbone — future) |
| **Action Plans** | Corrective action plan (text), preventive action plan (text), containment-action narrative (text description of what was done — references Nonconformance containment_action_id if there is one) |
| **Effectiveness Reviews** | Effectiveness-verified flag, verifier, verification timestamp, notes, effectiveness_review_due date |
| **CAPA Source Links** | Reference to source entity: NCR / audit finding / risk item / customer complaint / internal escalation |

Corrective-Action is **NOT** authoritative for:
- The underlying nonconformance (Nonconformance module owns that)
- The containment actions themselves (Nonconformance module owns those — CAPA just describes what was done in narrative form)
- AS9100 effectiveness window rules (Fireproof overlay)

---

## 3. Data Ownership

All tables include `tenant_id`. Shared-DB model, row-level isolation per platform standard.

| Table | Purpose | Key fields |
|-------|---------|-----------|
| **capas** | CAPA records | `id`, `tenant_id`, `capa_number`, `capa_type` (canonical: corrective/preventive), `title`, `description`, `status` (canonical: open/investigation/planning/implementation/verification/effectiveness_review/closed), `owner_id`, `assigned_to`, `source_entity_type` (ncr/audit_finding/risk_item/customer_complaint/internal_escalation), `source_entity_id`, `root_cause_analysis`, `containment_narrative`, `corrective_action`, `preventive_action`, `due_date`, `closed_at`, `effectiveness_review_due`, `effectiveness_verified`, `effectiveness_verified_by`, `effectiveness_verified_at`, `effectiveness_notes`, `created_by`, `created_at`, `updated_at` |
| **capa_status_history** | Append-only transition log | `id`, `tenant_id`, `capa_id`, `from_status`, `to_status`, `transitioned_by`, `transitioned_at`, `notes` |
| **capa_status_labels** | Per-tenant display labels over canonical status set | `id`, `tenant_id`, `canonical_status`, `display_label`, `description`, `updated_at`, `updated_by` — unique on (`tenant_id`, `canonical_status`) |
| **capa_type_labels** | Per-tenant display labels for type (corrective/preventive) | Same shape as `capa_status_labels` |

**Multi-tenancy:** shared DB + `tenant_id` column.

---

## 4. OpenAPI Surface

### 4.1 CAPA Endpoints
- `POST /api/corrective-action/capas` — Create CAPA (body carries source_entity_type + source_entity_id)
- `GET /api/corrective-action/capas/:id` — Retrieve CAPA (response includes both canonical status and tenant display label)
- `PUT /api/corrective-action/capas/:id` — Update narrative fields (RCA, action plans, assignee, due date); status transitions go through dedicated endpoint
- `POST /api/corrective-action/capas/:id/transition` — Advance status (body: target canonical status, notes)
- `POST /api/corrective-action/capas/:id/verify-effectiveness` — Record effectiveness verification (body: verified_by, notes)
- `POST /api/corrective-action/capas/:id/close` — Close CAPA (requires effectiveness_verified = true)
- `GET /api/corrective-action/capas` — List with filters: status, capa_type, source_entity_type, owner_id, due_before, etc.

### 4.2 Status/Type Label Endpoints
- `GET /api/corrective-action/status-labels` — List tenant's display labels for all canonical statuses
- `PUT /api/corrective-action/status-labels/:canonical` — Set/update tenant display label (role-gated)
- `GET /api/corrective-action/type-labels` — Same for type
- `PUT /api/corrective-action/type-labels/:canonical` — Same for type

---

## 5. Events Produced & Consumed

Platform envelope: `event_id`, `occurred_at`, `tenant_id`, `source_module` (= `"corrective-action"`), `source_version`, `correlation_id`, `causation_id`, `payload`.

### 5.1 Events Produced

| Event name | Trigger | Key payload |
|------------|---------|-------------|
| `corrective_action.capa.opened.v1` | CAPA created | `capa_id`, `capa_number`, `capa_type`, `source_entity_type`, `source_entity_id`, `owner_id`, `due_date` |
| `corrective_action.capa.status_changed.v1` | Status transition | `capa_id`, `from_status`, `to_status`, `transitioned_by`, `transitioned_at` |
| `corrective_action.capa.effectiveness_verified.v1` | Effectiveness verification recorded | `capa_id`, `verified_by`, `verified_at` |
| `corrective_action.capa.closed.v1` | CAPA closed | `capa_id`, `closed_at`, `effectiveness_verified` |
| `corrective_action.capa.effectiveness_review_due.v1` | Daily sweep: `effectiveness_review_due < now()` and not yet verified | `capa_id`, `capa_number`, `owner_id`, `assigned_to`, `review_due`, `days_overdue` |
| `corrective_action.capa.overdue.v1` | Daily sweep: `due_date < now()` and status not in (`verification`, `effectiveness_review`, `closed`) | `capa_id`, `owner_id`, `due_date`, `current_status`, `days_overdue` |

### 5.2 Events Consumed

| Event name | Source | Behavior |
|------------|--------|----------|
| `nonconformance.ncr.dispositioned.v1` | Nonconformance | Optionally auto-open a CAPA when severity = major/critical AND disposition != `use_as_is` (configurable per tenant via flag table — future; initial impl requires manual CAPA creation) |
| `nonconformance.ncr.closed.v1` | Nonconformance | If a CAPA references this NCR and CAPA is still open past `effectiveness_review_due`, surface in reporting (no state change) |
| `internal_audit.finding.raised.v1` | Internal-Audit (future, `quality-governance` cluster) | Optionally auto-open a preventive CAPA linked to the finding (configurable) |

---

## 6. State Machines

### 6.1 CAPA Lifecycle
```
open ──> investigation ──> planning ──> implementation ──> verification ──> effectiveness_review ──> closed
                                                                                    │
                                                                                    └──> implementation (if review rejects)
```
Terminal: `closed`. Cannot reopen a closed CAPA — create a new CAPA that references the closed one.

**Forbidden transitions:**
- Skip forward past `verification` into `closed` (effectiveness review is mandatory)
- Reopen a `closed` CAPA
- Move to `effectiveness_review` without `effectiveness_review_due` set

---

## 7. Security & Tenant Isolation

- Shared DB, row-level isolation by `tenant_id`.
- Role gates: `corrective_action:transition`, `corrective_action:effectiveness:verify`, `corrective_action:close`, `corrective_action:labels:edit`.
- No PII beyond user IDs. No PCI, no financial data.

---

## 8. Required Invariants

1. **CAPA must reference a source entity.** `source_entity_type` and `source_entity_id` are both non-null. NCR is the common case but not the only one.
2. **Status transitions are gated by `allowed_transitions`.** No skip-ahead to `closed`.
3. **`effectiveness_verified` cannot be true without `effectiveness_verified_by` and `effectiveness_verified_at`.** Enforced at DB level via CHECK constraint.
4. **Closure requires effectiveness verification.** `close` endpoint rejects if `effectiveness_verified = false`.
5. **Status history is append-only.** No updates or deletes on `capa_status_history`.
6. **Tenant isolation cross-table.** `capa.tenant_id = capa_status_history.tenant_id` on any join.
7. **Canonical status and type sets are platform-owned.** Tenants can rename via `capa_status_labels` / `capa_type_labels`; cannot add or remove canonical values. Workflow changes go through vertical overlay.
8. **Events carry canonical status/type only.** No tenant display labels in event payloads.

---

## 9. What stays in Fireproof (AS9100 overlay)

Fireproof runs an overlay service that:
- Subscribes to `corrective_action.capa.opened.v1` and `.status_changed.v1`
- Stores AS9100-specific metadata in its own tables (e.g. `fireproof_capa_as9100`): AS9100 clause reference, effectiveness-review window per clause (AS9100 has stricter timing than ISO 9001), customer notification status if the finding came from an external audit, specific AS9102 linkage if applicable
- Enforces AS9100 effectiveness timing rules at the overlay level (e.g. warn if effectiveness review is overdue by > 30 days per AS9100 guidance)
- Emits Fireproof-specific events that TrashTech/HuberPower/RanchOrbit don't need

Platform Corrective-Action never sees these fields. The platform module is complete and correct for ISO 9001 without them.

---

## 10. Open questions

- **Auto-open CAPA from NCR.** Should platform implement the "auto-open CAPA on major/critical NCR disposition" workflow, or keep it manual in v0.1 and let verticals implement as overlays? Recommend: manual in v0.1 for simplicity; add a per-tenant config table in a later version after real customer demand. Keeps the initial module smaller.
- **Structured RCA (5-why, fishbone).** Fireproof today stores RCA as free text. Platform could add structured-RCA tables. Defer unless verticals ask for it.
- **CAPA-to-CAPA linkage.** When a closed CAPA doesn't solve the problem and a new one is opened, should there be a formal link? Recommend: add `related_capa_id` column now (cheap), don't build link management UI until needed.
- **Cross-module effectiveness review.** If a CAPA's effectiveness is being tested by "no recurrence for 90 days," what measures recurrence? Today that's a Nonconformance query — CAPA would have to ask "are there new NCRs on the same part/process since verification_at?" Defer to v0.2; stub the endpoint but don't implement cross-module query.

---

## 11. Migration notes (from Fireproof)

- Fireproof's `capa/` module (~1,250 LOC) consolidates into this platform module.
- Fireproof's `containment_action` TEXT column becomes `containment_narrative` here (renamed for clarity — the Nonconformance module owns real containment action records; this is just the narrative description).
- Fireproof's required `ncr_id` becomes `source_entity_type` + `source_entity_id` with NCR being one of several valid source types. Existing Fireproof rows migrate as `source_entity_type = 'ncr'`, `source_entity_id = <old ncr_id>`.
- Fireproof's audit-finding CAPA link (migration 000122) maps to `source_entity_type = 'audit_finding'`.
- Sample data only — no ETL; rebuild schema fresh on platform.
