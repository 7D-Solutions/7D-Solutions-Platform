# gl — Revision History

> **What this file is:** The complete record of every change to this module after it was proven. Agents modifying this module must add a row here before committing. Products adopting a new version read this file to understand what changed.
> **Standard:** See `docs/VERSIONING.md` for the rules governing this file.

## Required fields

Every row in the Revisions table must have these fields filled in (no placeholders):

| Field | Column | Requirement |
|-------|--------|-------------|
| 3.0.1 | 2026-04-01 | Import extract_tenant from platform-sdk instead of local copy (bd-o1a03) |
| 2.1.4 | 2026-03-31 | bd-vnuvp.8 | Add tenant_id filter to 3 revrec idempotency queries: revrec_schedules EXISTS (schedule_repo + amendment_repo), revrec_contracts EXISTS (contract_repo). Also adds tenant_id param to contract_exists function signature. | P0 tenant isolation: idempotency checks queried by primary key only, allowing a schedule/contract ID from tenant A to satisfy the check for tenant B. | No |
| 2.1.3 | 2026-03-31 | bd-decba | Add RequirePermissionsLayer with MODULE_READ permission to all read routes. Previously, read endpoints were accessible without JWT authentication. | P0 security: aerospace/defense requires all data endpoints gated by JWT. Read routes were unprotected since initial plug-and-play rollout. | No (consumers who already provide valid JWT + read permissions are unaffected) |
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
| 3.3.1 | 2026-04-14 | bd-pfk8e | Add optional `tenant_tz` fields to the GL period-close request contracts and make period-close validation use tenant-local midnight boundaries before checking unbalanced journal entries. The validation query now converts the tenant's `period_start`/`period_end` window to UTC instants before filtering `journal_entries.posted_at`. | GAP-20 needs period-close cutoff checks to respect tenant-local boundaries instead of UTC date casts. | No |
| 3.1.2 | 2026-04-04 | bd-0clpi | SoC: extract close_checklist + period_close SQL into checklist_repo.rs and period_repo.rs | Separation of concerns — handler files mixed HTTP logic with raw SQL queries | No |
| 3.1.1 | 2026-04-02 | bd-vcly8 | Delete dead health.rs stub (unreferenced after SDK conversion) | Dead code cleanup | No |
| 3.1.0 | 2026-04-02 | bd-5b1q2 | Add POST /api/gl/journal-entries HTTP endpoint. Consolidation module needs synchronous journal entry creation (elimination entries). Handler maps HTTP request to GlPostingRequestV1, uses UUID v5 idempotency from tenant+source_doc_id. Added ToSchema derives to GlPostingRequestV1, SourceDocType, JournalLine, Dimensions. | Consolidation calls POST /api/gl/journal-entries but endpoint did not exist — entries were only created via event consumers. | No |
| 3.0.3 | 2026-04-02 | bd-azq84 | Minor utoipa annotation fixes on revrec and trial_balance | Plug-and-play standardization | No |
| 3.0.2 | 2026-04-02 | bd-9v3vx | Add body= to utoipa response annotations on 3 period reopen endpoints (request_reopen, approve_reopen, reject_reopen). | OpenAPI specs were missing response schemas, causing codegen to emit Result<(), ClientError> instead of typed responses. | No |
| 3.0.0 | 2026-04-02 | bd-161aq | Add utoipa annotations to health (3) and admin (3) handlers. Convert 3 list endpoints (get_checklist_status, get_approvals, list_reopen_requests) from Vec/Value to PaginatedResponse. Add typed ReopenRequestResponse. Register all new paths and schemas in OpenApi. | GL response standardization — every public handler now has a utoipa annotation and every list endpoint returns PaginatedResponse, matching the Consolidation/Fixed-Assets/Treasury standard. | YES — list endpoints (checklist, approvals, reopen requests) now return `{data: [...], pagination: {...}}` instead of bare arrays. Consumers must unwrap `.data` field. |
| 2.2.0 | 2026-04-01 | bd-0gdw3 | Add ToSchema derives and typed response/request bodies to all 34 OpenAPI handler annotations. Register 70+ schema types in ApiDoc components. Every endpoint now has fully typed request and response schemas in the spec. | GL openapi.json had 31 paths but only 2 schemas — endpoints compiled but responses were untyped (serde_json::Value in spec). Typed clients need real schemas for codegen. | No |
| 2.1.9 | 2026-04-01 | bd-manm4 | Add openapi_dump binary for standalone OpenAPI spec generation to stdout. | OpenAPI spec hygiene — all modules must emit complete specs for client codegen. | No |
| 2.1.8 | 2026-03-31 | bd-5vmu6.2 | Convert main.rs to platform-sdk ModuleBuilder. SDK handles DB, bus, outbox, CORS, JWT, health, metrics. 11 consumers started via bus_arc() in routes closure (retain existing retry/DLQ). SLO metrics registered with global registry. Created module.toml. Added platform-sdk dep. | SDK batch conversion — eliminate two classes of modules. | No |
| 2.1.7 | 2026-03-31 | bd-68y44 | `create_amendment` now sets `supersedes_event_id` on the `revrec.contract_modified` outbox event, linking it to the most recent prior event for the same contract. | E2E test correctly expected supersession linkage but the repo never looked up the prior event. | No |
| 2.1.6 | 2026-03-31 | bd-tbnqm.3.1 | Fix gl.period.reopened outbox event mutation_class from invalid 'period_reopen' to 'ADMINISTRATIVE'. | Mutation class registry compliance test caught invalid value — all outbox events must use one of the 7 standard mutation classes. | No |
| 2.1.3 | 2026-03-31 | bd-decba.3 | Add GL_READ permission layer to all GL read routes. Extracted read routes into separate router with RequirePermissionsLayer so trial-balance, income-statement, balance-sheet, reporting, period, detail, activity, fx-rates, and cash-flow require JWT auth. Health/ops endpoints remain unauthenticated. | GL read routes were accessible without JWT authentication — security gap found in auth/RBAC coverage audit. | No |
| 3.3.0 | 2026-04-13 | bd-y6gco | Wire platform-audit into GL mutation handlers: journal entry posting, period close. Each writes a `WriteAuditRequest` inside the existing transaction. Audit log migration creates `audit_log` table. | SOC2/compliance: financial mutations must have an append-only audit trail. | No |
| 3.2.0 | 2026-04-13 | bd-zwf9n | Add `POST /api/gl/import/chart-of-accounts` bulk import endpoint. Accepts CSV or JSON, validates all rows before writing (account_code, name, type required; type must be asset/liability/equity/revenue/expense), idempotent upsert by account_code, infers normal_balance from type, 10K row limit, transactional. | Onboarding: customers need to bulk-load their chart of accounts during initial setup. | No |
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
