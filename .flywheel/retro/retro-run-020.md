# Retro Run #020 — 2026-03-05

**Trigger:** count-based (6 closes since last retro)
**Analysis window:** 6 closes since retro 019 (retro_seq 445–450)
**Runner:** SageDesert (manual — run-retro.sh not found, bd-sqeld)

## Beads Analyzed

| Bead | Title | Agent | Commits | Key pattern |
|------|-------|-------|---------|-------------|
| bd-2f1xv | Phase C1: quality-inspection-rs scaffold + Docker/CI wiring | DarkOwl | 1 | New module scaffold |
| bd-1y2nc | Phase C1: Inspection plan model + receiving inspection record core | DarkOwl | 1 | Domain logic + tests |
| bd-2g7el | Phase A: Integration proof (Inventory retrofit + BOM core) end-to-end | CopperRiver | 1 | Cross-module e2e tests |
| bd-1mgdw | Phase A: bom-rs Docker/CI container + compose watch wiring | PurpleCliff | 2 | Docker/CI wiring |
| bd-16fy6 | Phase C1: Quarantine/hold + disposition outcomes with events | DarkOwl | 1 | State machine + events |
| bd-2ya5w | Phase B: production-rs scaffold + Docker/CI wiring | SageDesert | 2 | New module scaffold |

## Signals

- **Closes in window:** 6
- **Avg commits per bead:** 1.3
- **Agent spread:** DarkOwl (3), CopperRiver (1), PurpleCliff (1), SageDesert (1)
- **Zero-commit beads:** 0
- **Child beads spawned:** 0
- **Port conflict fixes:** 1 (bd-2ya5w needed a second commit)

## Patterns Observed

### 1. Port conflicts are the #1 scaffold hazard — check the service catalog first
bd-2ya5w (production-rs) initially used port 8106 for the service and 5460 for Postgres. Both were already taken (quality-inspection-rs on 8106, fireproof-postgres on 5460). This required a second commit to fix. The PLATFORM-SERVICE-CATALOG.md exists but wasn't consulted before choosing ports. New module scaffolds should check docker-compose.*.yml and the service catalog for port allocations before writing config.

### 2. Module scaffolds follow a consistent template — 18-20 files, ~730 LOC
Both bd-2f1xv (quality-inspection-rs) and bd-2ya5w (production-rs) produced nearly identical scaffolds: Cargo.toml, Dockerfile.workspace, initial migration, config.rs, db/resolver.rs, domain/outbox.rs, events/mod.rs, http/health.rs, http/tenant.rs, lib.rs, main.rs, metrics.rs, plus compose entries and CI jobs. The pattern is well-established and can be followed mechanically.

### 3. Integration proof beads belong at phase boundaries — they catch cross-module gaps
bd-2g7el was a dedicated integration proof bead at the end of Phase A. It created 721 LOC of e2e tests covering all exit criteria (BOM explosion, production receipts, purchase receipt regression, depth guards). This is the right place to catch gaps: after all Phase A beads landed, before Phase B started. Cross-module tests at phase boundaries provide confidence that independently-built modules actually work together.

### 4. State machines should emit events on every transition, not just creation
bd-16fy6 added 4 event types for disposition transitions (held, released, accepted, rejected). Each transition emits an event with the inspector, reason, and before/after states. This makes the full lifecycle auditable and lets downstream consumers react to disposition changes, not just initial creation. Financial consumers (GL cost adjustment for rejected goods) depend on these transition events.

### 5. New domain fields on inspections need matching indexes for query patterns
bd-1y2nc added receipt_id, part_id, and part_revision to inspections and created indexes for "query by receipt" and "query by part+revision". These index-backed query patterns are critical for the receiving inspection workflow where multiple inspections reference the same receipt or part. Always add indexes when adding FK-like columns that will be queried in list/filter operations.

### 6. Docker/CI beads should be separate from domain logic beads
bd-1mgdw (BOM Docker/CI) was a separate bead from the BOM core logic bead (bd-1uy2l). This separation worked well: the Docker/CI bead discovered a port conflict (8098 already used by party-rs) and fixed it without touching any domain code. Keeping infrastructure wiring separate from domain logic reduces the blast radius of both.
