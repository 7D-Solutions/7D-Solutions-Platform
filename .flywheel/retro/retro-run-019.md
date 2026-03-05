# Retro Run #019 — 2026-03-05

**Trigger:** count-based (5 closes since last retro)
**Analysis window:** 5 closes since retro 018 (retro_seq 438–442)
**Runner:** CopperRiver (manual — run-retro.sh not found, bd-1t1ne)

## Beads Analyzed

| Bead | Title | Agent | Commits | Key pattern |
|------|-------|-------|---------|-------------|
| bd-2gx4q | Ops: restart ChatGPT browser worker with Brave CDP | MaroonHarbor | 0 | Ops task, no code |
| bd-3eajx | Ops: connect ChatGPT worker to authenticated Chrome | MaroonHarbor | 0 | Ops task, no code |
| bd-187i8 | Fix: ChatGPT scripts read from ~/.flywheel/ | MaroonHarbor | 1 | Config path consolidation |
| bd-194cd | Phase A: Inventory retrofit — source_type + production receipt + make/buy | MaroonHarbor | 1 (+1 child) | Large domain feature |
| bd-2vc9u | Phase A: GL consumer — branch by source_type (WIP vs COGS) + production receipts | CopperRiver | 1 | Cross-module consumer update |

## Signals

- **Closes in window:** 5
- **Avg commits per bead (code beads only):** 1.3 (bd-194cd: 2 incl child, bd-2vc9u: 1, bd-187i8: 1)
- **Agent spread:** MaroonHarbor (4), CopperRiver (1)
- **Zero-commit beads:** 2 (bd-2gx4q, bd-3eajx — ops tasks)
- **Child beads spawned:** 1 (bd-194cd.1 — TLS fix for inventory tests)

## Patterns Observed

### 1. Large domain features spawn child beads for environment issues
bd-194cd (938 lines changed, 41 files) spawned bd-194cd.1 to fix TLS connection strings and a missing field. The parent had all domain logic correct but couldn't pass integration tests due to a pre-existing environment issue (non-TLS dev containers rejecting SSL handshakes). When a parent bead touches 27+ test files, pre-existing env issues surface as child beads. This is the system working correctly — child beads isolate env fixes from domain logic.

### 2. Test connection strings must include ?sslmode=disable for dev containers
bd-194cd.1 fixed 27 inventory test files that all lacked `?sslmode=disable` in their fallback connection strings. This caused "unexpected response from SSLRequest: 0x00" against non-TLS dev containers. Every new module or test file that connects to Postgres in dev must include this parameter in its fallback connection string.

### 3. GL consumer hard-fails on unknown source_type — correct defensive design
bd-2vc9u explicitly made the GL consumer reject unknown source_type values with a non-retriable error (DLQ'd). This prevents silent misposting: if Inventory ever adds a new source_type, GL will immediately fail loudly rather than posting to the wrong accounts. Hard-fail on unknown enum variants is the correct pattern for financial consumers.

### 4. Cross-module changes require careful sequencing with dependency beads
bd-2vc9u depended on bd-194cd (inventory retrofit must land first so GL has the new event payloads). The dependency was declared in the bead system and respected — CopperRiver started bd-2vc9u only after MaroonHarbor closed bd-194cd. This prevented integration failures from missing event fields.

### 5. Ops beads with no code commits should be typed differently
bd-2gx4q and bd-3eajx were ops tasks (restarting browser workers) with zero commits. They're valid work but leave no artifact trail. Ops tasks that involve running commands but producing no code could benefit from a summary comment or closing note to preserve the knowledge of what was done.

### 6. ChatGPT auth state should live in user home, not project dir
bd-187i8 consolidated ChatGPT auth state to `~/.flywheel/browser-profiles/chatgpt-state.json` instead of project-local. This eliminated the copy step between projects. Shared tooling config (auth, browser state) belongs in user home; project-specific config (bead state, retro counter) stays project-local.
