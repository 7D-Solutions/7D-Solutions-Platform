# Fireproof Reuse Investigation: Physical Hierarchy, Storage Locations, and Inventory Movement

**Investigator:** CopperRiver
**Date:** 2026-03-05
**Bead:** bd-6sb5k

---

## Executive Summary

All three Fireproof modules are **generic and well-structured** — none contain gauge-specific business logic. However, the 7D Platform already has a simpler location model inside Inventory. The recommendation is:

| Module | Recommendation | Rationale |
|--------|---------------|-----------|
| Organization hierarchy | **ADAPT-PATTERN** | Generic concept, but the platform already uses a flat warehouse model. Extracting the full 3-level hierarchy would require reworking Inventory's warehouse_id FK relationships. Better to adapt the pattern into a new platform module when a second vertical needs it. |
| Storage locations | **ADAPT-PATTERN** | Tightly coupled to org hierarchy via zone_id. The platform already has a `locations` table under Inventory (keyed by warehouse_id). The Fireproof model adds richer taxonomy (location_type, allowed_item_types) that should inform future platform enhancements. |
| Inventory movement | **EXTRACT** | Entirely new capability. The platform has no physical movement tracking — Inventory deals with financial transactions (receipts/issues/transfers between warehouses). Movement tracking (immutable evidence of items physically relocating) complements the existing model without conflict. |

---

## Module 1: Organization Hierarchy

**Source:** `/Users/james/Projects/Fireproof-ERP/crates/fireproof-erp/src/organization/`
**LOC:** ~1,165 (types: 231, repository: 443, service: 491)

### What It Does

Three-level physical hierarchy: **Facility → Building → Zone**, all tenant-scoped with integer IDs. Each level has code/name/is_active/display_order. Business invariants:

- Can't deactivate the last active facility for a tenant
- Can't deactivate a facility with active buildings
- Can't deactivate a building with active zones
- Parent validation on create/update (building must belong to existing facility, zone to existing building)
- RBAC checked via `organization:read` / `organization:write` scopes

### Is It Generic?

**Yes.** No gauge-specific logic anywhere. A food manufacturer or machine shop would need the exact same Facility → Building → Zone hierarchy. The only Fireproof coupling is the import paths for `ApiError`, `RequestContext`, and `AuthzGate` — all from shared Fireproof infrastructure crates, not domain logic.

### Platform Comparison

The platform has **no org hierarchy module**. Inventory uses a flat `warehouse_id` (UUID) on items, locations, ledger entries, and transfers. There is no concept of facilities, buildings, or zones in the platform today.

The warehouse concept appears across 20+ files in the Inventory domain. Adding a Facility → Building → Zone → Warehouse mapping would require either:
- A) Retrofitting all existing warehouse_id references to point through a hierarchy (high risk, touches proven 22K LOC module)
- B) Building org hierarchy as an independent module that warehouses optionally reference (lower risk)

### What Changes Would Be Needed for Extraction

1. **Strip Fireproof imports:** Replace `crate::error_registry::ApiError` with platform error pattern, replace `crate::identity_auth::context::RequestContext` / `AuthzGate` with platform `security` crate equivalents
2. **ID type decision:** Fireproof uses integer SERIAL IDs (matching legacy). Platform modules use UUID. Would need to decide which convention to follow.
3. **Event emission:** Fireproof org hierarchy has NO outbox/events. Platform convention requires outbox events for CRUD operations. Would need to add.
4. **Migration ownership:** The tables (facilities, buildings, zones) reference `tenants(id)` — platform would need equivalent tenant FK or use string tenant_id pattern.

### Recommendation: ADAPT-PATTERN

Don't extract the code directly. Instead, when the platform needs physical hierarchy (likely Phase E or when a second vertical onboards), build a new `organization-hierarchy` platform module using Fireproof's schema and invariants as the reference design. Estimated effort: ~400 LOC new module (simplified from 1,165 by using platform conventions, Guard→Mutation→Outbox pattern, UUID IDs).

### Manufacturing Roadmap Impact

- **Phase E (Maintenance Workcenter Consumption):** Workcenters could optionally reference a facility/zone, but this is not required by the current roadmap.
- **No current phase is blocked** by the absence of org hierarchy in the platform.

---

## Module 2: Storage Locations

**Source:** `/Users/james/Projects/Fireproof-ERP/crates/fireproof-erp/src/storage_location/`
**LOC:** ~746 (types: 201, repository: 189, service: 354)

### What It Does

Physical storage spots (bins, shelves, racks, cabinets, drawers, rooms) linked to the org hierarchy via `zone_id`. Features:

- **Location type taxonomy:** bin, shelf, rack, cabinet, drawer, room, other (validated on create/update)
- **Allowed item types:** per-location filter restricting what can be stored (gauges, tools, parts)
- **Hierarchy resolution:** List query JOINs through zone → building → facility to show full path
- **RBAC:** Uses `organization:read`/`organization:write` scopes
- **Deactivation:** Soft delete with count protection (org service checks active storage locations before deactivating a zone)

