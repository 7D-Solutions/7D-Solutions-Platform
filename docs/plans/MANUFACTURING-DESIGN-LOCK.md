# Manufacturing Design Lock — Phase 0

**Created:** 2026-03-05
**Bead:** bd-p4mx2
**Status:** DRAFT — Pending Sign-off
**Scope:** Discrete manufacturing only. No backflush v1, no MRP, no NCR/CAPA, no process manufacturing, no scheduling/capacity.

---

## Purpose

This document locks the one-way-door architectural decisions before any manufacturing code is written. Once signed off, these decisions are binding for Phases A through E. Changing them requires orchestrator + ChatGPT approval and a new design-lock bead.

---

## 1. Cost Rollup Flow

**The invariant:** Manufacturing cost is derived from immutable FIFO consumption. No approximations, no averages, no standard cost overrides.

### Flow: Component FIFO Issue → FG Unit Cost → GL Journal

```
1. Production creates Work Order (WO) referencing BOM revision
2. Operator issues each component explicitly (no backflush)
   → Inventory.issue_item() consumes FIFO layers
   → Each issue emits inventory.item_issued with consumed_layers detail
   → GL consumer posts COGS entry (DR WIP, CR Inventory) per issue event
3. Operations complete; Production accumulates:
   - material_cost = Σ(component issue total_cost_minor values)
   - labor_cost = Σ(operation hours × rate) — tracked by Production
   - overhead_cost = applied overhead policy (fixed per-unit or % of labor)
4. WO closes → Production calls Inventory receipt:
   - source_type = "production"
   - unit_cost_minor = (material_cost + labor_cost + overhead_cost) / fg_quantity
   - work_order_id = WO UUID
   → Inventory creates FIFO layer at rolled-up cost
   → Inventory emits inventory.item_received with source_type: "production"
   → GL consumer posts (DR Finished Goods Inventory, CR WIP)
```

### Cost Trust Model

- **Inventory does not validate** that unit_cost equals the sum of consumed components. Production owns the rollup invariant.
- **Inventory does not distinguish** production FIFO layers from purchase FIFO layers during future consumption. A layer is a layer.
- **GL selects posting template** based on `source_type` in the `inventory.item_received` event payload.

### GL Posting Templates by Source Type

| Source Type | Event | Debit | Credit |
|-------------|-------|-------|--------|
| `purchase` | `inventory.item_received` | Inventory (asset) | AP Accrual (liability) |
| `production` | `inventory.item_received` | Finished Goods Inventory (asset) | WIP (asset) |
| `purchase` | `inventory.item_issued` | COGS (expense) | Inventory (asset) |
| `production` | `inventory.item_issued` | WIP (asset) | Raw Material Inventory (asset) |

**Note:** The `inventory.item_issued` posting template already exists for COGS (see `gl_inventory_consumer.rs`). The Phase A/B work adds the `source_type` field so GL can distinguish production issues (DR WIP) from sales issues (DR COGS). The `inventory.item_received` consumer for production receipts is new — GL needs one additional consumer.

---

## 2. Manufacturing Identity Graph

All manufacturing entities use UUIDs as primary keys, consistent with the platform standard. The graph below shows canonical IDs and how they appear in event payloads.

### Entity IDs

| Entity | Module | ID Field | In Event Payloads |
|--------|--------|----------|-------------------|
| Item | Inventory | `item_id: UUID` | All BOM, Production, Inspection events |
| Item Revision | Inventory | `item_revision_id: UUID` | BOM revision lines, Inspection plans |
| BOM Header | BOM | `bom_header_id: UUID` | `bom.header.*` events |
| BOM Revision | BOM | `bom_revision_id: UUID` | `bom.revision.*` events, WO creation |
| BOM Line | BOM | `bom_line_id: UUID` | `bom.line.*` events |
| Work Order | Production | `work_order_id: UUID` | All Production events, Inventory issue/receipt source_ref |
| Operation | Production | `operation_id: UUID` | `production.operation.*` events |
| Workcenter | Production (Phase B) | `workcenter_id: UUID` | Operation events, Maintenance downtime |
| Lot | Inventory | `lot_code: String` | Inventory receipt/issue, Inspection records |
| Serial | Inventory | `serial_codes: Vec<String>` | Inventory receipt/issue (when serialized) |
| Inspection Plan | Inspection | `inspection_plan_id: UUID` | `inspection.plan.*` events |
| Inspection Record | Inspection | `inspection_record_id: UUID` | `inspection.record.*` events |
| ECO | BOM | `eco_id: UUID` | `bom.eco.*` events, BOM revision link |

