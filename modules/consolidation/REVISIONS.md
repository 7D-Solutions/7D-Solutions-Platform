# consolidation — Revision History

> **What this file is:** The complete record of every change to this module after it was proven. Agents modifying this module must add a row here before committing. Products adopting a new version read this file to understand what changed.
> **Standard:** See `docs/VERSIONING.md` for the rules governing this file.

## Required fields

Every row in the Revisions table must have these fields filled in (no placeholders):

| Field | Column | Requirement |
|-------|--------|-------------|
| 2.2.7 | 2026-04-02 | bd-ke13e | Replace raw reqwest with PlatformClient in GL/AR/AP integration clients. Adds tenant header injection, correlation IDs, and retry on 429/503. Remove reqwest dependency. | Raw reqwest bypassed PlatformClient's tenant headers and retry logic. | No |
| 2.2.6 | 2026-04-01 | bd-o1a03 | Import extract_tenant from platform-sdk instead of local copy. | Centralized extract_tenant across all modules. | No |
| 2.2.3 | 2026-03-31 | bd-vnuvp.8 | Add tenant_id filter to 4 queries: csl_group_entities SELECT, csl_elimination_rules SELECT, csl_fx_policies DELETE, csl_trial_balance_cache DELETE. Uses subquery through csl_groups.tenant_id since child tables lack direct tenant_id column. Also threads tenant_id into cache_result function signature. | P0 tenant isolation: queries on child tables filtered only by group_id without verifying tenant ownership at the SQL level. Defence-in-depth against cross-tenant data access. | No |
| 2.2.2 | 2026-03-31 | bd-decba | Add RequirePermissionsLayer with MODULE_READ permission to all read routes. Previously, read endpoints were accessible without JWT authentication. | P0 security: aerospace/defense requires all data endpoints gated by JWT. Read routes were unprotected since initial plug-and-play rollout. | No (consumers who already provide valid JWT + read permissions are unaffected) |
| Version | Version | Exact SemVer matching the package file |
| Date | Date | ISO date YYYY-MM-DD, not the literal placeholder |
| Bead | Bead | Active bead ID (not bd-xxxx) |
| Summary | What Changed | Concrete — name endpoints, fields, events, behaviors. Not "TODO" or "various improvements." |
| Why | Why | The problem solved or requirement fulfilled. Not "TODO." |
| Proof | (Gate 1) | `scripts/proof_consolidation.sh` must exist before 1.0.0 is committed. |
| Compatibility | Breaking? | "No" if consumers are unaffected. "YES: <migration note>" if breaking. Never empty. |

## Revisions

| Version | Date | Bead | What Changed | Why | Breaking? |
|---------|------|------|-------------|-----|-----------|
| 2.2.8 | 2026-04-02 | bd-vcly8 | Delete dead health.rs stub (unreferenced after SDK conversion) | Dead code cleanup | No |
| 2.2.5 | 2026-04-01 | bd-pezs6 | Replace hand-written GL/AR/AP HTTP clients (~415 LOC) with thin adapters over platform-client-{gl,ar,ap}. Error type unified from per-module GlClientError/ArClientError/ApClientError to platform_sdk::ClientError. Response types kept locally until codegen supports typed responses. | Typed client standardisation — all inter-module HTTP calls use generated clients for type safety and consistency. | No |
| 2.2.4 | 2026-03-31 | bd-5vmu6 | Convert to platform-sdk ModuleBuilder. Replaces manual dotenv/tracing/pool/middleware/health/shutdown boilerplate with SDK startup sequence. Ops routes stripped from http::router(). | SDK batch conversion — eliminate two classes of modules. | No |
| 2.2.1 | 2026-03-31 | bd-z5rek.3 | Migrate config.rs to ConfigValidator for multi-error startup validation. All config errors reported at once in table format. | Plug-and-play wave 2: consistent startup validation across all modules. | No |
| 2.2.0 | 2026-03-30 | bd-nhmgu | Make ApiDoc pub in http/mod.rs; add openapi_dump utility binary. | OpenAPI spec validation: offline dump needed for spec extraction and validation. | No |
| 1.0.0 | 2026-03-28 | bd-1mhtm | Initial proof. Group/entity/COA-mapping/FX-policy/elimination-rule CRUD with validation, consolidation engine (per-entity TB fetch, COA mapping, FX translation, cross-entity aggregation, intercompany elimination), deterministic input hashing, csl_trial_balance_cache (DELETE+INSERT idempotent), consolidated BS/PL from cache, admin endpoints, tenant isolation, intercompany matching engine. 30 unit tests, 33 integration tests (real Postgres). | Multi-entity consolidation module complete and proven. All gates pass. | No |
| 2.1.0 | 2026-03-30 | bd-gazt3 | Plug-and-play conversion. All 5 list endpoints return `PaginatedResponse<T>` (`{data, pagination}`). All errors use `ApiError` from `platform-http-contracts` with `request_id` from `TracingContext`. Removed inline `ErrorBody`. OpenAPI via utoipa 5.x on all 26 handlers with Bearer JWT `SecurityScheme`, `/api/openapi.json` route. Auto-migrations: `sqlx::migrate!()` added to `main.rs`. Admin endpoints converted to `ApiError`. | Standardize consolidation module to match plug-and-play pattern (Inventory/Party/BOM). Consumers can parse standard envelopes, generate TS clients from spec, deploy to fresh DB without manual migration. | YES: List endpoints return `{data, pagination}` instead of bare arrays. Error responses return `{error, message, request_id}` instead of `{error, message}`. |

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
