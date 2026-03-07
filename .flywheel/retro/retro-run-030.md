# Retro Run #030 — 2026-03-07

**Trigger:** count-based (7 closes since last retro)
**Analysis window:** 7 closes (retro bead bd-3lp21)
**Runner:** DarkOwl (bd-3lp21)

## Beads Analyzed

| Bead | Title | Agent | Cycle time |
|------|-------|-------|------------|
| bd-3etyq | HTTP smoke: Inventory Lots + Serials + Reservations | unknown | ~10m |
| bd-3ocpg | HTTP smoke: Maintenance Work Orders | unknown | ~8m |
| bd-2jc8g | HTTP smoke: GL RevRec + Accruals | BrightHill | ~5m |
| bd-38zco | HTTP smoke: Production Work Orders + Operations | unknown | ~8m |
| bd-3956t | HTTP smoke: Notifications | unknown | ~6m |
| bd-mgpz4 | HTTP smoke: AP POs + Bills + Payment Runs | unknown | ~7m |
| bd-3ud6r | HTTP smoke: Fixed Assets (13 routes) | DarkOwl | ~12m |

## Signals

- **Avg cycle time:** ~8m
- **Reopen count:** 0 across 7 beads
- **Agent spread:** DarkOwl (1), BrightHill (1), unknown (5)
- **Theme:** HTTP smoke test sprint — all 7 beads are HTTP boundary tests proving route wiring + auth enforcement

## Patterns Observed

### 1. Smoke test sprint saturating the pool
All 7 analyzed beads are HTTP smoke tests. This is a focused, systematic verification sprint covering the entire platform surface. The pattern is: one bead per service module, each covering 13–17 routes.

### 2. Fixed Assets required careful lifecycle ordering
The Fixed Assets smoke test (bd-3ud6r) exposed that disposal tests need status='draft' assets, and depreciation schedule generation requires in_service_date + useful_life_months to be set at creation time. These domain invariants are not obvious from the route signatures alone — they require reading the service layer.

### 3. Hook blocking from missing env vars (workflow friction)
DarkOwl's session for bd-3ud6r was blocked by the hook server because BEADS_ACTOR and AGENT_RUNNER_BEAD were not set as process env vars. The bead was correctly claimed in the beads system (in_progress, assignee=DarkOwl) but the hook still blocked writes. This is a known friction point for interactive Claude Code sessions not launched via agent-runner.sh.

### 4. assert_unauth body must be structurally valid
A recurring pattern: sending an empty or invalid body to a mutation route without JWT can return 422 (validation error) instead of 401 (auth error) if validation fires before auth middleware. Always send a minimally valid body to assert_unauth to ensure the 401 test is meaningful.

### 5. Cargo.toml [[test]] entry required for every new smoke test
Every new test file needs both the .rs file and the [[test]] entry in Cargo.toml. The file compiles fine with cargo build but the test never runs in CI without the entry. This was caught by the compile-before-commit practice.

## CM Rules Extracted

10 rules added to CM playbook (batch id: b-mmgqww72):
- testing: lifecycle order for smoke test asset creation
- testing: draft asset required for disposal tests
- integration: in_service_date required for depreciation schedule
- testing: JWT probe gate before running smoke test body
- testing: structurally valid body for assert_unauth
- testing: full CRUD lifecycle in one test function
- integration: generate schedule before creating depreciation run
- workflow: commit after clean compile, before live test run
- testing: verify [[test]] entry in Cargo.toml for every new test file
- workflow: hook server checks process env vars only, not tracking file
