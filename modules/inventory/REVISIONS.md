# inventory — Revision History

> **What this file is:** The complete record of every change to this module after it was proven. Agents modifying this module must add a row here before committing. Products adopting a new version read this file to understand what changed.
> **Standard:** See `docs/VERSIONING.md` for the rules governing this file.


## 2.7.3
- chore: rustfmt reflow + regenerate typed clients (no behavior change)

## Required fields

Every row in the Revisions table must have these fields filled in (no placeholders):

| Field | Column | Requirement |
|-------|--------|-------------|
| 2.4.9 | 2026-04-01 | Import extract_tenant from platform-sdk instead of local copy (bd-o1a03) |
| Version | Version | Exact SemVer matching the package file |
| Date | Date | ISO date YYYY-MM-DD, not the literal placeholder |
| Bead | Bead | Active bead ID (not bd-xxxx) |
| Summary | What Changed | Concrete — name endpoints, fields, events, behaviors. Not "TODO" or "various improvements." |
| Why | Why | The problem solved or requirement fulfilled. Not "TODO." |
| Proof | (Gate 1) | `scripts/proof_inventory.sh` must exist before 1.0.0 is committed. |
| Compatibility | Breaking? | "No" if consumers are unaffected. "YES: <migration note>" if breaking. Never empty. |

## Revisions

