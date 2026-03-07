# Retro Run #028 — 2026-03-07

**Trigger:** count-based (5 closes since last retro)
**Analysis window:** 7 closes since retro #027 (seq 548-554)
**Runner:** PurpleCliff (bd-24s02)

## Beads Analyzed

| Bead | Title | Agent | Cycle time |
|------|-------|-------|------------|
| bd-1zst4 | RIE: Retro + CM learning — 11 bead closes | CopperRiver | ~10m |
| bd-sti2l | Stress: Auth under load — 100 concurrent logins prove rate limiting and no bypass | MaroonHarbor | ~1m |
| bd-1k8g5 | Stress: Financial double-spend — 50 concurrent allocations cannot exceed invoice balance | DarkOwl | ~4m |
| bd-xj60d | Stress: Event bus flood — 500 events prove zero data loss and DLQ correctness | PurpleCliff | ~1m |
| bd-327sl | Stress: GL double-post — 50 concurrent posts prove exactly-once journal entry | DarkOwl | ~3m |
| bd-22wio | Investigate identity-auth Dockerfile build context — health crate missing | BrightHill | ~2m |
| bd-1otyt | E2E test: sanitized DB errors + oversized body rejected + negative amounts rejected | SageDesert | ~1m |

## Signals

- **Avg cycle time:** ~3m
- **Reopen count:** 0 across 7 beads
- **Agent spread:** DarkOwl (2), BrightHill (1), CopperRiver (1), MaroonHarbor (1), PurpleCliff (1), SageDesert (1)
- **Beads with deviations:** 0
- **Theme:** Stress testing sprint — 4 of 7 beads are concurrency stress tests

## Patterns Observed

### 1. Stress tests are fast to write when the pattern is established
All four stress test beads closed in under 5 minutes each. The pattern from bd-oag8b (retro #027) — seed data, hammer concurrently, assert invariant — was reused across auth rate limiting, financial double-spend, event bus flood, and GL double-post. Once a stress test template exists, new stress tests become formulaic.

### 2. Each stress test targets a specific invariant, not generic load
Auth: rate limiter fires under 100 concurrent logins. Financial: allocations never exceed invoice balance. Event bus: 500 events with zero data loss. GL: exactly-once journal entry under 50 concurrent posts. This invariant-first approach produces sharper assertions than generic "does it survive N requests" tests.

### 3. E2E security tests validate defense-in-depth at HTTP boundary
bd-1otyt proved DB error sanitization, body size limits, and negative monetary validation all work at the real HTTP layer. This completes the security audit cycle: fix (bd-ubp52) then prove (bd-1otyt) with real databases and real HTTP, no mocks.

### 4. Docker build context issues surface late in the pipeline
bd-22wio found the health crate was outside the Docker build context for identity-auth. This is a recurring pattern — workspace-level crate dependencies break single-service Dockerfiles. The fix was already known from bd-18gf (retro #027) but reoccurred, suggesting a CI gate for Docker build verification would catch these earlier.

### 5. Retro beads themselves are lightweight operational overhead
bd-1zst4 took ~10 minutes — the longest bead in this window. All implementation beads were faster. The retro + CM extraction process works but is the slowest "bead" in the sprint. Keeping the retro window small (5-10 closes) keeps this manageable.

_See CM extraction below._
