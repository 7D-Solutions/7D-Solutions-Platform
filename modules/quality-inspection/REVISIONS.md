# quality-inspection — Revision History

> **What this file is:** The complete record of every change to this module after it was proven. Agents modifying this module must add a row here before committing. Products adopting a new version read this file to understand what changed.
> **Standard:** See `docs/VERSIONING.md` for the rules governing this file.


## 3.1.2
- chore: rustfmt reflow + regenerate typed clients (no behavior change)

## Required fields

Every row in the Revisions table must have these fields filled in (no placeholders):

| Field | Column | Requirement |
|-------|--------|-------------|
| 2.0.4 | 2026-04-01 | Import extract_tenant from platform-sdk instead of local copy (bd-o1a03) |
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
| 3.1.1 | 2026-04-09 | bd-qbdfs | Disposition handlers (hold/release/accept/reject) now forward the inbound bearer token to Workforce Competence HTTP calls via PlatformClient.with_bearer_token() | WC authorization endpoint requires workforce_competence.read permission; without the token, WC returns 401 and QI maps it to 503, blocking all disposition transitions | No |
| 3.1.0 | 2026-04-04 | bd-b89j6,bd-89i91,bd-5448p,bd-6ys1x | Add list_plans endpoint, extract inspection_repo.rs from inspection_service.rs (23 SQL queries), extract SQL from inspection_routes.rs (12 queries), fix test tenant_id validation + wc_client URL encoding | SoC: repo layer extraction + friction sweep list endpoint + test hardening | No |
| 3.0.0 | 2026-04-02 | bd-1f3qr | Remove direct workforce-competence DB access. QI now calls WC via HTTP PlatformClient instead of reading WC database. | Module boundary violation — QI was coupled to WC database | Yes — WORKFORCE_COMPETENCE_DATABASE_URL replaced with WORKFORCE_COMPETENCE_BASE_URL |
| 2.1.0 | 2026-04-02 | bd-binuj | Remove dead health.rs (health/ready/version handlers). SDK ModuleBuilder provides these endpoints; the file was unreferenced dead code. | Dead code cleanup — annotation audit revealed health.rs handlers were never mounted after SDK conversion. | No |
| 2.0.3 | 2026-03-31 | bd-5vmu6 | Convert to platform-sdk ModuleBuilder. Dual DB pool created in routes closure. Consumer bridges spawn via bus_arc(). | SDK batch conversion — eliminate two classes of modules. | No |
| 2.0.2 | 2026-03-30 | bd-nhmgu | Add openapi_dump utility binary for offline spec generation. | OpenAPI spec validation: offline dump needed for spec extraction and validation. | No |
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