| Version | Date | Bead | What Changed | Why | Breaking? |
|---------|------|------|-------------|-----|-----------|
| 2.7.2 | 2026-04-14 | bd-5ea4y.1 | Add structured fields to bare tracing::error! calls in HTTP handler files (imports.rs). Error vars surfaced via `error = %e`. | Structured logging standard (bd-5ea4y) requires at least one field before the message string in all HTTP handler log calls. CI check-log-fields.sh now passes. | No |
| 2.7.1 | 2026-04-14 | bd-07qp4 | Replace the inventory admin router unit test's fake `PgPool::connect_lazy("postgres://localhost/fake")` pool with a real test database helper that loads `DATABASE_URL` or falls back to the local inventory Postgres URL and runs `db/migrations` before building the router. | The compile-only admin router test should exercise the same database setup as the rest of inventory's integration coverage instead of relying on a fake connection string. | No |
| 2.6.2 | 2026-04-10 | bd-s56d3 | Add `[dev-dependencies]`: `http-body-util`, `uuid` (v4), `security` (path). Required for integration tests that validate auth middleware with real JWT tokens against the inventory HTTP router. | Integration tests for bd-s56d3 (e2e test suite) need to construct HTTP requests with bearer tokens and typed response bodies without pulling these crates into production builds. | No |
| 2.6.1 | 2026-04-04 | bd-bxngm | SoC: wire valuation, expiry, labels, cycle_count, status repo modules | Separation of concerns — complete repo module wiring for remaining inventory domain services | No |
| 2.6.0 | 2026-04-04 | bd-yrmmq,bd-bxngm | SoC: extract SQL from receipt, reservation, transfer, cycle_count, expiry, labels, status, valuation services into repo modules (69 queries total) | Separation of concerns — GL exemplar pattern | No |
| 2.5.0 | 2026-04-02 | bd-fh6u1 | Add utoipa::path annotations to 3 health endpoints (health, ready, version). All 59 handlers now annotated. | OpenAPI spec completeness — codegen requires typed annotations on every endpoint. | No |
| 2.4.10 | 2026-04-02 | bd-p9n1w | Replace `extract_tenant` + match boilerplate with `TenantId` Axum extractor in all 5 item handlers (create, get, update, list, deactivate). Pilot for platform-wide tenant context middleware. | Manual `extract_tenant` in every handler is error-prone — a missed call leaks cross-tenant data. `TenantId` extractor makes tenant extraction automatic and returns 401 before the handler runs. | No |
| 2.4.8 | 2026-04-01 | bd-9c1mo | Add `#[utoipa::path]` to 4 missing handlers: `projection_status`, `consistency_check`, `list_projections` (admin), `post_batch_receipts` (batch receipts). Made admin handlers pub. Added `ToSchema` derives to batch receipt types. Registered all 4 paths in ApiDoc. | OpenAPI spec was missing 4 routed endpoints, breaking spec-to-router parity. | No |
| 2.4.7 | 2026-03-31 | bd-5vmu6.3 | Convert main.rs to platform-sdk ModuleBuilder. Replace bus supervisor with SDK bus init. SDK handles DB, bus, outbox (inv_outbox), CORS, JWT, health, metrics. Consumers started via bus_arc(). Created module.toml. Added platform-sdk dep. SLO metrics registered with global registry. | SDK batch conversion — eliminate two classes of modules. | No |
| 2.4.6 | 2026-03-31 | bd-vnuvp.9 | Add tenant_id filter to inventory_reservations reversal-existence check in fulfill_service. | P0 tenant isolation sweep: queries must filter by tenant_id to prevent cross-tenant data leakage. | No |
| 2.4.5 | 2026-03-30 | bd-41dpi | Fix response envelope conformance: `list_uoms` and `list_conversions` now return `PaginatedResponse` instead of bare arrays. `get_serials_for_item` returns `PaginatedResponse` instead of `{"serials":[...]}`. Sub-resource endpoints (`lots`, `labels`, `revisions`, `reorder-policies`, `valuation-snapshots`, `locations`) switched from `limit/offset` to `page/page_size` query params. | All list endpoints must use the standard `{data, pagination}` envelope with `page/page_size` query params per platform contract. | YES: Sub-resource list endpoints now accept `page`/`page_size` instead of `limit`/`offset`. Serials response shape changed from `{"serials":[...]}` to `{data:[...], pagination:{...}}`. |
| 2.4.4 | 2026-03-30 | bd-2scog | Fix items list pagination: `ListItemsQuery` now accepts `page`/`page_size` instead of `limit`/`offset`. Consumers sending `?page_size=5` now get 5 results (was ignored, always 50). Default page_size remains 50. | Consumers could not control page size — `page_size` param was silently dropped. | YES: `ListItemsQuery` fields renamed from `limit`/`offset` to `page`/`page_size`. Internal callers constructing this struct directly must update. |
| 2.4.3 | 2026-03-30 | bd-of3dw | `cargo fmt` reformat of openapi_dump.rs and items_repo.rs — import ordering and line wrapping only. | Linter auto-format after prior commits. | No |
| 2.4.2 | 2026-03-30 | bd-of3dw.1 | Fix `ListItemsQuery` IntoParams: added `#[into_params(parameter_in = Query)]` so search/tracking_mode/make_buy/active/limit/offset appear as query params in the OpenAPI spec, not path params. | Generated TS client would send list filters as URL path segments instead of query string, breaking pagination and search. | No |
| 2.7.0 | 2026-04-13 | bd-zwf9n | Add `POST /api/inventory/import/items` bulk import endpoint. Accepts CSV or JSON, validates all rows before writing (item_code, name required; tracking_mode must be none/lot/serial), idempotent upsert by SKU, 10K row limit, transactional. | Onboarding: customers need to bulk-load item master data during initial setup. | No |
| 2.4.1 | 2026-03-30 | bd-of3dw | Add `openapi_dump` utility binary: generates OpenAPI JSON spec to stdout without needing database or NATS. Used by TS client codegen pipeline (`cargo run --bin openapi_dump > openapi.json`). | Running service binary requires DB/NATS to start; codegen needs the spec offline. | No |
| 2.4.0 | 2026-03-30 | bd-9a1jj | OpenAPI via utoipa 5.x: `#[utoipa::path]` on all 50 handlers, ToSchema on all types, SecurityAddon (Bearer JWT), `/api/openapi.json` route, per-handler INVENTORY_READ/INVENTORY_MUTATE declarations, idempotency 201/200/409 documented. Fixed pre-existing compile error from bd-rbhj1 (stubbed unimplemented start_outbox_publisher). | Consumers reverse-engineering endpoints from source; spec enables automatic TS client generation. | No |
| 2.3.0 | 2026-03-30 | bd-rbhj1 | BUS_TYPE/NATS_URL config + AppState now keeps an `Arc<dyn EventBus>` and bus health, event bus supervisor starts the component issue + FG receipt consumers and the inv_outbox publisher, /api/ready reports both DB and NATS health. | Consumers get graceful NATS wiring: HTTP comes up even when the bus is down, readiness shows degradation, and events queue in the outbox until the NATS connection recovers. | No |
| 2.2.0 | 2026-03-30 | bd-9a1jj | OpenAPI via utoipa 5.x: `#[utoipa::path]` annotations on all 50 HTTP handlers, `ToSchema` derives on all request/response/domain types, `SecurityAddon` with Bearer JWT scheme, per-handler security declarations (INVENTORY_READ / INVENTORY_MUTATE), `/api/openapi.json` endpoint serving the full spec. Idempotency semantics (201/200/409) documented per-endpoint. Tenant identity documented as JWT-derived (no X-Tenant-Id header). | Consumers reverse-engineering endpoints from source code; OpenAPI spec eliminates this and enables automatic client generation. | No |
| 2.1.0 | 2026-03-30 | bd-2x9uf | Startup validation collector: Config::from_env() now collects ALL invalid/missing env vars and reports them at once instead of failing on the first error. Auto-migrations: sqlx::migrate!("./db/migrations") runs at startup after pool creation, enabling empty-DB-to-ready path. | Deploying with partial config previously required fixing one env var at a time and restarting to discover the next; now all problems are reported in a single startup failure. Migrations must run automatically so fresh deployments don't require a manual migration step. | No |
| 2.0.0 | 2026-03-29 | bd-dehre | Standard response envelopes: all 7 list endpoints migrated from ad-hoc JSON to `PaginatedResponse<T>` (`{data:[...],pagination:{page,page_size,total_items,total_pages}}`). All handler error responses migrated from inline `json!()` to `ApiError` from platform-http-contracts (`{error,message,request_id,details}`). `request_id` populated from TracingContext on all error responses. 27 domain error enums get `From<XxxError> for ApiError`. `extract_tenant` returns `ApiError`. Idempotency documented: POST with idempotency_key → 201 (created) / 200 (replay) / 409 (conflict). | Consumers deserializing responses must update: list endpoints return `{data,pagination}` instead of `{items,total,limit,offset}` or bare arrays; error responses return `{error,message,request_id}` instead of inline JSON. | YES: See consumer migration guide in README.md — every list endpoint envelope changed, error envelope changed. |
| 1.0.1 | 2026-03-29 | bd-7c0t1 | Split 3 oversize domain files: revisions.rs (1013 LOC) → revisions/{models,service,queries}.rs; issue_service.rs (877 LOC) → issue/{types,service,idempotency}.rs; adjust_service.rs (637 LOC) → adjust/{types,service}.rs. All files ≤500 LOC. Public API unchanged via re-exports in mod.rs. | Enforce 500 LOC CI limit; prepare for utoipa annotation work. | No |
| 1.0.0 | 2026-03-28 | bd-1qw2e | Initial proof. Item CRUD with search/filter/pagination, stock receipts (FIFO layers, lot/serial tracking, idempotency), issues (FIFO cost drain), transfers, adjustments (positive/negative with guard), reservations, cycle counts (submit/approve), batch receipts endpoint, reorder policies, UOM management, location tracking, expiry monitoring, genealogy/trace, label generation, make/buy classification, revision management, valuation engine (FIFO/WAC/snapshot), low-stock alerts, status transitions, outbox atomicity, event contract publishing, admin endpoints, DLQ replay drill. 237 unit tests pass. Integration tests blocked by known pg_hba.conf infrastructure issue (not code). | Inventory module code complete and unit-tested. Clippy clean. | No |

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