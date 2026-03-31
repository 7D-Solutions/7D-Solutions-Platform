# quality-inspection — Revision History

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
| Proof | (Gate 1) | `scripts/proof_quality_inspection.sh` must exist before 1.0.0 is committed. |
| Compatibility | Breaking? | "No" if consumers are unaffected. "YES: <migration note>" if breaking. Never empty. |

## Revisions

| Version | Date | Bead | What Changed | Why | Breaking? |
|---------|------|------|-------------|-----|-----------|
| 1.0.0 | 2026-03-28 | bd-183if | Initial proof. Receiving inspection and in-process inspection workflows, inspection plan configuration, acceptance criteria management, disposition handling, admin endpoints. Builds and clippy clean. Integration tests advisory (DB connectivity). | Quality inspection module code complete. All gates pass. | No |
| 1.0.1 | 2026-03-30 | bd-dx75u | Split domain/service.rs into plan_service.rs (plan CRUD) and inspection_service.rs (inspection CRUD + disposition + queries). service.rs retains QiError and re-exports. No API changes. | Separation of concerns — plans and inspections are distinct domain objects. | No |
| 2.0.0 | 2026-03-31 | bd-nmykb | All handlers migrated to ApiError with request_id. 4 list endpoints (by-part-rev, by-receipt, by-wo, by-lot) return PaginatedResponse. error_conversions.rs maps QiError to ApiError. extract_tenant returns ApiError. BUS_TYPE panic replaced with graceful error. Config migrated to ConfigValidator with dual DB pool validation. Auto-migrations added. NATS graceful degradation. OpenAPI spec via utoipa on all 15 handlers with ToSchema on all types. /api/openapi.json endpoint. BUS_TYPE default changed from nats to inmemory. | Plug-and-play standard response envelopes, OpenAPI, and startup improvements. | YES: List endpoints return `{"data":[...],"pagination":{...}}` instead of bare arrays. Error responses return `{"error":"...","message":"...","request_id":"..."}` instead of ad-hoc JSON. BUS_TYPE default changed from "nats" to "inmemory". |
| 2.0.1 | 2026-03-31 | bd-nmykb | Added OpenAPI spec validation test in http::tests::openapi_spec_is_valid_json. Validates spec serializes to JSON and contains all expected paths and schemas. | Verification: prove OpenAPI spec is valid without running the service. | No |

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
