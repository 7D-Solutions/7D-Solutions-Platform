# tenant-registry — Revision History

> **What this file is:** The complete record of every change to this module after it was proven. Agents modifying this module must add a row here before committing. Products adopting a new version read this file to understand what changed.
> **Standard:** See `docs/VERSIONING.md` for the rules governing this file.

## Revisions

| Version | Date | Bead | What Changed | Why | Breaking? |
|---------|------|------|-------------|-----|-----------|
| 1.0.2 | 2026-02-22 | bd-22i8 | Test fix: extracted `validate_seed_password()` sync function; replaced three concurrent async env-var tests with deterministic sync unit tests that call the validator directly; fixed `max_connections(0)` pool panic in test helper. All 76 tests pass. No production code behavior change. | Concurrent async tests raced on process-global env vars causing intermittent failures. | No |
| 1.0.1 | 2026-02-22 | bd-2t65 | Security: removed hardcoded `changeme123` default from `seed_identity_module`. Now requires `SEED_ADMIN_PASSWORD` env var; refuses to seed if unset, empty, or matching a known-bad default (changeme123, password, admin, etc.). Added `InvalidSeedPassword` error variant. E2E tests updated to supply the env var. No API surface change. | REVIEW-1 remediation — production must never ship with a deterministic default credential path. | No |
| 1.0.0 | 2026-02-22 | bd-tzsh | Initial proof. All tests passing (unit + integration against real DB). Handles tenant list/detail, plan catalog, entitlements, app-id mapping, tenant status, and tenant summary (parallel module fanout). Key routes consumed by control-plane (app-id, summary), TTP (app-id), and identity-auth (entitlements, status). Proof command: `./scripts/proof_tenant_registry.sh` | Module build complete. Phase 44 Track B promotion — second in dependency order after control-plane. | — |

## How to read this table

- **Version:** The version in the package file (`Cargo.toml`) after this change.
- **Date:** When the change was committed.
- **Bead:** The bead ID that tracked this work.
- **What Changed:** A concrete description of the change. Name specific endpoints, fields, events, or behaviors affected.
- **Why:** The reason the change was necessary.
- **Breaking?:** `No` if existing consumers are unaffected. `YES` if any consumer must change code to handle this version.

## Rules

- Add a new row for every version bump. One row per version.
- Do not edit old rows. If a previous change is reversed, add a new row explaining the reversal.
- The commit that bumps the version in the package file must also add the row here. Same commit.
- If the change is breaking (MAJOR version bump), the "Breaking?" column must describe what consumers need to change.
