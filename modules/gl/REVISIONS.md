# gl — Revision History

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
| Proof | (Gate 1) | `scripts/proof_gl.sh` must exist before 1.0.0 is committed. |
| Compatibility | Breaking? | "No" if consumers are unaffected. "YES: <migration note>" if breaking. Never empty. |

## Revisions

| Version | Date | Bead | What Changed | Why | Breaking? |
|---------|------|------|-------------|-----|-----------|
| 2.1.2 | 2026-03-31 | bd-zznx6.1 | Fix NUMERIC→BIGINT cast in invariants.rs aggregate queries. COALESCE(SUM()) returns Postgres NUMERIC which cannot decode to Rust i64. Added ::BIGINT casts to assert_all_entries_balanced query. | Financial accuracy tests exposed ColumnDecode error when unbalanced entries exist. | No |
| 2.1.1 | 2026-03-31 | bd-z5rek.3 | Migrate config.rs to ConfigValidator for multi-error startup validation. All config errors reported at once in table format. | Plug-and-play wave 2: consistent startup validation across all modules. | No |
| 2.1.0 | 2026-03-31 | bd-m1dh1 | OpenAPI spec via utoipa on all 34 handler functions. Added /api/openapi.json endpoint with Bearer JWT SecurityScheme. ApiDoc struct in http/mod.rs lists all paths. Tags: Accounts, Accruals, Financial Statements, Period Summary, GL Detail, Exports, FX Rates, Reporting Currency, Period Close, Close Checklist, Revenue Recognition. | Machine-readable API specification for client codegen, documentation, and contract testing. | No |
| 2.0.2 | 2026-03-31 | bd-yg2mv | Complete full ApiError migration: auth.rs (extract_tenant returns ApiError, add with_request_id), admin.rs (3 endpoints), accruals.rs (3 handlers), and all 10 remaining report/query handlers (trial_balance, balance_sheet, income_statement, cashflow, period_summary, account_activity, gl_detail, exports, fx_rates, reporting_currency, accounts). All custom XxxErrorResponse types removed. | Completes the v2.0.0 migration for the remaining 14 handler files that still used local error types and old extract_tenant tuple pattern. | No |
| 2.0.1 | 2026-03-31 | bd-yg2mv | Complete ApiError migration: add platform-http-contracts dep with axum feature, migrate revrec.rs (4 handlers), period_close.rs (6 handlers), close_checklist.rs (6 handlers) from local ErrorResponse/PeriodCloseHttpError to ApiError with TracingContext request_id. Remove dead admin_types.rs (ErrorBody). | Finishes v2.0.0 migration — 3 files were missed in previous commit due to missing Cargo.toml dependency. | No |
| 2.0.0 | 2026-03-31 | bd-yg2mv | Standard response envelopes. All custom ErrorResponse/ErrorBody/XxxErrorResponse types replaced with platform ApiError. request_id populated via TracingContext on every error path. extract_tenant returns ApiError. All 11 consumers untouched. 17 handler files migrated. | Plug-and-play wave 2: platform-wide error contract conformance for GL module. | YES — error response shape changed from ad-hoc `{error: string}` to `{error, message, request_id}`. Consumers parsing error responses must update. |
| 1.0.1 | 2026-03-31 | bd-uxity | Split oversize files by separation of concerns. revrec_repo → contract/schedule/amendment repos. accruals → accruals + accruals_reversal. Consumer files (gl_inventory, ar_tax_liability, fixed_assets_depreciation) → consumer wiring + posting logic. balance_sheet_service allowlisted (single cohesive concern). Fixed pre-existing unwrap panics in accrual posting paths. All re-exports preserved. | Internal refactor to enforce 500 LOC limit without breaking consumers. | No |
| 1.0.0 | 2026-03-28 | bd-gxt1h | Initial proof. Double-entry journal posting with atomic balance updates. Period close validation + snapshot hashing. Trial balance, balance sheet, income statement services. Revenue recognition scheduling. Accruals + reversals. FX conversion with banker's rounding. Event consumers: AR tax, AP bills, inventory, credit notes, writeoffs, FX realized, labor costs, fixed assets depreciation. DLQ with retry. Multi-tenant boundary isolation. | Platform GL foundation complete. All invariants enforced (balanced entries, no duplicates, valid accounts, closed period protection, reversal chain depth). Proof script passing. | No |

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
