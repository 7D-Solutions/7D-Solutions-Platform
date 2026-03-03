# Production Gap Analysis Brief

> **For:** Claude Desktop codebase analysis
> **Date:** 2026-03-03
> **Context:** First paying customer signed. Aerospace/defense manufacturing (Fireproof ERP).

## What This Platform Is

A modular Rust backend platform — 33 crates, PostgreSQL per service, NATS JetStream event bus, Docker Compose orchestration. Powers Fireproof ERP for aerospace/defense manufacturing.

## Current State (verified)

- **33/33 crates pass** integrated tests against real Postgres and real NATS (zero mocks, zero stubs)
- **Platform contracts validated** — event schemas and envelope compliance proven
- **NATS JetStream healthy** — AUTH_EVENTS stream with real data, DLQ empty, file persistence
- **50 Docker containers running** — all services, databases, NATS, monitoring infra
- **Proof runbook** (`scripts/proofs_runbook.sh`) captures evidence automatically

### Completed hardening (Phase 65, in progress)

| Done? | Item |
|-------|------|
| Yes | CI proof runbook — runs on main merge + nightly, gates on failures |
| Yes | JetStream backup/restore — runbook + drill script, tested PASS |
| In progress | DLQ replay drill automation |
| In progress | Outbox backlog alerting + dashboards |
| In progress | Release manifest + version tags |

## What We Need From You

Scan the actual codebase and answer these questions:

### 1. Top 5 Risks for First Customer

Look at real code, configs, docker-compose files, and test coverage. Not theoretical — what's actually exposed?

**Start with these files:**
- `docker-compose.services.yml` — all service definitions
- `docker-compose.yml` — infrastructure (NATS, Postgres instances, monitoring)
- `.env` or `.env.example` — config and secrets
- `scripts/proofs_runbook.sh` — what the test evidence covers
- `docs/VERSIONING.md` — release standard
- `Dockerfile` files in `platform/` and `modules/` directories
- `docs/PLATFORM-SERVICE-CATALOG.md` — service inventory
- `docs/security-audit-2026-02-25.md` — prior security audit
- `docs/sql-injection-audit-2026-02-25.md` — prior SQL injection audit
- `docs/DEPLOYMENT.md`, `docs/DEPLOYMENT-PRODUCTION.md` — deployment docs
- `docs/OBSERVABILITY-PRODUCTION.md` — monitoring setup
- `docs/HEALTH-CONTRACT.md` — health endpoint standard

### 2. Must-Fix vs Ship-and-Harden

For each risk: is it a blocker for go-live, or can we ship and improve later? Be honest — "nice to have" items should be labeled as such.

### 3. Minimum Work to Ship

What's the MINIMUM additional work (3-5 items max) to get from where we are to a defensible first deployment? Not a 20-item wishlist. Just what will bite us first.

## Ground Rules

- **Be direct.** What's actually dangerous vs what's nice-to-have.
- **Stay lean.** We have 5 agents ready to work. Small, focused items that can ship in hours, not days.
- **No busywork.** Don't recommend things we already have (check the docs listed above before suggesting).
- **Aerospace context matters.** Data integrity, audit trails, and traceability are non-negotiable. Flashy dashboards are not.
- **Look at the code.** Don't guess — read the actual files and tell us what you find.

## Output Format

Write your findings to `docs/gap-analysis-results.md` with:
1. A risk table (risk, severity, must-fix or defer, evidence from codebase)
2. A prioritized punch list (3-5 items, each with concrete scope)
3. Anything that surprised you (good or bad)
