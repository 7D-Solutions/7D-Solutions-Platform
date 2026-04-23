# health — Revision History

> **What this file is:** The complete record of every change to this module after it was proven. Agents modifying this module must add a row here before committing. Products adopting a new version read this file to understand what changed.
> **Standard:** See `docs/VERSIONING.md` for the rules governing this file.


## 1.3.0
- feat: operational vitals types — VitalsResponse, DlqVitals, OutboxVitals, ProjectionVitals, ConsumerVitals for the platform vitals API ([bd-fpulo])

## 1.2.1
- chore: rustfmt reflow + regenerate typed clients (no behavior change)

## Required fields

Every row in the Revisions table must have these fields filled in (no placeholders):

| Field | Column | Requirement |
|-------|--------|-------------|
| Version | Version | Exact SemVer matching the package file |
| Date | Date | ISO date YYYY-MM-DD, not the literal placeholder |
| Bead | Bead | Active bead ID (not bd-xxxx) |
| Summary | What Changed | Concrete — name endpoints, fields, events, behaviors. Not "TODO" or "various improvements." |
| Why | Why | The problem solved or requirement fulfilled. Not "TODO." |
| Proof | (Gate 1) | `scripts/proof_health.sh` must exist before 1.0.0 is committed. |
| Compatibility | Breaking? | "No" if consumers are unaffected. "YES: <migration note>" if breaking. Never empty. |

## Revisions

| Version | Date | Bead | What Changed | Why | Breaking? |
|---------|------|------|-------------|-----|-----------|
| 1.2.0 | 2026-04-15 | bd-jrwlc.1 | Add CircuitBreakerInfo struct to public API, referenced by platform-sdk circuit-breaker metrics. | Type was referenced in platform-sdk but never defined; gap left over from bd-wpxqn. | No |
| 1.1.0 | 2026-04-13 | bd-swikj | Added `TenantReadinessCheck` trait, `TenantReadinessRegistry` (Arc<Mutex<HashSet>> impl), `TenantReadiness` struct, `TenantReadyStatus` enum. Added optional `tenant: Option<TenantReadiness>` field to `ReadyResponse` (skipped when absent). `build_ready_response` initializes `tenant: None`. 11 new tests (5 unit registry + 6 route contract). | GAP-31: `/api/ready?tenant_id=` support for per-module tenant provisioning probes — unblocks GAP-16 activate_tenant polling. | No — `tenant` field is `skip_serializing_if = None`; all existing callers unaffected. |
| 1.0.0 | 2026-03-28 | bd-1icrg | Initial promotion. Proven: `healthz()` liveness probe, `build_ready_response` readiness builder, `ready_response_to_axum` HTTP adapter, `db_check`/`db_check_with_pool`/`nats_check` dependency helpers, `PoolMetrics` observability struct. 22 tests (6 unit + 16 contract). | Canonical health endpoints used by all platform services — stabilize public API for v1 consumers. | No |

## How to read this table

- **Version:** The version in the package file (`Cargo.toml` or `package.json`) after this change.
- **Date:** When the change was committed.
- **Bead:** The bead ID that tracked this work.
- **What Changed:** A concrete description of the change. Name specific endpoints, fields, events, or behaviors affected. Do not write "various improvements" or "minor fixes."
- **Why:** The reason the change was necessary. Reference the problem it solves or the requirement it fulfills.
- **Breaking?:** `No` if existing consumers are unaffected. `YES` if any consumer must change code to handle this version. If YES, include a brief migration note or reference a migration guide.

## Rules

- Add a new row for every version bump. One row per version.
- Do not edit old rows. If a previous change is reversed, add a new row explaining the reversal.
- The commit that bumps the version in the package file must also add the row here. Same commit.
- If the change is breaking (MAJOR version bump), the "Breaking?" column must describe what consumers need to change.