# Retro Run #014 — 2026-03-04

**Trigger:** count-based (5 closes since last retro)
**Analysis window:** 6 closes since retro 013 (retro_seq 404–409)
**Runner:** CopperRiver (manual — run-retro.sh not found, bd-1qztq)

## Beads Analyzed

| Bead | Title | Agent | Commits | Key pattern |
|------|-------|-------|---------|-------------|
| bd-11sv4 | RIE: Retro + CM learning — 10 bead closes | PurpleCliff | 0 | Retro/meta bead — no code commits |
| bd-vkvll | Secrets management — replace plaintext .env in prod | MaroonHarbor | 5 | Docker secrets migration, entrypoint wrappers, deploy scripts |
| bd-1cozh | Fix crash-looping containers from migration mismatch | BrightHill | 0 | Triage bead — containers fixed without code commits |
| bd-vxeii | Check agent resource conflicts | BrightHill | 0 | Coordination — file reservation check between agents |
| bd-1fq39 | Port binding + hardcoded creds fix | CopperRiver | 2 | Legacy compose files missed by prior bead, child bead for test auth |
| bd-1he4g | Fix flaky workflow escalation timer test | CopperRiver | 1 | Concurrent cargo test interference via shared DB |

## Signals

- **Closes in window:** 6
- **Avg commits per bead (code beads only):** 2.7 (bd-vkvll: 5, bd-1fq39: 2, bd-1he4g: 1)
- **Agent spread:** CopperRiver (2), BrightHill (2), PurpleCliff (1), MaroonHarbor (1)
- **Reopen count:** 0
- **Zero-commit beads:** 3 (retro, triage, coordination — all valid non-code work)

## Patterns Observed

### 1. Infrastructure changes must cover ALL compose file variants
bd-1fq39 existed because bd-1atj1 fixed port binding in the primary compose files (data/services) but missed the legacy files (infrastructure/platform/modules). The same hardcoded credentials bug existed in docker-compose.platform.yml even after bd-1atj1 fixed it in docker-compose.services.yml. When making infrastructure changes, always grep across all compose files to find every instance — never assume a single file is sufficient.

### 2. Shared databases cause flaky tests under concurrent cargo build slots
bd-1he4g revealed that when multiple cargo test processes run in parallel (via build slots), they share the same database. A global `tick()` function queried ALL tenants, meaning one test's tick picked up another test's timers. The fix was adding `tick_for_tenant()` that scopes queries by tenant_id. Any test that operates on shared infrastructure (timers, queues, scheduled jobs) must scope operations to the test's own tenant or namespace to prevent cross-test interference.

### 3. Secrets migration requires entrypoint wrapper pattern
bd-vkvll implemented Docker secrets by creating an entrypoint wrapper script that reads secret files and exports them as environment variables before exec'ing the original entrypoint. This avoids changing application code — services still read env vars. The production compose overlay mounts secrets and overrides entrypoints. This pattern (secrets → env vars at container startup) is reusable for any Dockerized service.

### 4. Child beads catch cascading failures from infrastructure changes
bd-1fq39 spawned child bead bd-1fq39.1 when test suites broke because they hardcoded NATS URLs without credentials. After adding NATS auth (bd-1atj1), every test file that manually constructed NATS connections needed updating. Creating child beads for discovered failures keeps the parent bead's scope clean while ensuring nothing is skipped.

### 5. File reservations prevent multi-agent conflicts on shared files
bd-vxeii was a coordination bead where BrightHill checked whether CopperRiver and SageDesert were conflicting on docker-compose.services.yml. The file reservation system exists precisely for this — agents should proactively reserve files they're editing via `reserve-files.sh` before starting work.

### 6. Crash-looping containers from migration mismatches need systematic diagnosis
bd-1cozh addressed 13 containers crash-looping with VersionMissing migration errors. When containers reference migrations that no longer exist in the codebase (removed or renamed), the _sqlx_migrations table becomes inconsistent. The fix is either restoring the migration file or cleaning the migration table. Always verify migration consistency after rebasing or merging branches that touch migration files.
