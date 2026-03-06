# Manufacturing Build Prerequisites — Claude Desktop

**Date:** 2026-03-04
**Bead:** bd-2wahd
**Status:** Pre-bead design decisions and contract definitions
**Audience:** BrightHill (Orchestrator), all implementing agents

---

## Purpose

This document captures the design decisions and contract definitions that must be settled before manufacturing beads are created. It incorporates the consensus from 7 reviewers (synthesis doc), the 11-tab lifecycle diagram (drill-down review), and four scoping questions raised by the platform owner.

Everything in this document is a prerequisite for Phase A beads. Implementing agents should treat this as binding unless the orchestrator overrides.

---

## 1. Production Receipt Cost Contract

### The Problem

Inventory's receipt service currently accepts a `unit_cost_minor` field that represents purchase price (from PO line). When Production receipts finished goods, the cost is different — it's the rolled-up cost of consumed components + labor + overhead. The Inventory retrofit needs to know what this interface looks like without knowing how Production computes the cost (Production doesn't exist yet in Phase A).

### The Decision

Inventory accepts a **caller-provided unit cost** on production receipts. Inventory does not compute or validate the rollup — it trusts the caller (Production module in Phase B) to provide the correct value. This is the same trust model as purchase receipts, where Inventory trusts the PO price.

### Contract: Production Receipt API

Extend the existing `POST /api/inventory/receipts` endpoint. The current `ReceiptRequest` struct already has the right shape — the changes are additive:

```
ReceiptRequest (existing fields preserved, new fields added)
├── tenant_id: String                      (existing)
├── item_id: Uuid                          (existing)
├── warehouse_id: Uuid                     (existing)
├── location_id: Option<Uuid>              (existing)
├── quantity: i64                          (existing)
├── unit_cost_minor: i64                   (existing — Production provides rolled-up cost)
├── currency: String                       (existing)
├── idempotency_key: String                (existing)
├── correlation_id: Option<String>         (existing)
├── causation_id: Option<String>           (existing)
├── lot_code: Option<String>               (existing)
├── serial_codes: Option<Vec<String>>      (existing)
├── uom_id: Option<Uuid>                   (existing)
├── purchase_order_id: Option<Uuid>        (existing — null for production receipts)
│
│   NEW FIELDS:
├── source_type: Option<String>            (NEW — "purchase" | "production" | "return" | null)
├── work_order_id: Option<Uuid>            (NEW — set for production receipts)
└── cost_breakdown: Option<CostBreakdown>  (NEW — optional structured cost detail)

CostBreakdown (new, optional)
├── material_cost_minor: i64               (sum of component FIFO costs consumed)
├── labor_cost_minor: i64                  (operation labor hours × rate)
└── overhead_cost_minor: i64               (applied overhead)
```

### How It Works

**Purchase receipt (existing behavior, unchanged):**
```json
{
  "source_type": "purchase",
  "unit_cost_minor": 1500,
  "purchase_order_id": "uuid-of-po",
  "work_order_id": null,
  "cost_breakdown": null
}
```

**Production receipt (Phase B, when Production module exists):**
```json
{
  "source_type": "production",
  "unit_cost_minor": 4200,
  "purchase_order_id": null,
  "work_order_id": "uuid-of-work-order",
  "cost_breakdown": {
    "material_cost_minor": 2800,
    "labor_cost_minor": 900,
    "overhead_cost_minor": 500
  }
}
```

### What Inventory Does With This

1. **Ledger entry:** `entry_type` = `'produced'` (new enum value, added in Phase A migration). The existing `reference_type` / `reference_id` columns store `"work_order"` / `work_order_id`.
2. **FIFO layer:** Created at `unit_cost_minor` = 4200 (the total rolled-up cost). The layer is consumed during future issues using the same FIFO logic as purchased layers — no special treatment.
3. **Cost breakdown:** Stored as a JSONB column on the ledger row (`cost_detail`). Optional — if null, only the aggregate `unit_cost_minor` is recorded. This is metadata for reporting, not used in FIFO consumption.
4. **On-hand projection:** Updated exactly like a purchase receipt. `quantity_on_hand += quantity`.
5. **Event:** `inventory.item_received` with `source_type: "production"` in the payload. Downstream consumers (GL, reporting) use `source_type` to determine which GL accounts to hit (WIP → Finished Goods, not AP Accrual → Inventory).

### What Inventory Does NOT Do

- Does not validate that `unit_cost_minor == material + labor + overhead`. Production owns that invariant.
- Does not know about BOMs, routings, or operations. Inventory sees a finished good arriving at a cost.
- Does not distinguish production FIFO layers from purchase FIFO layers during consumption. A layer is a layer.

### GL Posting Implications

The `source_type` field is what GL uses to determine journal entry shape:

| Source Type | Debit | Credit |
|-------------|-------|--------|
| `purchase` | Inventory (asset) | AP Accrual (liability) |
| `production` | Finished Goods Inventory (asset) | WIP (asset) |

GL consumes the `inventory.item_received` event and reads `source_type` to select the posting template. This means GL needs a minor update (new posting template for production receipts) but no structural change.

### Phase A Retrofit Scope (Inventory)

1. **Migration:** Add `'produced'` to the `entry_type` enum. Add nullable `cost_detail JSONB` column to `inventory_ledger`. Add nullable `source_type TEXT` column to `inventory_ledger` (backfill existing rows as `'purchase'`).
2. **ReceiptRequest:** Add `source_type`, `work_order_id`, `cost_breakdown` fields (all optional, backward compatible).
3. **Receipt service:** When `source_type = "production"`, use `entry_type = 'produced'`, set `reference_type = "work_order"`, store `cost_breakdown` in `cost_detail`.
4. **Event payload:** Add `source_type` field to `ItemReceivedPayload`.
5. **No changes** to FIFO layer logic, on-hand projection, issue service, or valuation.

### What This Enables in Phase B

Production module creates a work order, tracks operations, accumulates costs, and when the WO closes, calls `POST /api/inventory/receipts` with the rolled-up cost. Inventory doesn't need to change again. The cost rollup computation lives entirely in Production.

---

## 2. Workcenter Ownership Transition Plan

### The Problem

Production owns the workcenter master (consensus decision), but Production doesn't exist until Phase B. Maintenance currently has a bare `workcenter_id: UUID` on DowntimeEvent with no source-of-truth table. The Phase A Maintenance retrofit needs to add a workcenter table.

### The Decision

Build a **minimal workcenter table in Maintenance during Phase A**. Transfer ownership to Production in Phase B via event-driven projection.

### Phase A: Maintenance Owns Temporarily

Create the workcenter master table in Maintenance with minimal fields:

```sql
CREATE TABLE workcenters (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id       TEXT NOT NULL,
    code            TEXT NOT NULL,
    name            TEXT NOT NULL,
    description     TEXT,
    location        TEXT,
    active          BOOLEAN NOT NULL DEFAULT true,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT now(),

    CONSTRAINT uq_workcenter_tenant_code UNIQUE (tenant_id, code)
);
```

**What Maintenance does NOT add in Phase A:**
- No capacity fields (hours/day, shifts, calendars) — that's Production's concern
- No routing associations — that's Production's concern
- No machine-to-workcenter mapping beyond the existing downtime FK
- No cost rates — that's Production's concern

**Events emitted in Phase A:**
- `maintenance.workcenter.created`
- `maintenance.workcenter.updated`
- `maintenance.workcenter.deactivated`

These events use the `maintenance.*` namespace because Maintenance owns the table in Phase A.

### Phase B: Ownership Transfers to Production

When Production is scaffolded:

1. **Production creates its own workcenter table** with full fields (capacity, calendars, cost rates, routing associations) plus all the fields from Maintenance's table.
2. **One-time data migration:** Copy rows from Maintenance's `workcenters` table to Production's `workcenters` table.
3. **Production emits:** `production.workcenter.created`, `production.workcenter.updated`, etc.
4. **Maintenance drops its workcenter table** and instead maintains a **read-only projection** populated by consuming `production.workcenter.*` events. Maintenance's downtime tracking continues to reference workcenter IDs — the FK just points to its local projection table instead of its own master table.
5. **Event namespace change:** Maintenance stops emitting `maintenance.workcenter.*` events. Any consumer that subscribed to those events needs to migrate to `production.workcenter.*`. Since Maintenance is unproven (v0.1.0), this is a non-breaking change — no version bump required.

### Why Not Build It "Neutral"

There is no neutral location in this architecture. The platform standard says: every table is owned by exactly one module. Shared platform crates (`platform/*`) are libraries, not services with databases. Creating a `platform/workcenters` crate would violate the module standard and create an orphan service that no one maintains. Better to accept temporary ownership in Maintenance and transfer cleanly.

### Implementing Agent Instructions

The Maintenance retrofit bead should include this note:

> The workcenter table created in this bead is intentionally minimal. It provides a source of truth for workcenter IDs referenced by downtime events. Ownership of this table transfers to the Production module in Phase B. Do NOT add capacity, calendar, cost rate, or routing fields — those belong in Production. Do NOT create CRUD endpoints beyond basic create/update/deactivate. Do NOT build a workcenter management UI.

---

## 3. BOM Design Decisions

### 3a. Depth: Unlimited With Query Guard

**Decision:** The BOM data model supports **unlimited depth**. The multi-level explosion query has a **configurable per-tenant `max_explosion_depth` guard** (default: 20) and uses Postgres 16's native `CYCLE` detection to catch circular references.

**Data model:**

```sql
CREATE TABLE bom_lines (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id       TEXT NOT NULL,
    bom_revision_id UUID NOT NULL REFERENCES bom_revisions(id),
    component_item_id UUID NOT NULL,  -- FK to inventory items (cross-module ref by ID, not by import)
    quantity_per    NUMERIC(18, 6) NOT NULL CHECK (quantity_per > 0),
    uom_id          UUID,             -- unit of measure for quantity_per
    position        INT NOT NULL,     -- sort order within BOM
    reference_designator TEXT,        -- e.g., "R1", "C3" for electronics; null for most manufacturing
    notes           TEXT,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),

    CONSTRAINT uq_bom_line UNIQUE (bom_revision_id, component_item_id)
);
```

A component (`component_item_id`) can itself be a parent item in another BOM — this is what creates multi-level depth. There is no explicit depth column or tree structure; depth is emergent from the recursive BOM-item-BOM chain.

**Explosion query pattern:**

```sql
WITH RECURSIVE bom_tree AS (
    -- Base case: top-level BOM lines for the requested item + revision
    SELECT bl.component_item_id, bl.quantity_per, 1 AS depth
    FROM bom_lines bl
    JOIN bom_revisions br ON bl.bom_revision_id = br.id
    WHERE br.parent_item_id = $1
      AND br.tenant_id = $2
      AND br.effective_from <= $3
      AND (br.effective_to IS NULL OR br.effective_to > $3)

    UNION ALL

    -- Recursive case: explode sub-assemblies
    SELECT bl.component_item_id,
           bt.quantity_per * bl.quantity_per AS quantity_per,
           bt.depth + 1 AS depth
    FROM bom_tree bt
    JOIN bom_revisions br ON br.parent_item_id = bt.component_item_id
      AND br.tenant_id = $2
      AND br.effective_from <= $3
      AND (br.effective_to IS NULL OR br.effective_to > $3)
    JOIN bom_lines bl ON bl.bom_revision_id = br.id
    WHERE bt.depth < $4  -- max_explosion_depth guard
)
CYCLE component_item_id SET is_cycle USING path
SELECT component_item_id, SUM(quantity_per) AS total_quantity, MAX(depth) AS max_depth
FROM bom_tree
WHERE NOT is_cycle
GROUP BY component_item_id;
```

The `$4` parameter is `max_explosion_depth` (default 20). The `CYCLE` clause prevents infinite loops from circular BOM references and flags them in the result set so the caller can report the error.

**Why 20:** Real-world manufacturing BOMs rarely exceed 12-15 levels (complex aircraft assemblies are at the extreme end). 20 provides headroom without allowing runaway queries. The guard is per-tenant and configurable — if a specific tenant legitimately needs deeper BOMs, it can be raised.

### 3b. Effectivity: Date-Based Only, With Forward-Compatibility Seam

**Decision:** V1 supports **date-based effectivity only**. Serial-number-based effectivity is deferred. The data model includes an `effectivity_type` enum field for forward compatibility.

**Data model:**

```sql
CREATE TYPE effectivity_type AS ENUM ('date');
-- Future: ALTER TYPE effectivity_type ADD VALUE 'serial_number';
-- Future: ALTER TYPE effectivity_type ADD VALUE 'lot_number';

CREATE TABLE bom_revisions (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id       TEXT NOT NULL,
    parent_item_id  UUID NOT NULL,    -- the item this BOM builds
    revision_number INT NOT NULL,     -- auto-increment per (tenant_id, parent_item_id)
    status          TEXT NOT NULL DEFAULT 'draft' CHECK (status IN ('draft', 'released', 'obsolete')),
    effectivity_type effectivity_type NOT NULL DEFAULT 'date',
    effective_from  TIMESTAMPTZ,      -- null = draft (not yet effective)
    effective_to    TIMESTAMPTZ,      -- null = open-ended (currently effective)
    change_reason   TEXT,
    eco_id          UUID,             -- optional link to Engineering Change Order
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),

    CONSTRAINT uq_bom_rev UNIQUE (tenant_id, parent_item_id, revision_number),
    -- Non-overlapping date windows per item (same pattern as Inventory item_revisions)
    CONSTRAINT no_overlap EXCLUDE USING gist (
        tenant_id WITH =,
        parent_item_id WITH =,
        tstzrange(effective_from, effective_to) WITH &&
    ) WHERE (effective_from IS NOT NULL)
);
```

**Effectivity resolution:** Given an item and a timestamp, the active BOM revision is the one where `effective_from <= timestamp AND (effective_to IS NULL OR effective_to > timestamp)`. The exclusion constraint guarantees at most one active revision per item at any point in time.

**The forward-compatibility seam:** The `effectivity_type` enum field costs one column and zero runtime complexity in v1 (all rows are `'date'`, the explosion query ignores it). When serial-number effectivity is added later:

1. Add `'serial_number'` to the enum
2. Add nullable `effective_from_serial TEXT` and `effective_to_serial TEXT` columns
3. Update the explosion API to accept an optional `serial_number` parameter
4. When `effectivity_type = 'serial_number'`, resolve the active revision by serial range instead of date range

Fireproof can implement serial-based effectivity as an app-specific layer in the meantime — their vertical repo maps serials to date ranges using the BOM module's date-based API.

---

## 4. Phase A Bead Scope Summary

Based on everything above, Phase A produces these deliverables:

### Bead A1: Inventory Retrofit

**Scope:**
- Migration: add `'produced'` to `entry_type` enum
- Migration: add nullable `source_type TEXT` to `inventory_ledger` (backfill `'purchase'`)
- Migration: add nullable `cost_detail JSONB` to `inventory_ledger`
- Migration: add nullable `work_order_id UUID` to receipt flow
- Extend `ReceiptRequest` with `source_type`, `work_order_id`, `cost_breakdown`
- Extend `ItemReceivedPayload` event with `source_type`
- Add `procurement_type` item classification (via existing classifications system — no migration, just a documented convention: `classification_system = "procurement_type"`, `code = "make" | "buy" | "both"`)
- Production issue movement type: extend `IssueRequest` with `work_order_id` reference (same pattern as `purchase_order_id` on receipts)

**Does NOT include:** Backflush logic, cost rollup computation, BOM awareness, any new FIFO layer behavior.

**Tests:** Receipt with `source_type = "production"` creates correct ledger entry, FIFO layer, and event. Issue with `work_order_id` reference creates correct ledger entry. Existing purchase receipt behavior unchanged (regression).

### Bead A2: Maintenance Workcenter Retrofit

**Scope:**
- Migration: create `workcenters` table (id, tenant_id, code, name, description, location, active, timestamps)
- CRUD endpoints: create, update, deactivate, list, get
- FK from `downtime_events.workcenter_id` to `workcenters.id` (currently bare UUID)
- Events: `maintenance.workcenter.created`, `maintenance.workcenter.updated`, `maintenance.workcenter.deactivated`
- Document: ownership transfers to Production in Phase B

**Does NOT include:** Capacity, calendars, cost rates, routing associations, machine-to-workcenter mapping.

### Bead A3: BOM Module Scaffold

**Scope:**
- New crate: `bom-rs` under `modules/bom/`
- Entities: `BomHeader`, `BomRevision` (with `effectivity_type` enum, date-based only), `BomLine`
- ECO entity: `EngineeringChangeOrder`, `EcoAffectedItem` (lifecycle via Workflow module)
- Migrations: all tables above
- HTTP API: CRUD for BOMs, revisions, lines; activate/obsolete revision; multi-level explosion; where-used query
- Explosion query: recursive CTE with depth guard (default 20) and `CYCLE` detection
- Events: `bom.header.created`, `bom.revision.created`, `bom.revision.released`, `bom.line.added`, `bom.line.removed`, `bom.eco.submitted`, `bom.eco.approved`, `bom.eco.applied`
- Consumed events: `workflow.events.instance.completed` (ECO approval), `inventory.item.created` (optional, for awareness of new items)
- Numbering integration: part numbers and ECO numbers via Numbering module API
- Integration tests: multi-level explosion, effectivity date resolution, circular BOM detection, where-used reverse lookup

**Does NOT include:** Serial-number effectivity, BOM costing (deferred to Production cost rollup), BOM comparison/diff, BOM import/export, alternate components.

**Target: BOM v1.0.0 proof** at end of Phase A.

### Bead A4 (optional, can parallel): Shipping-Receiving Inspection Bridge

**Scope:**
- Wire existing `inspection_routing` decisions to emit a contract event consumable by the future Inspection module
- Currently `inspection_routing` sets a flag but creates no inspection record — add event emission so Inspection (Phase C) has a trigger point

**Does NOT include:** Inspection records (that's the Inspection module's job).

---

## 5. Decisions Register

For orchestrator reference — all decisions made in this session:

| # | Decision | Rationale | Affects |
|---|----------|-----------|---------|
| D1 | Cost rollup: Inventory accepts caller-provided unit cost | Keeps Inventory simple; Production computes cost in Phase B | Bead A1 |
| D2 | Cost breakdown stored as optional JSONB, not validated | Reporting use only; Production owns the invariant | Bead A1 |
| D3 | `source_type` field distinguishes purchase from production receipts | GL needs this to select posting template | Bead A1, GL |
| D4 | Workcenter table in Maintenance (Phase A), transfers to Production (Phase B) | No neutral location in architecture; pragmatic temporary ownership | Bead A2, Phase B |
| D5 | Workcenter table is intentionally minimal — no capacity/calendar/rates | Those fields belong to Production | Bead A2 |
| D6 | BOM supports unlimited depth; explosion query has configurable depth guard (default 20) | Real BOMs rarely exceed 15 levels; Postgres CYCLE detection catches circular refs | Bead A3 |
| D7 | Date-based effectivity only in v1 | Serial-number effectivity is aerospace-specific and cleanly deferrable | Bead A3 |
| D8 | `effectivity_type` enum on BomRevision for forward compatibility | One column now avoids schema redesign later | Bead A3 |
| D9 | No backflush in v1 | Explicit component issue first; backflush is Phase B+ | Bead A1 (scope fence) |
| D10 | Discrete manufacturing only in v1 | Process manufacturing (recipe/formula BOM) is out of scope | Bead A3 (scope fence) |

---

*This document is ready for bead creation. All Phase A prerequisites are defined.*