### Correlation and Causation Chains

Every event carries the platform `EventEnvelope` fields:
- `event_id` — unique, deterministic from business key (idempotency anchor)
- `correlation_id` — traces the entire business transaction (e.g., WO lifecycle)
- `causation_id` — links to the immediate triggering event

**WO lifecycle example:**

```
production.work_order_created  (correlation_id = WO-UUID)
  └─ inventory.item_issued     (correlation_id = WO-UUID, causation_id = wo_created_event_id)
       └─ gl.posting.request   (correlation_id = WO-UUID, causation_id = item_issued_event_id)
  └─ production.operation_completed (correlation_id = WO-UUID)
  └─ production.work_order_closed   (correlation_id = WO-UUID)
       └─ inventory.item_received   (correlation_id = WO-UUID, causation_id = wo_closed_event_id)
            └─ gl.posting.request   (correlation_id = WO-UUID, causation_id = item_received_event_id)
```

**Traceability guarantee:** Given a `work_order_id`, you can query all related events across all modules via `correlation_id`. Given any single event, you can walk the causation chain backward to the originating command.

### Cross-Module References

Manufacturing entities reference each other by UUID only — no foreign key imports across module databases. Each module stores the UUID and resolves it via HTTP API or event projection when needed.

| Reference | Stored In | Resolves Via |
|-----------|-----------|-------------|
| `item_id` on BOM line | BOM DB | Inventory HTTP API |
| `bom_revision_id` on Work Order | Production DB | BOM HTTP API |
| `work_order_id` on Inventory issue/receipt | Inventory DB | Production HTTP API (if needed) |
| `workcenter_id` on Operation | Production DB | Local table (Production owns) |
| `workcenter_id` on Downtime Event | Maintenance DB | Event projection from Production |
| `inspection_plan_id` on Inspection Record | Inspection DB | Local table |
| `eco_id` on BOM Revision | BOM DB | Local table (BOM owns ECO) |

---

## 3. WIP Representation Decision

**Decision: Inventory location/state model — no separate WIP ledger.**

### How It Works

- WIP is represented as **inventory in a designated WIP location** within the same warehouse.
- When components are issued to a work order, Inventory records an issue from the stock location. The cost leaves on-hand via FIFO consumption. The GL entry (DR WIP, CR Inventory) creates the WIP balance in GL, not in Inventory.
- Inventory does **not** track WIP balance as a separate quantity bucket. WIP exists only as a GL account balance.
- When the finished good is received, the GL entry (DR FG Inventory, CR WIP) closes the WIP balance.

### Why Not a Production Ledger

A separate production WIP ledger would:
1. Duplicate cost tracking that GL already provides.
2. Require reconciliation between two sources of truth for WIP valuation.
3. Add complexity to the audit trail — auditors would need to reconcile Inventory + Production + GL instead of just Inventory + GL.

### Implications

- **WIP valuation** is always derived from GL account balances, not from a production module query.
- **WIP aging** (how long a WO has been open) is derived from Production's work order timestamps, not from inventory movement dates.
- **Material still on the floor** (issued but not yet consumed in an operation) is cost-tracked via GL WIP, not via an intermediate inventory state. If an issued component is returned unused, Production issues a return that reverses the original issue.
- **Phase B must implement** a WO cost accumulator that tracks material + labor + overhead totals for the rollup calculation, but this is a Production-internal data structure, not an inventory position.

---

## 4. Variance Handling Policy v1