### Is It Generic?

**Mostly.** The `allowed_item_types` taxonomy (`gauges`, `tools`, `parts`) is gauge-lab-specific. A generic platform version would need a configurable or open-ended item type taxonomy. The location_type taxonomy (bin/shelf/rack/cabinet/drawer/room/other) is universal.

### Platform Comparison

The platform already has `modules/inventory/src/domain/locations.rs` — a simpler model:

| Feature | Platform Location | Fireproof Storage Location |
|---------|-------------------|---------------------------|
| ID type | UUID | Integer SERIAL |
| Parent | warehouse_id (flat) | zone_id → building → facility (3-level) |
| Type taxonomy | None | bin/shelf/rack/cabinet/drawer/room/other |
| Item type filter | None | allowed_item_types array |
| Display order | None | display_order integer |
| Description | Yes | Yes |
| Active/inactive | Yes | Yes |
| Uniqueness | code per (tenant, warehouse) | code per tenant |

The platform Location is simpler but functional. Adding Fireproof's richer features (type taxonomy, item type filter, hierarchy resolution) would enhance it.

### What Changes Would Be Needed for Extraction

1. **Decouple from org hierarchy:** The `zone_id` FK means storage locations can't exist without the full Facility → Building → Zone hierarchy. For platform extraction, either:
   - A) Extract org hierarchy first (heavy dependency)
   - B) Replace zone_id with a generic `parent_id` + `parent_type` pattern, or just `warehouse_id` to match existing platform convention
2. **Generalize item types:** Replace hardcoded `gauges/tools/parts` with an open taxonomy or per-tenant configuration
3. **Platform conventions:** UUID IDs, outbox events, Guard→Mutation→Outbox pattern

### Recommendation: ADAPT-PATTERN

The platform's existing Location model is adequate for current needs. When storage location richness is needed (likely when Fireproof goes live and needs to track which bins hold which gauges), the Fireproof schema serves as the reference design. The location_type taxonomy and allowed_item_types pattern should be adopted, but adapted to platform conventions (UUID IDs, outbox events, configurable taxonomies).

Estimated effort to enhance platform Location: ~200 LOC additions to existing module (add location_type, allowed_item_types, display_order columns + validation).

### Manufacturing Roadmap Impact

- **No current phase requires storage locations.** The manufacturing roadmap tracks inventory at the warehouse level (financial transactions).
- **Future value:** When Fireproof's vertical layer needs to answer "where is gauge SP-001A physically?", the storage location model is essential. But this is a Fireproof concern, not a platform concern today.

---

## Module 3: Inventory Movement

**Source:** `/Users/james/Projects/Fireproof-ERP/crates/fireproof-erp/src/inventory_movement/`
**LOC:** ~630 (types: 130, repository: 197, service: 303)

### What It Does

Physical movement tracking with two core tables:

1. **MovementRecord** — Immutable, append-only evidence of an item moving between storage locations. Fields: entity_type, entity_id, from_location_id (nullable for first move), to_location_id, quantity, reason, moved_by, timestamp.

2. **CurrentLocation** — Mutable projection: one row per (tenant, entity_type, entity_id) showing where the item is right now. Updated via upsert.

Key design properties:
- **Atomic transaction:** Movement record + current_location update happen in a single transaction. If either fails, both roll back.
- **Entity type validation:** gauge, tool, part (hardcoded)
- **Quantity constraints:** Gauges and tools must be quantity=1. Parts allow variable quantity.
- **History query:** Flexible filtering by entity_type, entity_id, location_id, with limit
- **"Items at location" query:** What's currently stored at a given spot

### Is It Generic?

**Almost.** The movement tracking pattern (evidence record + current state projection) is completely generic. The only gauge-specific aspect is:
- `ENTITY_TYPES` constant: `["gauge", "tool", "part"]` — should be configurable
- `QUANTITY_ONE_TYPES` constant: `["gauge", "tool"]` — serialized/individual items vs. bulk items, also configurable
- RBAC uses `gauges:read` / `gauges:update` scopes — should use `inventory:read` / `inventory:write`

### Platform Comparison

**The platform has NO equivalent.** This is a fundamentally different concept from platform Inventory:

| Concern | Platform Inventory | Fireproof Movement |
|---------|-------------------|-------------------|
| What it tracks | Financial transactions (receipts, issues, transfers) with cost layers | Physical location of individual items |
| Granularity | Item + warehouse aggregate quantities | Individual entity tracking by ID |
| Movement | Transfer = paired ledger entries (debit/credit) between warehouses | Movement = evidence record of physical relocation |
| Current state | On-hand quantity per (item, warehouse) | Current location per (entity_type, entity_id) |
| Cost | FIFO cost layers, weighted average | No cost — pure location tracking |

