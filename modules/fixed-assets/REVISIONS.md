# fixed-assets — Revision History

> **What this file is:** The complete record of every change to this module after it was proven. Agents modifying this module must add a row here before committing. Products adopting a new version read this file to understand what changed.
> **Standard:** See `docs/VERSIONING.md` for the rules governing this file.

## Required fields

Every row in the Revisions table must have these fields filled in (no placeholders):

| Field | Column | Requirement |
|-------|--------|-------------|
| Version | Version | Exact SemVer matching the package file |
| Date | Date | ISO date YYYY-MM-DD, not the literal placeholder |
| Bead | Bead | Active bead ID (not bd-xxxx) |
| Summary | What Changed | Concrete — name endpoints, fields, events, behaviors. Not "TODO" or "various improvements." |
| Why | Why | The problem solved or requirement fulfilled. Not "TODO." |
| Proof | (Gate 1) | `scripts/proof_fixed_assets.sh` must exist before 1.0.0 is committed. |
| Compatibility | Breaking? | "No" if consumers are unaffected. "YES: <migration note>" if breaking. Never empty. |

## Revisions

| Version | Date | Bead | What Changed | Why | Breaking? |
|---------|------|------|-------------|-----|-----------|
| 2.0.0 | 2026-03-30 | bd-70nin | Standard response envelopes: all handlers use `ApiError` and `PaginatedResponse` from platform-http-contracts. Tenant extraction returns `ApiError`. List endpoints (categories, assets, runs, disposals) wrapped in `PaginatedResponse`. `From<XError> for ApiError` impls for AssetError, DepreciationError, DisposalError. `ToSchema` derives on all domain models. | Consistent error/pagination contract for plug-and-play consumers. | YES: Response shape changed — list endpoints now return `{data, pagination}` envelope; error responses now return `{error, message, request_id}` envelope. |
| 1.0.0 | 2026-03-28 | bd-1lvg0 | Initial proof. Asset register CRUD, depreciation schedule generation (straight-line, 12-period), depreciation run execution, idempotent schedule generation, disposal workflow, asset categorization, admin endpoints, event publishing. 63 unit tests pass, clippy clean. | Fixed assets module complete and proven. All gates pass. | No |

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
