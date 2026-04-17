# Shop-Floor-Gates Module — Scope, Boundaries, Contracts (draft v0.1)

**7D Solutions Platform**
**Status:** Draft Specification — bd-ixnbs migration
**Date:** 2026-04-16
**Proposed module name:** `shop-floor-gates`
**Source of migration:** Fireproof ERP — `traveler_hold/` (~1,050 LOC), `operation_handoff/` (~910 LOC), `operation_start_verification/` (~1,760 LOC), `signoff/` (~530 LOC)
**Cross-vertical applicability:** Fireproof + HuberPower (any manufacturer running multi-operation work orders)

---

## 1. Mission & Non-Goals

### Mission
Shop-Floor-Gates is the **authoritative system for the gating and state-transition concerns that sit on top of work orders and operations**: holds that pause work, setup verifications before an operation runs, handoffs between operations, and signoff attestations at control points. All four are operational controls — they govern *whether* and *how* work flows through the shop, not the work definition itself (Production owns that).

### Non-Goals
Shop-Floor-Gates does **NOT**:
- Own work orders, operations, or routings (delegated to Production — Gates references `work_order_id` and `operation_id`)
- Own inspection logic (delegated to Quality-Inspection — a hold may reference an inspection that triggered it)
- Own nonconformance records (NCR stays in Fireproof — a hold with `hold_type = 'quality'` on a Fireproof tenant may correspond to a Fireproof NCR, but platform Gates doesn't link to NCR since NCR isn't platform scope)
- Execute labor time capture or kiosk interactions (Production owns time entries; kiosk UX stays in the vertical)
- Execute CAPA workflow when a hold reveals a systemic problem (CAPA stays in Fireproof)
- Send notifications (delegated to Notifications — Gates emits events)

---

## 2. Domain Authority

Shop-Floor-Gates is the **source of truth** for:

| Domain Entity | Authority |
|---------------|-----------|
| **Traveler Holds** | Hold records against a work order or operation: hold_type, reason, status, placed/released/cancelled timestamps and actors, release authority rules |
| **Operation Handoffs** | Records of work passing from one operation to the next: source/dest operations, quantity, lot/serial, push vs pull pattern, status (initiated/accepted/rejected) |
| **Operation Start Verifications** | Pre-run checks before an operation begins work: drawing verified, material verified, instructions verified, operator confirmation, outcome |
| **Signoffs** | Attestation records at control points: polymorphic entity ref, role, signer, timestamp, context — scoped to shop-floor entity types only (work_order, operation, hold, handoff, verification) |

Shop-Floor-Gates is **NOT** authoritative for:
- The underlying work order or operation (Production owns)
- Drawing/instruction content (doc-mgmt owns)
- Lot state (Inventory owns; Gates just references lot_number)
- Inspection results (Quality-Inspection owns)

---

## 3. Data Ownership

All tables include `tenant_id`, shared-DB model. All signoffs append-only.

