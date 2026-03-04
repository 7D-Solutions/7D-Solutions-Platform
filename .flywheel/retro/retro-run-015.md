# Retro Run #015 — 2026-03-04

**Trigger:** count-based (5 closes since last retro)
**Analysis window:** 7 closes since retro 014 (retro_seq 410–416)
**Runner:** PurpleCliff (manual — run-retro.sh not found, bd-2n4ai)

## Beads Analyzed

| Bead | Title | Agent | Commits | Key pattern |
|------|-------|-------|---------|-------------|
| bd-1qztq | RIE: Retro + CM learning — 5 bead closes | CopperRiver | 1 | Prior retro run (meta bead) |
| bd-fzruv | Fix container naming from image pinning | BrightHill | 0 | Hotfix for side-effect of compose override naming |
| bd-3h3c7 | Pin deploy images — compose build to image + release manifest | MaroonHarbor | 3 | Production overlay with immutable image tags |
| bd-1etu5 | Push bd-3h3c7 commits to remote | MaroonHarbor | 0 | Git push coordination |
| bd-yeg06 | Non-root containers + drop Linux capabilities | CopperRiver | 0 | Security hardening across Dockerfiles |
| bd-1abes | Clean up working tree — commit pending changes | MaroonHarbor | 5 | Housekeeping — TLS infra, Cargo.lock, proof artifacts |
| bd-2pbf6 | Postgres TLS enablement for all services | PurpleCliff | 3 | TLS certs, sslmode=require across all DATABASE_URLs |

## Signals

- **Closes in window:** 7
- **Avg commits per bead (code beads only):** 2.75 (bd-3h3c7: 3, bd-2pbf6: 3, bd-1abes: 5, bd-1qztq: 1)
- **Agent spread:** MaroonHarbor (2), CopperRiver (2), PurpleCliff (1), BrightHill (1), (bd-1qztq prior retro: CopperRiver)
- **Reopen count:** 0
- **Zero-commit beads:** 3 (bd-fzruv hotfix, bd-1etu5 push, bd-yeg06 Dockerfiles — may have been committed under different bead prefix)

## Patterns Observed

### 1. Compose override files prevent drift between dev and production
bd-3h3c7 used a production overlay (`docker-compose.production.yml`) to pin images rather than modifying the base `docker-compose.services.yml`. This prevents dev/prod drift — developers keep using `build:` locally while production uses `image:` with pinned tags. The release manifest (`scripts/release-manifest.sh`) generates the env file with version tags, and rollback is documented. This overlay pattern should be the standard approach for any production-specific config.

### 2. Image pinning can cause container naming side-effects
bd-fzruv was a hotfix created because the image pinning work in bd-3h3c7 changed container naming conventions (prefixed names). This is a compose behavior quirk — when switching from `build:` to `image:`, Docker Compose may generate different container names. Always verify `docker compose ps` after changing compose structure, and use explicit `container_name:` directives to prevent naming surprises.

### 3. TLS enablement requires sweeping ALL connection strings
bd-2pbf6 and bd-1abes together show that enabling Postgres TLS requires updating every DATABASE_URL across: (a) docker-compose env vars, (b) test configuration, (c) CI scripts, and (d) documentation. bd-2pbf6 found hardcoded `sslmode=disable` in test URLs that needed updating to `sslmode=require`. When enabling TLS on any service, grep for all connection strings — including test fixtures — to ensure nothing bypasses encryption.

### 4. Housekeeping beads bundle unrelated pending changes safely
bd-1abes was a cleanup bead that committed TLS infrastructure, Cargo.lock updates, and proof runbook artifacts in one pass. This pattern — creating a small "clean up working tree" bead — prevents pending changes from accumulating and causing confusion for other agents. When multiple beads leave uncommitted artifacts, a housekeeping bead keeps the tree clean.

### 5. Security hardening (non-root, TLS, image pinning) benefits from parallel execution
Three security beads ran concurrently: bd-yeg06 (non-root), bd-2pbf6 (TLS), bd-3h3c7 (image pinning). Each touched different file sets (Dockerfiles, compose data files, compose services files) so they didn't conflict. Phase 66's gap-analysis-driven approach — creating independent beads from specific findings — enabled parallel execution without coordination overhead.

### 6. Retro beads without run-retro.sh work fine manually
Both bd-1qztq (retro 014) and this bead (bd-2n4ai, retro 015) were completed manually because `run-retro.sh` doesn't exist. The manual process (read close events, analyze commits, write report, extract CM rules) is straightforward. The script isn't blocking — the pattern is well-documented in prior retro runs.
