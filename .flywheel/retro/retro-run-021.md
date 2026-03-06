# Retro Run #021 — 2026-03-06

**Trigger:** count-based (6 closes since last retro)
**Analysis window:** 6 closes since retro 020 (retro_seq 451–456)
**Runner:** CopperRiver (manual — run-retro.sh not found, bd-5wpej)

## Beads Analyzed

| Bead | Title | Agent | Commits | Key pattern |
|------|-------|-------|---------|-------------|
| bd-1bcvd | Phase B: Production workcenters master data (table + CRUD + events) | SageDesert | 1 | Master data CRUD |
| bd-2aem0 | Phase B: Work order lifecycle (create/release/close) with correlation chain | SageDesert | 1 | State machine + correlation |
| bd-1jo88 | Phase B: Routing templates + routing operations (schema + CRUD + events) | SageDesert | 1 | Immutability guards |
| bd-31f07 | Phase B: Operation execution model (start/complete) tied to routing ops | SageDesert | 1 | Sequence enforcement |
| bd-2kv4l | Fix auth route conflict: SOD policy GET/DELETE at same path | BrightHill | 1 | Route conflict fix |
| bd-3fjnp | Investigate Fireproof ERP reuse: Events infrastructure + DLQ + idempotency | DarkOwl | 1 | Investigation + prototype |

## Signals

- **Closes in window:** 6
- **Avg commits per bead:** 1.0
- **Agent spread:** SageDesert (4), BrightHill (1), DarkOwl (1)
- **Zero-commit beads:** 0
- **Child beads spawned:** 0
- **Phase B domain beads average:** ~1,140 LOC across 9 files each

## Patterns Observed

### 1. Phase B domain entities follow a strict dependency chain — build in order
SageDesert closed 4 Phase B beads in rapid succession, each building on the previous: workcenters → work orders (references workcenter) → routings (references workcenter via steps) → operations (references routing steps + work orders). This dependency chain was the right order — each entity could reference the ones before it. When building related domain entities, map the dependency graph first and build bottom-up.

### 2. Correlation chain is the standard for manufacturing entity lifecycles
Work orders introduced a correlation_id column with idempotent creation (duplicate correlation_id returns existing record). All subsequent events in the work order lifecycle carry the same correlation_id. Operations inherit their work order's correlation_id. This correlation chain pattern should be applied to any entity that spawns a chain of related events across multiple domain objects.

### 3. Immutability guards prevent modification of released/finalized entities
Routing templates block updates and step additions after being released. The guard is a simple status check at the top of the mutation function, returning a clear error before touching any data. This pattern should apply to any entity with a release gate (BOMs, work orders, inspection plans) — once released, the record becomes read-only except for explicit status transitions.

### 4. Sequence enforcement requires tracking predecessors and optional/required flags
Operation execution enforces that required predecessor operations must complete before the next can start, while optional operations can be skipped. This is implemented via sequence_number ordering and an is_required flag. The pattern is reusable for any sequential workflow where some steps are mandatory and others optional.

### 5. Axum route conflicts arise from same-shape path parameters with different semantics
bd-2kv4l fixed a collision between GET /sod-policies/{action_key} and DELETE /sod-policies/{rule_id}. Axum cannot distinguish path segments that differ only in parameter name. The fix: move one endpoint to a sub-path (/by-action/{action_key}). When designing HTTP routes, never rely on parameter name alone to differentiate endpoints at the same path depth.

### 6. Investigation beads that also prototype are more valuable than pure analysis
bd-3fjnp investigated Fireproof's event consumer infrastructure but also built a working receipt_event_bridge consumer in quality-inspection (201 LOC + 231 LOC test). This prototype proved the concept and gave subsequent consumer beads a concrete reference implementation. Investigation beads should produce a working prototype when the scope allows, not just a document.

### 7. One-commit-per-bead is achievable when scope is well-defined
All 6 beads were single-commit. The Phase B beads each delivered a complete domain entity (model + repo + events + HTTP + tests) in one clean commit. This is only possible because the bead scope was precise: one entity, one concern, complete vertical slice. Well-scoped beads enable atomic commits.

### 8. Version bumps required for proven module fixes
bd-2kv4l (auth route fix) included a version bump in identity-auth Cargo.toml and a REVISIONS.md entry, as required by the versioning policy for proven (>=1.0.0) modules. This is the correct pattern — even small fixes need version tracking on proven modules.
