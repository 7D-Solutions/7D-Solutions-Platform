# fixed-assets — Revision History

> **What this file is:** The complete record of every change to this module after it was proven. Agents modifying this module must add a row here before committing. Products adopting a new version read this file to understand what changed.
> **Standard:** See `docs/VERSIONING.md` for the rules governing this file.


## 2.1.10
- chore: rustfmt reflow + regenerate typed clients (no behavior change)

## Required fields

Every row in the Revisions table must have these fields filled in (no placeholders):

| Field | Column | Requirement |
|-------|--------|-------------|
| 2.1.6 | 2026-04-01 | Import extract_tenant from platform-sdk instead of local copy (bd-o1a03) |
| 2.1.4 | 2026-03-31 | bd-5vmu6 | Convert to platform-sdk ModuleBuilder with SDK consumer adapter for ap.vendor_bill_approved. | SDK batch conversion — eliminate two classes of modules. | No |
| 2.1.3 | 2026-03-31 | bd-vnuvp.8 | Add tenant_id filter to 3 test queries: fa_ap_capitalizations SELECT (2x in ap_bill_approved and capitalize tests), fa_assets status SELECT (disposals tests). | P0 tenant isolation: test assertions queried without tenant_id, masking potential cross-tenant data leaks. | No |
| 2.1.2 | 2026-03-31 | bd-decba | Add RequirePermissionsLayer with MODULE_READ permission to all read routes. Previously, read endpoints were accessible without JWT authentication. | P0 security: aerospace/defense requires all data endpoints gated by JWT. Read routes were unprotected since initial plug-and-play rollout. | No (consumers who already provide valid JWT + read permissions are unaffected) |
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
| 2.1.9 | 2026-04-15 | bd-p3duh | Move envelope.payload by value in on_vendor_bill_approved consumer — replace serde_json::from_value(envelope.payload.clone()) with from_value(envelope.payload). | Payload was heap-copied on every dispatch even though it is consumed once. Same perf fix applied by bd-g7zzj to notifications and maintenance. | No |
| 2.1.8 | 2026-04-04 | bd-xqpmn | SoC: extract 58 sqlx queries from domain services into repo modules (assets, capitalize, depreciation, disposals) | Separation of concerns — isolate persistence from business logic so domain services are testable and DB access is centralized. | No |
| 2.1.7 | 2026-04-02 | bd-azq84 | Removed local extract_tenant (now in SDK) | Plug-and-play standardization | No |
| 2.1.6 | 2026-04-02 | bd-9v3vx | Add body= to utoipa response annotations on 13 endpoints (categories CRUD, assets CRUD, depreciation schedule/run, disposals). | OpenAPI specs were missing response schemas, causing codegen to emit Result<(), ClientError> instead of typed responses. | No |
| 2.1.5 | 2026-04-01 | bd-manm4 | Add openapi_dump binary for standalone OpenAPI spec generation to stdout. | OpenAPI spec hygiene — all modules must emit complete specs for client codegen. | No |
| 2.1.1 | 2026-03-31 | bd-z5rek.3 | Migrate config.rs to ConfigValidator for multi-error startup validation. All config errors reported at once in table format. | Plug-and-play wave 2: consistent startup validation across all modules. | No |
| 2.1.0 | 2026-03-30 | bd-70nin | OpenAPI via utoipa: `#[utoipa::path]` annotations on all 17 handlers, `ApiDoc` struct in http/mod.rs, `/api/openapi.json` endpoint serving full spec with JWT bearer security scheme. Tags: Categories, Assets, Depreciation, Disposals. | Self-describing API for plug-and-play consumer onboarding. | No |
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