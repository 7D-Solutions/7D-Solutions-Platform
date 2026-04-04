# numbering — Revision History

> **What this file is:** The complete record of every change to this module after it was proven. Agents modifying this module must add a row here before committing. Products adopting a new version read this file to understand what changed.
> **Standard:** See `docs/VERSIONING.md` for the rules governing this file.

## Required fields

Every row in the Revisions table must have these fields filled in (no placeholders):

| Field | Column | Requirement |
|-------|--------|-------------|
| 2.1.4 | 2026-04-01 | Import extract_tenant from platform-sdk instead of local copy (bd-o1a03) |
| Version | Version | Exact SemVer matching the package file |
| Date | Date | ISO date YYYY-MM-DD, not the literal placeholder |
| Bead | Bead | Active bead ID (not bd-xxxx) |
| Summary | What Changed | Concrete — name endpoints, fields, events, behaviors. Not "TODO" or "various improvements." |
| Why | Why | The problem solved or requirement fulfilled. Not "TODO." |
| Proof | (Gate 1) | `scripts/proof_numbering.sh` must exist before 1.0.0 is committed. |
| Compatibility | Breaking? | "No" if consumers are unaffected. "YES: <migration note>" if breaking. Never empty. |

## Revisions

| Version | Date | Bead | What Changed | Why | Breaking? |
|---------|------|------|-------------|-----|-----------|
| 2.2.2 | 2026-04-04 | bd-0clpi | SoC: extract allocate + confirm SQL into db/ repos | Separation of concerns — handler files mixed HTTP logic with raw SQL queries | No |
| 2.2.1 | 2026-04-04 | bd-85tso | Replace tenant_id.parse().expect() with ApiError::bad_request on 3 request paths | Unwrap on user-supplied input causes panic (500) instead of returning 400 Bad Request. | No |
| 2.2.0 | 2026-04-02 | bd-binuj | Remove dead health.rs (health/ready/version/schema_version handlers). SDK ModuleBuilder provides these endpoints; the file was unreferenced dead code. | Dead code cleanup — annotation audit revealed health.rs handlers were never mounted after SDK conversion. | No |
| 2.1.3 | 2026-03-31 | bd-5vmu6 | Convert to platform-sdk ModuleBuilder. Replaces manual dotenv/tracing/pool/bus/outbox/middleware/health/shutdown boilerplate with SDK startup sequence. Bus and outbox publisher now configured via module.toml. | SDK batch conversion — eliminate two classes of modules. | No |
| 2.1.2 | 2026-03-31 | bd-z5rek.3 | Migrate config.rs to ConfigValidator for multi-error startup validation. All config errors reported at once in table format. | Plug-and-play wave 2: consistent startup validation across all modules. | No |
| 2.1.1 | 2026-03-30 | bd-nhmgu | Add openapi_dump utility binary for offline spec generation. | OpenAPI spec validation: offline dump needed for spec extraction and validation. | No |
| 1.0.0 | 2026-03-28 | bd-1icsq | Initial proof. Gap-free sequence allocation, numbering policy CRUD, format templates (year/prefix/padding), sequence counter with advisory lock, multi-tenant isolation, admin endpoints. 13 unit tests pass, clippy clean. | Numbering module complete and proven. All gates pass. | No |
| 2.1.0 | 2026-03-31 | bd-intvn | Plug-and-play conversion. All 4 handlers (allocate, confirm, upsert_policy, get_policy) migrated from inline `ErrorResponse` to `ApiError` with `request_id` from `TracingContext`. OpenAPI via utoipa 5.x on all handlers with Bearer JWT `SecurityScheme`, `/api/openapi.json` route. Added `platform-http-contracts` dependency. | Standardize numbering module to match plug-and-play pattern. Consumers get standard error envelopes and machine-readable OpenAPI spec. | YES: Error responses return `{error, message, request_id}` instead of `{error, message}`. Error codes changed (e.g. `database_error` → `internal_error`). |

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
