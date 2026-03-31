# subscriptions — Revision History

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
| Proof | (Gate 1) | `scripts/proof_subscriptions.sh` must exist before 1.0.0 is committed. |
| Compatibility | Breaking? | "No" if consumers are unaffected. "YES: <migration note>" if breaking. Never empty. |

## Revisions

| Version | Date | Bead | What Changed | Why | Breaking? |
|---------|------|------|-------------|-----|-----------|
| 2.2.1 | 2026-03-31 | bd-z5rek.3 | Migrate config.rs to ConfigValidator for multi-error startup validation. All config errors reported at once in table format. | Plug-and-play wave 2: consistent startup validation across all modules. | No |
| 2.2.0 | 2026-03-30 | bd-nhmgu | Export http module from lib.rs; add openapi_dump utility binary. | OpenAPI spec validation: offline dump needed for spec extraction and validation. | No |
| 2.1.0 | 2026-03-31 | bd-f97fk | OpenAPI spec at /api/openapi.json via utoipa 5.x on execute_bill_run. Bearer JWT SecurityScheme. Split http.rs (456 LOC) into http/bill_run.rs + http/health.rs + http/mod.rs. | Plug-and-play: OpenAPI + startup standardization. | No |
| 2.0.0 | 2026-03-31 | bd-f97fk | All errors migrated from ErrorResponse/ErrorBody to ApiError (platform-http-contracts). Admin endpoints also migrated. Error responses now include request_id field. | Plug-and-play: standard response envelopes. | YES: Error format changed to ApiError (error, message, request_id). Admin errors changed from ErrorBody to ApiError. |
| 1.0.0 | 2026-03-28 | bd-4zxqk | Initial proof. Recurring billing plan lifecycle (create/activate/pause/cancel/renew), invoice generation, usage-based metering, proration, trial management, plan versioning, admin endpoints, event publishing. 29 unit tests pass, clippy clean. | Subscriptions module complete and proven. All gates pass. | No |

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
