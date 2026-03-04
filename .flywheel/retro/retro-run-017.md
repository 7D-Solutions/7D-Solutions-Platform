# Retro Run #017 — 2026-03-04

**Trigger:** count-based (5 closes since last retro)
**Analysis window:** 6 closes since retro 016 (retro_seq 424–429)
**Runner:** CopperRiver (manual — run-retro.sh not found, bd-1odbc)

## Beads Analyzed

| Bead | Title | Agent | Commits | Key pattern |
|------|-------|-------|---------|-------------|
| bd-jxtc7 | RIE: Retro + CM learning — 6 bead closes | MaroonHarbor | 0 | Prior retro run (meta bead) |
| bd-34lec | Fix auth-lb and gateway nginx cap_drop | MaroonHarbor | 2 | Docker security capabilities |
| bd-qtge3 | Update consumer guide with all Phase 57+67 APIs and events | MaroonHarbor | 1 | Documentation completeness |
| bd-2fuow | Update OpenAPI contract YAMLs with Phase 67 extension endpoints | BrightHill | 1 | Contract synchronization |
| bd-2wahd.1 | Manufacturing modules scope review (PurpleCliff) | PurpleCliff | 1 | Independent review |
| bd-3bcwo | Manufacturing modules scope review (CopperRiver) | CopperRiver | 1 | Independent review |

## Signals

- **Closes in window:** 6
- **Avg commits per bead (code beads only):** 2 (bd-34lec: 2, others: 1 each)
- **Agent spread:** MaroonHarbor (3), BrightHill (1), PurpleCliff (1), CopperRiver (1)
- **Reopen count:** 0
- **Zero-commit beads:** 1 (bd-jxtc7 retro meta)

## Patterns Observed

### 1. cap_drop: ALL requires explicit cap_add for nginx privilege drops
bd-34lec revealed that `cap_drop: ALL` removes SETGID/SETUID capabilities that nginx needs to fork worker processes under a non-root user. The initial fix added SETGID/SETUID, then a second commit added CHOWN for the cache directory. When hardening containers with cap_drop: ALL, enumerate every capability the process actually uses — nginx needs at minimum SETGID, SETUID, and CHOWN.

### 2. Contract documentation drifts silently during feature phases
bd-qtge3 and bd-2fuow were both catch-up beads created because Phase 67 extension endpoints went undocumented in both the consumer guide and OpenAPI YAMLs. WhiteValley hit 404-like confusion because they symlink directly to our contracts/ directory. The lesson: feature beads should include contract/doc updates in scope, not as afterthoughts. If the API surface changes, the contract YAML changes in the same commit.

### 3. Split docs vs contracts into separate beads when scope creeps
bd-qtge3 originally covered both the consumer guide AND contract YAMLs but the scope was too large. BrightHill spun off bd-2fuow to handle just the OpenAPI YAMLs. This is the one-bead-one-concern principle applied correctly — when a bead's scope grows beyond what one agent can cleanly close, split it rather than doing a sloppy combined job.

### 4. Independent reviews surface different risks when agents work from the same brief
bd-2wahd.1 and bd-3bcwo were independent reviews of the same manufacturing proposal. PurpleCliff and CopperRiver each had different concerns — this multi-reviewer pattern catches blind spots that a single reviewer would miss. The key: same brief, no cross-talk during review, synthesis afterward.

### 5. Retro beads should be fast — don't block the pipeline
bd-jxtc7 was a retro bead that MaroonHarbor closed quickly, allowing them to move on to the real work (bd-34lec, bd-qtge3). Retro beads are maintenance, not feature work — process them briskly and extract the learning without over-analyzing.
