# timekeeping — Revision History

> **What this file is:** The complete record of every change to this module after it was proven. Agents modifying this module must add a row here before committing. Products adopting a new version read this file to understand what changed.
> **Standard:** See `docs/VERSIONING.md` for the rules governing this file.

## Required fields

Every row in the Revisions table must have these fields filled in (no placeholders):

| Field | Column | Requirement |
|-------|--------|-------------|
| 2.1.6 | 2026-04-01 | Import extract_tenant from platform-sdk instead of local copy (bd-o1a03) |
| 2.1.3 | 2026-03-31 | bd-decba | Add RequirePermissionsLayer with MODULE_READ permission to all read routes. Previously, read endpoints were accessible without JWT authentication. | P0 security: aerospace/defense requires all data endpoints gated by JWT. Read routes were unprotected since initial plug-and-play rollout. | No (consumers who already provide valid JWT + read permissions are unaffected) |
| Version | Version | Exact SemVer matching the package file |
| Date | Date | ISO date YYYY-MM-DD, not the literal placeholder |
| Bead | Bead | Active bead ID (not bd-xxxx) |
| Summary | What Changed | Concrete — name endpoints, fields, events, behaviors. Not "TODO" or "various improvements." |
| Why | Why | The problem solved or requirement fulfilled. Not "TODO." |
| Proof | (Gate 1) | `scripts/proof_timekeeping.sh` must exist before 1.0.0 is committed. |
| Compatibility | Breaking? | "No" if consumers are unaffected. "YES: <migration note>" if breaking. Never empty. |

## Revisions

| Version | Date | Bead | What Changed | Why | Breaking? |
|---------|------|------|-------------|-----|-----------|
| 2.1.5 | 2026-03-31 | bd-5vmu6 | Convert to platform-sdk ModuleBuilder. Replaces manual dotenv/tracing/pool/middleware/health/shutdown boilerplate with SDK startup sequence. Ops routes stripped from http::router(). | SDK batch conversion — eliminate two classes of modules. | No |
| 2.1.4 | 2026-03-31 | bd-vnuvp.9 | Add app_id filter to tk_billing_run_entries query via tk_billing_runs subquery. load_run_entries now takes app_id parameter. | P0 tenant isolation sweep: queries must filter by tenant_id to prevent cross-tenant data leakage. | No |
| 2.1.2 | 2026-03-31 | bd-z5rek.3 | Migrate config.rs to ConfigValidator for multi-error startup validation. All config errors reported at once in table format. | Plug-and-play wave 2: consistent startup validation across all modules. | No |
| 2.1.1 | 2026-03-30 | bd-nhmgu | Add openapi_dump utility binary for offline spec generation. | OpenAPI spec validation: offline dump needed for spec extraction and validation. | No |
| 1.0.0 | 2026-03-28 | bd-28v5u | Initial proof. Time entry CRUD, timesheet lifecycle, approval workflow, labor costing, overtime calculations, project/work-order allocation, admin endpoints, event publishing, multi-tenant isolation. 64 unit tests pass, clippy clean. | Timekeeping module complete and proven. All gates pass. | No |
| 2.0.0 | 2026-03-30 | bd-qtry2 | Response envelopes: 11 list endpoints wrapped in PaginatedResponse (employees, projects, entries, approvals, pending, allocations, exports, rates, rollup-by-project, rollup-by-employee, rollup-by-task). 3 sub-collections wrapped in {data:[]}. All handlers return Result<…, ApiError> — replaced inline json!() error construction with platform-http-contracts ApiError. Admin endpoints migrated to ApiError. Removed ErrorBody (admin_types.rs). Added utoipa ToSchema to all domain model structs and enums. | Plug-and-play: consumers get consistent paginated envelopes and structured error responses across all timekeeping endpoints. | YES: All list endpoints now return `{data:[], pagination:{…}}` instead of bare arrays. Error responses changed from ad-hoc JSON to `{error, message, request_id?, details?}`. Consumers must update response parsing. |
| 2.1.0 | 2026-03-30 | bd-w9g06 | OpenAPI spec via utoipa on all 44 handler functions (employees, projects, tasks, entries, approvals, allocations, rollups, exports, billing). Added /api/openapi.json endpoint. ToSchema on all request types (CreateEmployeeRequest, UpdateEmployeeRequest, CreateProjectRequest, etc.) and remaining model types (BillingRun, BillingLineItem, BillingRunResult, ExportArtifact). ApiDoc struct with SecurityAddon for Bearer JWT. | Machine-readable API specification for client codegen, documentation, and contract testing. | No |

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
