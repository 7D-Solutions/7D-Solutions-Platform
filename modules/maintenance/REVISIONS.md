# maintenance — Revision History

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
| Proof | (Gate 1) | `scripts/proof_maintenance.sh` must exist before 1.0.0 is committed. |
| Compatibility | Breaking? | "No" if consumers are unaffected. "YES: <migration note>" if breaking. Never empty. |

## Revisions

| Version | Date | Bead | What Changed | Why | Breaking? |
|---------|------|------|-------------|-----|-----------|
| 2.1.1 | 2026-03-31 | bd-z5rek.3 | Migrate config.rs to ConfigValidator for multi-error startup validation. All config errors reported at once in table format. | Plug-and-play wave 2: consistent startup validation across all modules. | No |
| 1.0.0 | 2026-03-28 | bd-19t7c | Initial proof. Preventive maintenance scheduling, asset management CRUD, calibration events, downtime tracking with impact classification, work order lifecycle, admin endpoints, outbox event publishing, idempotent operations. 63 unit tests pass, clippy clean. | Maintenance module code complete and proven. All gates pass. | No |
| 2.1.0 | 2026-03-30 | bd-ox08o | OpenAPI via utoipa: /api/openapi.json route, SecurityAddon (Bearer JWT), openapi_dump binary. | Machine-readable API spec for consumers and code generation. | No |
| 2.0.0 | 2026-03-30 | bd-ox08o | Standard response envelopes: all handlers migrated to ApiError with request_id, all list endpoints return PaginatedResponse with page/page_size/total_items/total_pages. ErrorBody removed. Count queries added to repos. ToSchema derives on all response types. | Consistent API error and pagination contracts across all endpoints. | YES: list endpoints now use page/page_size params instead of limit/offset; response shape changed from bare arrays to {data, pagination} envelope; error responses now include request_id field. |

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