These two models are **complementary, not conflicting.** Platform Inventory answers "how many of item X do we have and what did they cost?" while Fireproof Movement answers "where exactly is item Y right now?"

### What Changes Would Be Needed for Extraction

1. **Generalize entity types:** Replace hardcoded `["gauge", "tool", "part"]` with configurable taxonomy per tenant or module. The quantity-one vs. variable-quantity distinction should be driven by the entity type definition, not hardcoded.
2. **Decouple from storage_location FK:** The `to_location_id` references `storage_locations(id)`. For platform use, it could reference the platform's `locations` table instead, or accept any location identifier.
3. **Platform conventions:** UUID IDs (currently integer SERIAL for storage locations, BIGSERIAL for movements), outbox events for movement records, Guard→Mutation→Outbox pattern.
4. **RBAC scopes:** Replace `gauges:read`/`gauges:update` with `inventory:read`/`inventory:write` or a new `movements:read`/`movements:write`.
5. **Event emission:** Currently no outbox events. Platform extraction should emit `item_moved` events to NATS for downstream consumers (e.g., quality inspection could listen for items arriving at quarantine locations).

### Recommendation: EXTRACT

This should become a new platform capability — either a new `inventory-movement` module or a new domain within the existing Inventory module. The Fireproof code provides a clean, well-tested reference implementation.

**Extraction plan:**
1. Create `movement` domain within `modules/inventory/` (types, repository, service)
2. Replace integer IDs with UUIDs, entity_type constants with configurable taxonomy
3. Add outbox event emission for movements (item_moved, item_initial_placement)
4. Wire HTTP routes: `POST /api/inventory/movements`, `GET /api/inventory/movements/current/:entity_type/:entity_id`, `GET /api/inventory/movements/history`
5. Migration: `current_locations` + `inventory_movements` tables in inventory DB

**LOC estimate:** ~500 LOC new code (types ~100, repository ~150, service ~200, HTTP handlers ~50), adapted from Fireproof's 630 LOC by using platform conventions and removing gauge-specific constraints.

### Manufacturing Roadmap Impact

- **Phase B (Production):** The component issue workflow could use movement tracking to record which physical storage location parts were pulled from. Not required by current roadmap, but would add traceability.
- **Phase C1 (Receiving Inspection):** Quarantine/hold locations could be modeled as storage locations. Movement from receiving dock → quarantine → approved stock would create an audit trail.
- **Phase E (Maintenance):** Tool crib management (which tools are checked out, where are they) is exactly this pattern.
- **Fireproof go-live:** This is a **hard requirement** for Fireproof — they need to track where every gauge and tool is physically located. Without this in the platform, Fireproof must keep its own copy.

---

## Cross-Module: How Storage Locations Map to Production Workcenters

The bead asks how storage locations relate to workcenters. Here's the analysis:

**Current state:** Production workcenters (code, name, capacity, cost_rate_minor) have NO physical location reference. They are abstract work execution points.

**Natural mapping:** A workcenter IS (or contains) a physical zone or set of storage locations. For example:
- Workcenter "CNC-01" might be Zone "Machine Shop A" in Building "Manufacturing" at Facility "Main Plant"
- Within that zone, individual tools and gauges are at specific storage locations (bins, racks)

**Recommendation:** Don't force this mapping now. The current roadmap doesn't need it. When Phase E (Maintenance Workcenter Consumption) arrives, workcenters could optionally reference a zone_id or facility_id, but this is a Phase E concern.

---

## Summary of Recommended Beads

| # | Bead Title | Priority | Phase | Est LOC |
|---|-----------|----------|-------|---------|
| 1 | Extract inventory movement tracking into platform | P2 | Pre-Fireproof go-live | ~500 |
| 2 | Enhance platform Location model with type taxonomy | P3 | Pre-Fireproof go-live | ~200 |
| 3 | Build organization hierarchy platform module | P3 | Phase E or second vertical | ~400 |

Bead 1 is the highest value because it's a genuinely new capability the platform lacks, it's directly needed for Fireproof go-live, and it complements (not conflicts with) existing Inventory. Beads 2 and 3 can wait — the existing Location and flat warehouse model are sufficient for the manufacturing roadmap.

---

## Appendix: Dependency Graph

```
Organization Hierarchy (Fireproof)
  └─ Storage Locations (depends on zones)
       └─ Inventory Movement (depends on storage_locations)
            └─ Current Location (projection of movements)

Platform Inventory (existing)
  └─ Warehouses (flat)
       └─ Locations (simple bins within warehouse)
            └─ Transfers (financial, between warehouses)
```

The two models run in parallel. Platform Inventory is the financial truth (costs, quantities). Fireproof Movement is the physical truth (locations, evidence trail). A future integration point would be: when a movement crosses warehouse boundaries, also create an inventory transfer.