**Decision: Variances are explicitly disallowed in v1. All cost discrepancies are errors, not variances.**

### What This Means

- The FG receipt `unit_cost_minor` MUST equal `(Σ material issues + labor + overhead) / fg_quantity`, computed by Production.
- There is no "standard cost vs actual cost" comparison. There are no purchase price variances. There is no usage variance bucket.
- If actual material consumption differs from BOM quantities (e.g., scrap during production), the extra issue is recorded as an additional `inventory.item_issued` event. The cost flows into WIP via the normal GL path. The FG receipt reflects the actual total cost.

### Enforcement Point

- **Production module** enforces the cost rollup arithmetic before calling Inventory receipt. If the numbers don't balance, Production rejects the WO close.
- **Inventory** does not validate — it trusts the caller (same as purchase receipts trusting the PO price).
- **GL** does not validate — it posts whatever the source modules emit.

### Why No Variance Accounts

Variance accounting requires:
1. A standard cost master (doesn't exist yet, not planned).
2. Variance account definitions per variance type (price, usage, efficiency, volume).
3. Variance disposition rules (period-end allocation to COGS/inventory).

All of this is significant complexity that adds no value when the cost model is "actual cost, FIFO." The first customer (aerospace/defense) uses actual cost — standard cost is not needed.

### Future Path

If a future customer requires standard costing:
1. Add a standard cost field to items.
2. Compute variances as the difference between standard and actual at each transaction.
3. Post variances to dedicated GL accounts.

This can be added as a configuration option without changing the FIFO consumption model.

---

## 5. GL Posting Trigger Model

### Which Events Trigger GL

| Trigger Event | GL Action | Poster | Phase |
|---------------|-----------|--------|-------|
| `inventory.item_issued` (source_type = sale/consumption) | DR COGS, CR Inventory | GL inventory consumer | Existing |
| `inventory.item_issued` (source_type = production) | DR WIP, CR Raw Material Inventory | GL inventory consumer (extended) | A/B |
| `inventory.item_received` (source_type = purchase) | DR Inventory, CR AP Accrual | GL inventory consumer | Existing |
| `inventory.item_received` (source_type = production) | DR FG Inventory, CR WIP | GL inventory consumer (new template) | B |

### Who Posts

**GL posts itself.** Source modules (Inventory, Production) emit business events. GL subscribes and creates journal entries. No module calls GL directly.

This is the existing pattern — `gl_inventory_consumer.rs` already subscribes to `inventory.item_issued` and posts COGS journals. The manufacturing extension adds:
1. A `source_type` check in the existing consumer to select the correct debit account (COGS vs WIP).
2. A new subscription to `inventory.item_received` with `source_type = "production"` to post the FG receipt journal.

### Minimal Payload Fields Required for GL Posting

From `inventory.item_issued`:
- `tenant_id` — GL account lookup
- `total_cost_minor` — journal amount
- `currency` — journal currency
- `source_ref.source_type` — template selection (sale → COGS, production → WIP)
- `item_id` — dimensions (optional, for cost center reporting)
- `warehouse_id` — dimensions (optional, for location reporting)

From `inventory.item_received` (production):
- `tenant_id` — GL account lookup
- `unit_cost_minor` × `quantity` — journal amount
- `currency` — journal currency
- `source_type` — must be "production" for FG receipt template
- `work_order_id` — dimensions (for WO cost tracking)

### SourceDocType Extension

The existing `SourceDocType` enum in `gl_posting_request_v1.rs` already includes `InventoryReceipt` and `InventoryIssue`. No new enum values needed for manufacturing — the `source_type` field inside the event payload provides the production/purchase distinction.

---

## 6. BOM Schema Decisions (Confirmed)

*Carried forward from `docs/plans/manufacturing-prerequisites-claude-desktop.md`, Section 3.*

### 6a. Depth: Unlimited With Query Guard

- BOM data model supports **unlimited depth** (multi-level through recursive item → BOM → item chains).
- Explosion query uses Postgres recursive CTE with **configurable per-tenant `max_explosion_depth`** (default: 20).
- Postgres 16 `CYCLE` clause detects circular references.
- Real-world manufacturing BOMs rarely exceed 12-15 levels; 20 provides headroom.

### 6b. Effectivity: Date-Based Only

- V1 supports **date-based effectivity only** (effective_from/effective_to timestamp range).
- Non-overlapping constraint per (tenant, parent_item) via Postgres `EXCLUDE USING gist`.
- `effectivity_type` enum field on BOM revision for forward compatibility (`'date'` only in v1).
- Serial-number effectivity deferred — Fireproof can map serials to dates via their app layer.

---

## 7. Workcenter Ownership Path (Confirmed)

*Carried forward from `docs/plans/manufacturing-prerequisites-claude-desktop.md`, Section 2. Updated per MANUFACTURING-ROADMAP.md Key Decisions.*

**Decision: Production owns the workcenter master from Phase B. No temporary workcenter table in Maintenance during Phase A.**

### What Happens in Each Phase

| Phase | Workcenter State |
|-------|-----------------|
| Phase A | Maintenance's `downtime_events.workcenter_id` remains a bare UUID with no FK. No workcenter table created anywhere. |
| Phase B | Production creates the workcenter master table with full fields (code, name, capacity, calendars, cost rates). Production emits `production.workcenter.*` events. |
| Phase E | Maintenance consumes `production.workcenter.*` events to build a read-only projection. Downtime FK resolves against the projection. |

### Why Changed from Prerequisites Doc

The prerequisites doc proposed a temporary workcenter table in Maintenance during Phase A. The MANUFACTURING-ROADMAP.md (post-7-reviewer synthesis) decided against this: "No temporary table in Maintenance during Phase A." Rationale: creating a table only to migrate and delete it in Phase B is unnecessary churn. Maintenance's bare UUID works fine during Phase A — it's an unproven module (v0.1.0) with no constraint enforcement needed yet.

---

## 8. Event Contract Naming Review

### Existing Pattern

The platform uses: `<module>.<entity>_<action>` for event types, published on NATS subject `<module>.events.<event_type>`.

Existing examples from codebase:
- `inventory.item_received` → `inventory.events.inventory.item_received`
- `inventory.item_issued` → `inventory.events.inventory.item_issued`
- `inventory.adjusted` → `inventory.events.inventory.adjusted`
- `gl.posting.accepted` → `gl.events.gl.posting.accepted`
- `auth.events.password_reset_completed`

### Confirmed Naming Pattern

Format: `<module>.<entity>_<action>` (underscored entity+action, dotted module prefix).

This matches the existing inventory events. The bead description suggested `{module}.{entity}_{action}` — confirmed.

### Minimal New Manufacturing Events (Phases A-C)

#### Phase A — BOM Module

| Event Type | NATS Subject | Emitted When |
|------------|-------------|-------------|
| `bom.header_created` | `bom.events.bom.header_created` | New BOM header created for a parent item |
| `bom.revision_created` | `bom.events.bom.revision_created` | New BOM revision created (draft) |
| `bom.revision_released` | `bom.events.bom.revision_released` | BOM revision set to released status with effectivity dates |
| `bom.revision_obsoleted` | `bom.events.bom.revision_obsoleted` | BOM revision set to obsolete status |
| `bom.line_added` | `bom.events.bom.line_added` | Component line added to a BOM revision |
| `bom.line_removed` | `bom.events.bom.line_removed` | Component line removed from a BOM revision |

#### Phase A — Inventory Extensions (No New Events)

The existing `inventory.item_received` and `inventory.item_issued` events gain a `source_type` field in the payload. No new event types — the same events serve purchase and production flows, differentiated by `source_type`.

#### Phase B — Production Module

| Event Type | NATS Subject | Emitted When |
|------------|-------------|-------------|
| `production.work_order_created` | `production.events.production.work_order_created` | WO created (not yet released) |
| `production.work_order_released` | `production.events.production.work_order_released` | WO released to floor |
| `production.work_order_closed` | `production.events.production.work_order_closed` | WO completed, FG receipt triggered |
| `production.component_issued` | `production.events.production.component_issued` | Component issued to WO (triggers Inventory issue) |
| `production.operation_started` | `production.events.production.operation_started` | Operation begins at workcenter |
| `production.operation_completed` | `production.events.production.operation_completed` | Operation finished |
| `production.fg_received` | `production.events.production.fg_received` | Finished good receipt recorded (triggers Inventory receipt) |
| `production.workcenter_created` | `production.events.production.workcenter_created` | New workcenter defined |
| `production.workcenter_updated` | `production.events.production.workcenter_updated` | Workcenter details changed |

#### Phase C1 — Receiving Inspection

| Event Type | NATS Subject | Emitted When |
|------------|-------------|-------------|
| `inspection.plan_created` | `inspection.events.inspection.plan_created` | Inspection plan defined for item/revision |
| `inspection.record_created` | `inspection.events.inspection.record_created` | Inspection record opened (e.g., from S-R receipt) |
| `inspection.record_completed` | `inspection.events.inspection.record_completed` | All characteristics inspected |
| `inspection.disposition_decided` | `inspection.events.inspection.disposition_decided` | Accept/reject/hold decision recorded |

#### Phase C2 — In-Process/Final Inspection

No new event types beyond Phase C1 — in-process and final inspections use the same `inspection.record_*` events with a `inspection_type` field in the payload (`receiving`, `in_process`, `final`).

#### Phase D — ECO

| Event Type | NATS Subject | Emitted When |
|------------|-------------|-------------|
| `bom.eco_submitted` | `bom.events.bom.eco_submitted` | ECO submitted for approval |
| `bom.eco_approved` | `bom.events.bom.eco_approved` | ECO approved via Workflow |
| `bom.eco_applied` | `bom.events.bom.eco_applied` | ECO changes applied (BOM revision superseded) |

### Events Consumed by Manufacturing Modules

| Consumer | Subscribes To | Purpose |
|----------|--------------|---------|
| GL | `inventory.item_issued` | COGS or WIP journal (extended with source_type) |
| GL | `inventory.item_received` | FG receipt or purchase receipt journal (extended with source_type) |
| Inspection (C1) | S-R receipt event (TBD) | Auto-create receiving inspection record |
| Inspection (C2) | `production.operation_completed` | Auto-create in-process inspection record |
| Maintenance (E) | `production.workcenter_created/updated` | Workcenter projection |

---

## 9. Scope Fences (Restated)

These constraints apply to ALL manufacturing phases. They cannot be changed without orchestrator + ChatGPT approval.

- **Discrete manufacturing only** — no process/recipe BOM, no repetitive/rate-based, no mixed-mode
- **No backflush in v1** — explicit component issue only (operator scans each part)
- **No MRP/Planning** — manual work order creation
- **No NCR/CAPA lifecycle** — Phase C provides inspection + hold/release only; NCR/CAPA is a separate future module
- **No special process rule catalogs** — platform provides generic evidence capture; aerospace rules live in Fireproof
- **No production scheduling/capacity optimization**
- **No standard costing or variance accounts** — actual cost, FIFO only
- **No CostBreakdown JSONB in v1** — source_type + caller-provided unit_cost is sufficient
- **Tests are integrated** — real Postgres, real NATS, no mocks, no stubs

---

## 10. Sign-off

| Reviewer | Role | Status | Date |
|----------|------|--------|------|
| BrightHill | Orchestrator | — | — |
| CopperRiver | Implementation agent | — | — |
| PurpleCliff | Implementation agent | — | — |
| SageDesert | Implementation agent | — | — |
| DarkOwl | Implementation agent | — | — |
| MaroonHarbor | Implementation agent | DRAFTED | 2026-03-05 |
| ChatGPT | External reviewer | — | — |

**Approval required from:** BrightHill + at least 3 implementation agents + ChatGPT.

---

*This document is the Phase 0 design lock. Phase A beads cannot be created until sign-off is complete.*
