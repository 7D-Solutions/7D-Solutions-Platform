# reporting — Revision History

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
| Proof | (Gate 1) | `scripts/proof_reporting.sh` must exist before 1.0.0 is committed. |
| Compatibility | Breaking? | "No" if consumers are unaffected. "YES: <migration note>" if breaking. Never empty. |

## Revisions

| Version | Date | Bead | What Changed | Why | Breaking? |
|---------|------|------|-------------|-----|-----------|
| 2.1.1 | 2026-03-31 | bd-5vmu6 | Convert to platform-sdk ModuleBuilder. Replaces manual dotenv/tracing/pool/middleware/health/shutdown boilerplate with SDK startup sequence. | SDK batch conversion — eliminate two classes of modules. | No |
| 2.1.0 | 2026-03-31 | bd-xyz19 | OpenAPI via utoipa + ConfigValidator: added #[utoipa::path] annotations to all 8 handlers (pl, balance-sheet, cashflow, ar-aging, ap-aging, kpis, forecast, rebuild), ToSchema derives to all response types, IntoParams derives to all query param structs, /api/openapi.json route, openapi_dump binary. Migrated config.rs from raw env::var to ConfigValidator. | Plug-and-play alignment: machine-readable API spec and validated startup config. | No |
| 2.0.0 | 2026-03-31 | bd-xyz19 | Standard response envelopes: replaced ErrorBody with ApiError (platform-http-contracts) across all handlers (rebuild, projection-status, consistency-check, list-projections, ar-aging, ap-aging, cashflow, forecast, kpis, pl, balance-sheet). All error responses now include request_id from TracingContext. Removed admin_types.rs. Added tenant.rs (extract_tenant + with_request_id helpers). | Plug-and-play alignment: consistent error envelope with request_id on every error path. | YES: error responses change shape from `{"error","message"}` to `{"error","message","request_id"}`. Consumers parsing error bodies must update. |
| 1.0.0 | 2026-03-28 | bd-2e24e | Initial proof. Trial balance, income statement, balance sheet, cash flow, AR/AP aging reports, KPI dashboard, PDF export, report caching (rpt_*_cache tables), event-driven cache invalidation, projection consistency checks, admin endpoints, multi-tenant isolation. 98 unit tests pass, clippy clean. | Reporting module complete and proven. All gates pass. | No |

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
