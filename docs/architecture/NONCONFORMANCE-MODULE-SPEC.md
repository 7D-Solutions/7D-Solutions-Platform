# Nonconformance Module — Scope, Boundaries, Contracts (draft v0.1)

> **NOT PLATFORM SCOPE (2026-04-16).** User ruled the full QMS cluster (NCR, CAPA, concession, containment, MRB, internal audit, management review, etc.) stays in Fireproof because other verticals (HuberPower, TrashTech, RanchOrbit) will not use the ISO 9001 / AS9100 formal workflow. This draft is retained for historical reference only. See `docs/plans/bd-ixnbs-fireproof-platform-migration.md`.

**7D Solutions Platform**
**Status:** Retired draft — not being built on platform
**Date:** 2026-04-16
**Proposed module name:** `nonconformance`
**Source of migration:** Fireproof ERP (`ncr/`, `concession/`, `containment/`) + MRB disposition currently inside `ncr/`

---

## 1. Mission & Non-Goals

### Mission
The Nonconformance module is the **authoritative system for tracking quality nonconformances, disposition decisions, concessions, containment actions, and return-merchandise lifecycles** across all verticals. It owns the record of what went wrong, what we're doing about it, and how the affected material is contained or returned.

### Non-Goals
Nonconformance does **NOT**:
- Own corrective/preventive action lifecycles (delegated to a future `corrective-action` module — CAPA)
- Own First Article Inspection form shape (stays in Fireproof as AS9102-specific)
- Own AS9100 clause mapping, effectiveness tracking, or aerospace-specific disposition codes (Fireproof overlay)
- Own customer/regulator notification workflows (delegated to Notifications module; Fireproof orchestrates)
- Perform inspection itself (delegated to Quality-Inspection module)
- Create production rework work orders (delegated to Production module; Nonconformance emits an event, Production acts)
- Post to GL (delegated to GL via `gl.posting.requested` events when scrap-write-off is material)

---

## 2. Domain Authority

Nonconformance is the **source of truth** for:

| Domain Entity | Authority |
|---------------|-----------|
| **Nonconformance Records (NCRs)** | Record lifecycle (raised → under_review → dispositioned → closed), severity, description, source entity link, affected part/lot/quantity, reported-by, root cause field |
| **Disposition Decisions** | Disposition type (rework/scrap/use_as_is/return_to_supplier), approval chain, timestamps |
| **Concessions** | Customer-accepted deviations from spec: lifecycle, amount, expiry, approvals |
| **Containment Actions** | Quarantine of affected entities (lots, batches, work orders), affected-entity enumeration, review/release decisions |
| **RMA Records** | Return Merchandise Authorization lifecycle: pending → authorized → shipped → received → evaluated → closed |

Nonconformance is **NOT** authoritative for:
- Corrective action effectiveness (CAPA module owns this)
- Aerospace-specific disposition codes beyond the generic five (Fireproof overlay owns this)
- Physical part/lot inventory state (Inventory module owns this; Nonconformance just references lot IDs)

---

## 3. Data Ownership

All tables include `tenant_id` (shared-DB model, row-level isolation per platform standard). Every query filters by `tenant_id`.

