# bom — Revision History

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
| Proof | (Gate 1) | `scripts/proof_bom.sh` must exist before 1.0.0 is committed. |
| Compatibility | Breaking? | "No" if consumers are unaffected. "YES: <migration note>" if breaking. Never empty. |

## Revisions

| Version | Date | Bead | What Changed | Why | Breaking? |
|---------|------|------|-------------|-----|-----------|
| 2.2.2 | 2026-03-31 | bd-5vmu6 | Convert to platform-sdk ModuleBuilder. Replaces manual dotenv/tracing/pool/middleware/health/shutdown boilerplate with SDK startup sequence. | SDK batch conversion — eliminate two classes of modules. | No |
| 2.2.1 | 2026-03-30 | bd-nhmgu | Add openapi_dump utility binary for offline spec generation. | OpenAPI spec validation: offline dump needed for spec extraction and validation. | No |
| 2.2.0 | 2026-03-29 | bd-pyoiy | OpenAPI spec via utoipa on all 25 BOM/ECO endpoints. Serves /api/openapi.json with Bearer JWT SecurityScheme. All request/response models derive ToSchema; query types derive IntoParams. Explosion returns flat rows (not recursive tree). Documents NUMBERING_URL dependency in spec description. | Enable typed client generation and API documentation for BOM consumers. | No |
| 2.1.0 | 2026-03-29 | bd-y4zq5 | Startup validation collector: config.rs now reports ALL env errors at once instead of stopping at the first. Added sqlx::migrate!() to main.rs for automatic DB migrations on startup. | BOM could silently skip migrations (no migrate call in main.rs) and config errors were reported one-at-a-time, slowing operator debugging. | No |
| 2.0.0 | 2026-03-29 | bd-obyfm | All list endpoints wrapped in PaginatedResponse<T> with page/page_size query params. All error responses migrated to ApiError with request_id. Added GET /api/bom list endpoint. Added ToSchema derives to all response models. Explosion and where-used remain as complete tree responses. | Standardize BOM API responses to match platform-http-contracts envelope conventions. | YES: List endpoints now return `{data, pagination}` envelope instead of bare arrays. Error responses now return `{error, message, request_id}` instead of `{error, message}`. Consumers must update response parsing for both list and error shapes. |
| 1.0.2 | 2026-03-29 | bd-t21av | Split oversize domain files: eco_service.rs → eco_service/{lifecycle,service}.rs, bom_service.rs → bom_service/{headers,lines}.rs. All files under 500 LOC. Pure move/rename, no logic changes. | CopperRiver review flagged two domain files over 500 LOC limit. | No |
| 1.0.1 | 2026-03-28 | bd-29c9i.2 | Sanitized BOM HTTP 409 duplicate responses so unique-constraint violations return a static business-safe message instead of raw PostgreSQL error text. | Security audit found that duplicate-create conflicts leaked internal constraint names and schema details to API consumers. | No |
| 1.0.0 | 2026-03-28 | bd-32crl | Initial proof. BOM header/line CRUD, revision management, multi-level explosion (recursive CTE with cycle detection and max-depth guard), scrap factor validation, quantity guards, admin endpoints. 5 unit tests pass, clippy clean. Integration tests advisory (DB connectivity). | Bill of materials module code complete and unit-tested. All gates pass. | No |

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
