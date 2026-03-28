# audit — Revision History

> **What this file is:** The complete record of every change to this module after it was proven. Agents modifying this module must add a row here before committing. Products adopting a new version read this file to understand what changed.
> **Standard:** See `docs/VERSIONING.md` for the rules governing this file.

## Revisions

| Version | Date | Bead | What Changed | Why | Breaking? |
|---------|------|------|-------------|-----|-----------|
| 1.0.0 | 2026-03-28 | bd-3nhlq | Initial proof. Append-only audit trail with field-level diffs (`Diff`), policy enforcement (`StrictImmutable`, `CompensatingRequired`, `MutableWithAudit`, `MutableStandard`), `AuditWriter` with pool and transaction modes, `Actor` identity (User/Service/System with deterministic UUIDs), outbox bridge for completeness checking and backfill. 69 tests (23 unit, 14 diff, 15 policy, 9 outbox bridge, 8 writer integration). DB triggers enforce append-only invariant. Proof command: `./scripts/proof_audit.sh`. | Platform audit is a compliance requirement for aerospace/defense (Fireproof ERP). Every mutation must have a traceable audit record. | — |

## How to read this table

- **Version:** The version in `Cargo.toml` after this change.
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