| Table | Purpose | Key fields |
|-------|---------|-----------|
| **ncrs** | Nonconformance records | `id`, `tenant_id`, `ncr_number`, `title`, `description`, `severity` (minor/major/critical), `status` (open/under_review/dispositioned/closed), `source_entity_type`, `source_entity_id`, `part_number`, `part_revision`, `lot_id`, `quantity_affected`, `reported_by`, `reported_at`, `root_cause`, `closed_at` |
| **ncr_dispositions** | Disposition decisions per NCR | `id`, `tenant_id`, `ncr_id`, `disposition` (canonical: pending/rework/scrap/use_as_is/return_to_supplier/completed), `decided_by`, `decided_at`, `approval_chain_json`, `notes` |
| **disposition_labels** | Per-tenant display labels over the canonical set | `id`, `tenant_id`, `canonical_disposition`, `display_label`, `description`, `updated_at`, `updated_by` — unique on (`tenant_id`, `canonical_disposition`) |
| **ncr_rma** | RMA lifecycle for supplier-returns | `id`, `tenant_id`, `ncr_id`, `rma_number`, `status` (pending/authorized/shipped/received/evaluated/closed), `supplier_ref`, `tracking_number`, `shipped_at`, `received_at` |
| **rework_work_orders** | Link records (not the WO itself — that's in Production) | `id`, `tenant_id`, `ncr_id`, `work_order_id` (opaque ref to Production), `status`, `created_at` |
| **concessions** | Customer-accepted deviations | `id`, `tenant_id`, `concession_number`, `title`, `description`, `status` (draft/submitted/approved/rejected/expired), `requested_by`, `approved_by`, `approved_at`, `expiry_at`, `customer_ref` |
| **containment_actions** | Quarantine records | `id`, `tenant_id`, `ncr_id` (nullable — not all containment originates in NCR), `initiated_by`, `initiated_at`, `status` (active/under_review/released/escalated), `review_decision`, `reviewed_by`, `reviewed_at` |
| **containment_affected_entities** | What's quarantined | `id`, `tenant_id`, `containment_action_id`, `entity_type` (lot/batch/work_order/shipment), `entity_id`, `discovered_at`, `released_at` |

**Monetary precision:** no monetary fields owned here. Scrap write-off amounts come from Inventory when valuation is needed.

**Multi-tenancy:** shared DB + `tenant_id` column, matching AR/AP/Inventory pattern — not database-per-tenant.

---

## 4. OpenAPI Surface

### 4.1 NCR Endpoints
- `POST /api/nonconformance/ncrs` — Create NCR in `open` state
- `GET /api/nonconformance/ncrs/:id` — Retrieve NCR with latest disposition + any active containment
- `PUT /api/nonconformance/ncrs/:id` — Update NCR (title, description, root_cause, severity while open)
- `POST /api/nonconformance/ncrs/:id/submit-for-review` — Transition `open → under_review`
- `POST /api/nonconformance/ncrs/:id/close` — Transition `dispositioned → closed` (requires disposition)
- `GET /api/nonconformance/ncrs` — List NCRs (query: `status`, `severity`, `source_entity_type`, `part_number`, `reported_after/before`, paging)

### 4.2 Disposition Endpoints
- `POST /api/nonconformance/ncrs/:id/dispositions` — Record disposition decision (body carries canonical code)
- `GET /api/nonconformance/ncrs/:id/dispositions` — List disposition history (append-only audit trail; response includes both canonical code and tenant display label)
- `GET /api/nonconformance/disposition-labels` — List the tenant's display labels for all canonical dispositions
- `PUT /api/nonconformance/disposition-labels/:canonical` — Set/update tenant display label for one canonical disposition (role-gated)

### 4.3 RMA Endpoints
- `POST /api/nonconformance/ncrs/:id/rma` — Create RMA linked to NCR
- `POST /api/nonconformance/rma/:id/transition` — Move through RMA states
- `GET /api/nonconformance/rma/:id` — Retrieve RMA

### 4.4 Concession Endpoints
- `POST /api/nonconformance/concessions` — Create concession (draft)
- `POST /api/nonconformance/concessions/:id/submit` — Submit for approval
- `POST /api/nonconformance/concessions/:id/approve` — Approve (role-gated)
- `POST /api/nonconformance/concessions/:id/reject` — Reject
- `GET /api/nonconformance/concessions` — List with filters

### 4.5 Containment Endpoints
- `POST /api/nonconformance/containment` — Initiate containment (optionally linked to NCR)
- `POST /api/nonconformance/containment/:id/affected` — Add affected entities
- `POST /api/nonconformance/containment/:id/review` — Review and release/escalate
- `GET /api/nonconformance/containment/:id` — Retrieve containment with affected list

---

## 5. Events Produced & Consumed

All events use the platform envelope: `event_id`, `occurred_at`, `tenant_id`, `source_module` (= `"nonconformance"`), `source_version`, `correlation_id`, `causation_id`, `payload`.

### 5.1 Events Produced

| Event name | Trigger | Key payload |
|------------|---------|-------------|
| `nonconformance.ncr.raised.v1` | NCR created | `ncr_id`, `ncr_number`, `severity`, `source_entity_type/id`, `part_number`, `lot_id`, `quantity_affected` |
| `nonconformance.ncr.under_review.v1` | NCR transitioned to under_review | `ncr_id` |
| `nonconformance.ncr.dispositioned.v1` | Disposition decision recorded | `ncr_id`, `disposition`, `decided_by` |
| `nonconformance.ncr.closed.v1` | NCR closed | `ncr_id`, `closed_at` |
| `nonconformance.rework.requested.v1` | Disposition = rework → requests Production to create WO | `ncr_id`, `part_number`, `lot_id`, `quantity` |
| `nonconformance.scrap.requested.v1` | Disposition = scrap → requests Inventory write-off | `ncr_id`, `part_number`, `lot_id`, `quantity` |
| `nonconformance.rma.authorized.v1` | RMA moved to authorized | `rma_id`, `ncr_id`, `supplier_ref` |
| `nonconformance.rma.closed.v1` | RMA closed | `rma_id`, `outcome` |
| `nonconformance.concession.submitted.v1` | Concession submitted | `concession_id`, `customer_ref` |
| `nonconformance.concession.approved.v1` | Concession approved | `concession_id`, `approved_by`, `expiry_at` |
| `nonconformance.containment.initiated.v1` | Containment started | `containment_id`, `ncr_id`, `affected_count` |
| `nonconformance.containment.released.v1` | Containment review released entities | `containment_id`, `entity_refs` |

### 5.2 Events Consumed

| Event name | Source | Behavior |
|------------|--------|----------|
| `quality_inspection.inspection.failed.v1` | Quality-Inspection | Optionally auto-raise NCR (configurable per tenant) |
| `production.work_order.rework_created.v1` | Production | Link back to NCR's rework_work_orders row |
| `production.work_order.completed.v1` | Production | If tied to rework for NCR, update NCR status |
| `inventory.lot.write_off.completed.v1` | Inventory | Mark scrap disposition complete |

---

## 6. State Machines

### 6.1 NCR Lifecycle
```
open ──> under_review ──> dispositioned ──> closed
         │                                     ↑
         └──> dispositioned (skip review for minor) ┘
```
Terminal: `closed`. No reopen — create a follow-up NCR instead.

### 6.2 Disposition Lifecycle (per-disposition record)
```
pending ──┬──> rework
          ├──> scrap
          ├──> use_as_is
          └──> return_to_supplier ──> completed (after RMA closes)
```

### 6.3 RMA Lifecycle
```
pending ──> authorized ──> shipped ──> received ──> evaluated ──> closed
```

### 6.4 Concession Lifecycle
```
draft ──> submitted ──┬──> approved ──> expired (on expiry_at)
                      └──> rejected
```

### 6.5 Containment Lifecycle
```
active ──> under_review ──┬──> released
                          └──> escalated (promotes to new NCR)
```

---

## 7. Security & Tenant Isolation

- Shared DB, row-level isolation by `tenant_id` (every table, every query).
- JWT-derived tenant from `VerifiedClaims`.
- Role gates: disposition approval, concession approval, containment review are permission-guarded (e.g. `nonconformance:disposition:approve`, `nonconformance:concession:approve`).
- All events carry `tenant_id`.
- No PII beyond reporter/approver user IDs. No PCI, no financial data.

---

## 8. Required Invariants

1. **Disposition requires NCR under_review or dispositioned.** Cannot disposition a closed NCR.
2. **RMA must link to an NCR.** RMAs don't exist standalone.
3. **Containment affected entities are append-only until review.** Can add; cannot remove, only release.
4. **Concession expiry is enforced by event.** A daily sweep emits `concession.expired.v1` when `expiry_at < now()`.
5. **Tenant isolation cross-table.** `ncr.tenant_id = disposition.tenant_id = rma.tenant_id = containment.tenant_id` for any joined records.
6. **Disposition history is append-only.** Corrections create a new disposition row, not edits.
7. **NCR closure requires disposition.** Cannot close an NCR without at least one completed disposition.
8. **Canonical disposition set is platform-owned, not tenant-configurable.** Tenants can rename display labels via `disposition_labels`; they cannot add, remove, or reroute the canonical codes. Genuinely new disposition workflows (not just renames) are handled by vertical overlays, not platform config.
9. **Events carry canonical disposition only.** `nonconformance.ncr.dispositioned.v1` and related events emit the canonical code (e.g. `scrap`), never the tenant display label. Downstream platform modules (Inventory, Production) match on canonical code.

---

## 9. What stays in Fireproof (AS9100 overlay)

Fireproof runs its own `nonconformance-overlay` service (or equivalent) that:
- Subscribes to `nonconformance.ncr.raised.v1` and `nonconformance.ncr.dispositioned.v1`
- Stores AS9100-specific overlay data in its own tables (e.g. `fireproof_ncr_as9100`): AS9100 clause references, aerospace-specific disposition codes beyond the generic five, effectiveness review state, FAA/DCMA notification status, AS9102 FAI linkage
- Exposes Fireproof-specific UI over those fields
- Emits Fireproof-specific events (e.g. `fireproof.ncr.as9100_effectiveness_verified`) that TrashTech/HuberPower/RanchOrbit never need

Platform Nonconformance never sees or depends on these overlay fields. The platform module is complete and correct without them.

---

## 10. Open questions

- **Source-entity ref generality.** Fireproof's NCR has `source_entity_type` as a free-text varchar. For platform, should this be a constrained enum (`inspection`/`receiving`/`work_order`/`customer_complaint`/`supplier_finding`/`internal_audit`) or stay open? Favor constrained for cross-vertical consistency.
- **Containment without NCR.** Fireproof allows containment initiated from an audit finding before any NCR is raised. Keep `containment_actions.ncr_id` nullable, or require NCR first? Keep nullable; matches real practice.
- **Rework WO creation mechanism.** Event-driven ("nonconformance emits request, production responds") or synchronous HTTP call from Nonconformance → Production? Pattern elsewhere on platform is event-driven; stick with that.
- **Concession vs disposition overlap.** A concession (customer accepts deviation) is effectively a disposition of `use_as_is` with external approval. Keep separate tables for clarity, or unify? Keep separate — different approval chains, different audit needs.

---

## 11. Migration notes (from Fireproof)

- Fireproof's `ncr/` module (~2,100 LOC), `concession/` module, `containment/` module, and MRB disposition logic currently inside `ncr/` consolidate into this platform module.
- Fireproof's data is sample only — no ETL. Drop Fireproof's tables, create this module's schema fresh, rewrite Fireproof's callers to use the typed client (`platform_client_nonconformance::*`).
- AS9100-specific fields on Fireproof's current `ncrs` table (e.g. `as9100_clause`, `effectiveness_verified`) move to the Fireproof overlay service's own tables.