| Table | Purpose | Key fields |
|-------|---------|-----------|
| **traveler_holds** | Hold records | `id`, `tenant_id`, `hold_number`, `hold_type` (canonical: quality/engineering/material/customer/other), `scope` (canonical: work_order/operation), `work_order_id`, `operation_id` (non-null iff scope=operation), `reason`, `status` (canonical: active/released/cancelled), `placed_by`, `placed_at`, `release_authority` (canonical: quality/engineering/planner/supervisor/owner_only/any_with_role), `released_by`, `released_at`, `release_notes`, `cancelled_by`, `cancelled_at`, `cancel_reason`, `created_at`, `updated_at` |
| **operation_handoffs** | Operation → operation handoff records | `id`, `tenant_id`, `handoff_number`, `work_order_id`, `source_operation_id`, `dest_operation_id`, `initiation_type` (canonical: push/pull), `status` (canonical: initiated/accepted/rejected/cancelled), `quantity`, `unit_of_measure`, `lot_number`, `serial_numbers` (array nullable), `initiated_by`, `initiated_at`, `accepted_by`, `accepted_at`, `rejected_by`, `rejected_at`, `rejection_reason`, `cancelled_by`, `cancelled_at`, `cancel_reason`, `notes`, `created_at`, `updated_at` |
| **operation_start_verifications** | Pre-run checks | `id`, `tenant_id`, `verification_number`, `work_order_id`, `operation_id`, `status` (canonical: pending/verified/rejected), `drawing_number`, `drawing_revision`, `drawing_verified` (bool), `material_description`, `lot_number`, `serial_numbers` (array nullable), `material_verified` (bool), `instruction_reference`, `instruction_verified` (bool), `operator_id`, `operator_confirmed_at`, `verifier_id`, `verified_at`, `rejection_reason`, `notes`, `created_at`, `updated_at` |
| **signoffs** | Polymorphic attestation | `id` (bigint), `tenant_id`, `entity_type` (canonical whitelist: work_order/operation/traveler_hold/operation_handoff/operation_start_verification), `entity_id` (UUID as string — matches referenced entity's id), `role` (canonical: quality/engineering/supervisor/operator/planner/material — tenant may rename via labels), `signed_by`, `signer_name` (denormalized display for audit readability), `signed_at`, `action_context` (free text — what action is being attested), `created_at` |
| **hold_type_labels**, **hold_scope_labels**, **hold_release_authority_labels**, **hold_status_labels** | Tenant display labels | Standard label-table shape per canonical enum |
| **handoff_initiation_labels**, **handoff_status_labels** | Same for handoffs | Standard shape |
| **verification_status_labels** | Same for verifications | Standard shape |
| **signoff_role_labels** | Tenant display labels over signoff role | Standard shape |

**Note on `signoff.entity_type`:** tightened to a whitelist of shop-floor entity types only. Fireproof currently uses free text here (e.g. signs off NCRs via the signoff table); those NCR signoffs stay in Fireproof since NCR is Fireproof-only. Platform signoff focuses on shop-floor entities. If another platform module wants signed attestations (e.g. quality-inspection), that module either embeds its own signoff record or we extend the whitelist in a later version.

---

## 4. OpenAPI Surface

### 4.1 Traveler Hold Endpoints
- `POST /api/shop-floor-gates/holds` — Place a hold (body: hold_type, scope, work_order_id, operation_id if applicable, reason, release_authority)
- `POST /api/shop-floor-gates/holds/:id/release` — Release (role-gated by release_authority)
- `POST /api/shop-floor-gates/holds/:id/cancel` — Cancel (body: reason)
- `GET /api/shop-floor-gates/holds/:id` — Retrieve
- `GET /api/shop-floor-gates/holds` — List (filters: status, hold_type, work_order_id, placed_after)
- `GET /api/shop-floor-gates/work-orders/:wo_id/holds` — List holds on a WO
- `GET /api/shop-floor-gates/operations/:op_id/holds` — List holds on an operation

### 4.2 Operation Handoff Endpoints
- `POST /api/shop-floor-gates/handoffs` — Initiate (source op's operator pushes OR dest op pulls)
- `POST /api/shop-floor-gates/handoffs/:id/accept` — Dest accepts
- `POST /api/shop-floor-gates/handoffs/:id/reject` — Dest rejects (body: reason)
- `POST /api/shop-floor-gates/handoffs/:id/cancel` — Source cancels while still `initiated`
- `GET /api/shop-floor-gates/handoffs/:id` — Retrieve
- `GET /api/shop-floor-gates/handoffs` — List (filters)

### 4.3 Operation Start Verification Endpoints
- `POST /api/shop-floor-gates/verifications` — Create (pending)
- `POST /api/shop-floor-gates/verifications/:id/confirm` — Operator confirms all fields → pending; verifier then signs off to move to verified
- `POST /api/shop-floor-gates/verifications/:id/verify` — Verifier approves → verified
- `POST /api/shop-floor-gates/verifications/:id/reject` — Verifier rejects → rejected (body: reason)
- `GET /api/shop-floor-gates/verifications/:id` — Retrieve
- `GET /api/shop-floor-gates/verifications` — List (filters)

### 4.4 Signoff Endpoints
- `POST /api/shop-floor-gates/signoffs` — Record signoff (body: entity_type, entity_id, role, signer_name, action_context)
- `GET /api/shop-floor-gates/signoffs` — List (filters: entity_type, entity_id, role, signed_by, date ranges)
- `GET /api/shop-floor-gates/entities/:entity_type/:entity_id/signoffs` — List signoffs for a specific entity

### 4.5 Label Endpoints
- Standard per-canonical-field CRUD, matching earlier module specs.

---

## 5. Events Produced & Consumed

Platform envelope. `source_module` = `"shop-floor-gates"`.

### 5.1 Events Produced

| Event name | Trigger | Key payload |
|------------|---------|-------------|
| `shop_floor_gates.hold.placed.v1` | Hold placed | `hold_id`, `hold_number`, `hold_type`, `scope`, `work_order_id`, `operation_id`, `reason`, `placed_by` |
| `shop_floor_gates.hold.released.v1` | Hold released | `hold_id`, `released_by`, `released_at` |
| `shop_floor_gates.hold.cancelled.v1` | Hold cancelled | `hold_id`, `cancel_reason` |
| `shop_floor_gates.handoff.initiated.v1` | Handoff created | `handoff_id`, `work_order_id`, `source_operation_id`, `dest_operation_id`, `quantity`, `lot_number` |
| `shop_floor_gates.handoff.accepted.v1` | Handoff accepted | `handoff_id`, `accepted_by`, `accepted_at` |
| `shop_floor_gates.handoff.rejected.v1` | Handoff rejected | `handoff_id`, `rejection_reason`, `rejected_by` |
| `shop_floor_gates.verification.created.v1` | Verification created | `verification_id`, `work_order_id`, `operation_id` |
| `shop_floor_gates.verification.verified.v1` | Verified | `verification_id`, `verifier_id`, `verified_at` |
| `shop_floor_gates.verification.rejected.v1` | Rejected | `verification_id`, `rejection_reason`, `verifier_id` |
| `shop_floor_gates.signoff.recorded.v1` | Signoff recorded | `signoff_id`, `entity_type`, `entity_id`, `role`, `signed_by`, `action_context` |

### 5.2 Events Consumed

| Event name | Source | Behavior |
|------------|--------|----------|
| `production.work_order.cancelled.v1` | Production | Auto-cancel any active holds/handoffs/verifications on the WO (set cancel_reason = "work order cancelled") |
| `production.work_order.completed.v1` | Production | Auto-release non-terminal holds on the WO (log as "auto-released on WO completion") |
| `production.operation.completed.v1` | Production | If verification exists for that operation and is still pending, log warning (operation completed without verification) |

---

## 6. State Machines

### 6.1 Traveler Hold
```
active ──┬──> released
         └──> cancelled
```

### 6.2 Operation Handoff
```
initiated ──┬──> accepted
            ├──> rejected
            └──> cancelled
```

### 6.3 Operation Start Verification
```
pending ──┬──> verified
          └──> rejected (may create a new verification for retry)
```

### 6.4 Signoff
No state machine — signoff records are single-timestamp append-only attestations.

---

## 7. Security & Tenant Isolation

- Shared DB, row-level isolation by `tenant_id`.
- Role gates:
  - `shop_floor_gates:hold:place`, `shop_floor_gates:hold:release` (further gated per-hold by `release_authority`)
  - `shop_floor_gates:hold:cancel`
  - `shop_floor_gates:handoff:initiate`, `shop_floor_gates:handoff:accept`, `shop_floor_gates:handoff:reject`
  - `shop_floor_gates:verification:verify`, `shop_floor_gates:verification:reject`
  - `shop_floor_gates:signoff:record` (role-gated by the signoff `role` field — a user must have the permission for that role to sign as it)
- Signoff records retain `signed_by` (user UUID) plus `signer_name` (denormalized display) so audit trail survives user renames or deactivations.

---

## 8. Required Invariants

1. **Operation-scoped hold requires operation_id.** If `scope = operation`, `operation_id` is non-null and references the work_order's operation.
2. **Hold release authority is enforced at release time.** The user releasing must hold the permission corresponding to `release_authority` (e.g. quality-only holds require quality role to release).
3. **Handoff source and dest must belong to the same work order.** `source_operation_id` and `dest_operation_id` both reference operations on `work_order_id`.
4. **Handoff dest can only accept if quantity matches.** Accept is allowed only if handoff `quantity` ≤ remaining quantity at the dest operation (per Production's op quantity tracking).
5. **Verification verify requires operator confirmation.** `verified` transition requires `operator_confirmed_at` to be non-null AND `drawing_verified` AND `material_verified` AND `instruction_verified` all true.
6. **Signoff entity_type is whitelisted.** Only the canonical set is accepted; unknown types return 422.
7. **Signoff records are append-only.** No updates, no deletes. Correcting a bad signoff creates a new record with `action_context` indicating the correction.
8. **Hold prevents operation start when active on that operation.** Downstream — Production should check for active operation-scoped holds before allowing an operation to start. Platform Gates emits `hold.placed.v1`; Production is the enforcer, not Gates. (Alternative: Gates returns active holds via a GET endpoint; Production calls it. Either works; design detail for implementation bead.)
9. **Tenant isolation cross-table.** All joins share `tenant_id`.
10. **Canonical status/type sets platform-owned.** Tenants rename via labels; cannot add/remove canonical codes. Events carry canonical values only.

---

## 9. Cross-module integration notes

- **Production:** Gates references WO/operation IDs. Production reacts to Gates events (e.g. hold placed → production may surface the hold on the work order UI; operation completed with open hold → warning). The enforcement of "cannot start operation with active hold" lives in Production (it owns operation execution) but uses data from Gates.
- **Inventory:** Handoffs and verifications reference `lot_number`/`serial_numbers`. Inventory owns lot state; Gates just references.
- **Quality-Inspection (optional):** verification's `material_verified = true` may be automated by a successful inspection on the lot (vertical wiring — not platform-enforced).
- **Notifications:** subscribes to hold/handoff/verification events to alert supervisors, quality team, operators as configured.

---

## 10. What stays in Fireproof (aerospace overlay, if needed)

Fireproof's AS9100-specific refinements — e.g. mandatory signoff role combinations per AS9100 clause, formal First Article Inspection hold categories, aerospace-specific verification forms — live in a Fireproof overlay service that:
- Subscribes to Gates events
- Stores AS9100-specific metadata (clause citations, formal form references) in its own tables
- Enforces aerospace-specific rules beyond platform's base invariants

Platform Gates has a complete, coherent, non-aerospace base model. HuberPower uses it directly with its own role labels and canonical mappings.

---

## 11. Open questions

- **Verification two-step vs one-step.** Current design: operator confirms → verifier verifies (two-step). Some verticals may want a single-person verification (operator self-verifies). Recommend: keep two-step as default; if verticals need self-verify, operator and verifier can be the same person with the appropriate role combination. Don't add a separate flow.
- **Multi-operation handoffs.** Current design: one handoff = one source op → one dest op. If a WO has a parallel branch (one op feeds two downstream ops), create two handoff records. Could model as handoff with multiple dests, but keeping 1:1 simpler.
- **Rejected verification and rework loop.** Current design: rejected terminal. To retry, create a new verification. Alternative: reopen rejected → pending. Recommend keeping terminal for audit clarity; retries are separate records.
- **Signoff cross-module.** Current design: signoff entity_type restricted to shop-floor entities. If Quality-Inspection wants signoffs on inspections, they embed their own. Revisit if the pattern keeps repeating across modules — could extract to a platform-level signing service.
- **Hold auto-placement from inspection fail.** Quality-Inspection can emit a failure event; Gates could auto-place a hold. Recommend: don't auto-create — leave to verticals. Opinion enforcement is vertical-specific (aerospace auto-holds on fail; others may just flag).

---

## 12. Migration notes (from Fireproof)

- Four Fireproof modules consolidate into `shop-floor-gates` platform module.
- Fireproof's signoff `entity_type` values that match shop-floor entities migrate directly. Non-shop-floor signoffs (e.g. NCR, inspection) stay in Fireproof, embedded in those modules or as Fireproof-local signoff records.
- Sample data — drop Fireproof tables, create platform schema, Fireproof rewires to typed client (`platform_client_shop_floor_gates::*`).
- Fireproof's AS9100-specific verification rules (mandatory clause citation, formal form lineage) migrate to Fireproof's overlay service subscribing to Gates events.
