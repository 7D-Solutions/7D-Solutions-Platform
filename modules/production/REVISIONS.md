# production — Revision History

> **What this file is:** The complete record of every change to this module after it was proven. Agents modifying this module must add a row here before committing. Products adopting a new version read this file to understand what changed.
> **Standard:** See `docs/VERSIONING.md` for the rules governing this file.


## 3.7.0 — 2026-04-17 — bd-3z1m9
- feat: direct cost accumulation for work orders
  - New DB tables: `work_order_cost_postings`, `work_order_cost_summaries`
  - `CostRepo::post_cost` — atomic posting + summary upsert in single transaction; idempotent on `source_event_id`
  - Three event consumers: `production.time_entry_approved` → labor cost (rate-based formula), `inventory.item_issued` → material cost, `outside_processing.order_closed` → OSP cost (with proration for partial acceptance)
  - Two new events: `production.cost_posted`, `production.work_order_cost_finalized` (emitted on WO close)
  - HTTP routes: POST `/work-orders/{id}/cost-postings`, GET `/work-orders/{id}/cost-summary`, GET `/work-orders/{id}/cost-postings`
  - Workcenter `cost_rate_minor` NULL → warn and skip labor posting (never zero)
  - 8 integration tests pass against real Postgres

## 3.5.3
- chore: rustfmt reflow + regenerate typed clients (no behavior change)

## Required fields

Every row in the Revisions table must have these fields filled in (no placeholders):

| Field | Column | Requirement |
|-------|--------|-------------|
| 2.3.3 | 2026-04-01 | Import extract_tenant from platform-sdk instead of local copy (bd-o1a03) |
| Version | Version | Exact SemVer matching the package file |
| Date | Date | ISO date YYYY-MM-DD, not the literal placeholder |
| Bead | Bead | Active bead ID (not bd-xxxx) |
| Summary | What Changed | Concrete — name endpoints, fields, events, behaviors. Not "TODO" or "various improvements." |
| Why | Why | The problem solved or requirement fulfilled. Not "TODO." |
| Proof | (Gate 1) | `scripts/proof_production.sh` must exist before 1.0.0 is committed. |
| Compatibility | Breaking? | "No" if consumers are unaffected. "YES: <migration note>" if breaking. Never empty. |

## Revisions

| Version | Date | Bead | What Changed | Why | Breaking? |
|---------|------|------|-------------|-----|-----------|
| 3.6.0 | 2026-04-17 | bd-ixnbs.1 | Add TimeEntryApproved event (`production.time_entry_approved`) plus `status` column on time_entries (pending/approved/rejected), POST /api/production/time-entries/:id/approve and /reject endpoints, atomic outbox-enqueue inside approval transaction, and new `production:time_entry:approve` role gate. Migration `20260417000001_time_entry_status.sql`. | Prerequisite for bd-3z1m9 Manufacturing Costing: labor cost must post after supervisor sign-off (AS9100 traceability), not on clock-out. Costing consumer listens for `time_entry_approved`, not `time_entry_stopped`. | No |
| 3.5.2 | 2026-04-14 | bd-c8253 | Fix `[platform.services]` BOM `default_url` from `http://7d-bom:8120` to `http://7d-bom:8107`. BOM listens on 8107 (PORT env override; matches service catalog and consumer guide). Production was calling the wrong port and createDraftWorkOrder returned 422 to every caller (Huber Power Phase 2 reported it via MistyCave). | Typo in the manifest — 8120 is numbering's port. No code change, just URL correction. | No |
| 3.5.1 | 2026-04-13 | bd-to4yf | ECO enforcement guard on both WO creation endpoints. `composite_create_work_order` and `create_work_order` now reject any `bom_revision_id` whose BOM revision status is `superseded`. Returns 422 with error code `BOM_REVISION_SUPERSEDED` and a message referencing the ECO number and successor revision ID. Direct-DB mode queries `eco_bom_revisions` for rich context. `CreateWorkOrderRequest.tenant_id` gains `#[serde(default)]` (matches existing `CompositeCreateWorkOrderRequest` pattern — tenant always derived from JWT). 4 new integration tests in `eco_enforcement_test.rs`. | Aerospace compliance requirement: once ECO rev B is approved, rev A cannot be built against. The guard was missing from `create_work_order` and the error was a generic `validation_error` with no ECO reference. | No |
| 3.5.0 | 2026-04-13 | bd-y6gco,bd-xiz0k | Wire platform-audit into Production mutation handlers: WO create/release/close, component issue, FG receipt. Move BOM fetch before TX in composite_create to avoid holding connections. Add BomRevisionClient (Platform/Direct/Permissive modes). Add NumberingClient::void_wo_number compensating action. Audit log migration. | SOC2 audit trail + composite WO was holding DB connections open during slow BOM HTTP calls, causing pool starvation under load. | YES: `composite_create` now requires a `&BomRevisionClient` parameter. |
| 3.4.1 | 2026-04-10 | bd-dhl7p | `create_work_order_repo()`: on `23505` unique-constraint violation, roll back and fetch the existing WO by `(tenant_id, order_number)` rather than returning `DuplicateOrderNumber` error. Makes WO creation fully idempotent when the Numbering service returns the same order number for a repeated idempotency key. | Composite WO create endpoint was not idempotent: retried calls after a partial failure raised a duplicate-key error instead of returning the already-created WO. | No |
| 3.4.0 | 2026-04-10 | bd-8v63o | Add `?include=workcenter_details` optional query param to `GET /api/production/routings/{id}/steps` and `GET /api/production/routings/{id}/steps/{step_id}`. When present, each step gains an embedded `workcenter` object with `workcenter_id`, `name`, and `code`. Without the param the response is identical to v3.3.x. New `WorkcenterDetails` and `RoutingStepEnriched` types. Two new repo methods (`list_steps_enriched`, `find_step_enriched`). 2 new integration tests. | Consumers (Fireproof shop-floor UI) were making a separate GET /workcenters/{id} per routing step to resolve names — classic N+1. Single optional JOIN eliminates all secondary calls. | No |
| 3.3.0 | 2026-04-10 | bd-dhl7p | Add `POST /api/production/work-orders/create` composite endpoint that allocates a WO number from the Numbering service and creates the work order in a single call. New `NumberingClient` wrapper (Platform + Direct modes). `bom_revision_id` and `routing_template_id` are optional in the composite request. Migration makes `work_orders.bom_revision_id` nullable. `WorkOrderCreatedPayload.bom_revision_id` changed to `Option<Uuid>`. 5 new integration tests (domain + HTTP-level). | Verticals orchestrated 5-7 steps to create a WO — allocate number, create WO, attach BOM, attach routing. Composite endpoint collapses these into one call. | YES: `WorkOrderCreatedPayload.bom_revision_id` is now `Option<Uuid>`; consumers reading the event payload must handle null. `WorkOrder.bom_revision_id` is now `Option<Uuid>` in the API response. |
| 3.2.2 | 2026-04-10 | bd-k5bla | Add `GET /api/production/work-orders?ids=a,b,c&include=operations,time_entries` batch fetch endpoint. Returns up to 50 WOs in a single IN-clause query with optional nested operations and time entries (each a single additional IN query). Validates empty ids → 400, >50 ids → 400. 6 new integration tests including HTTP-level validation and 200ms performance gate. | Dashboard was making 20+ individual calls per page load; eliminates N+1 pattern by fetching all WOs and their sub-collections in at most 3 queries. | No |
| 3.2.1 | 2026-04-10 | bd-e5yna | Generate contracts/production/openapi.json from openapi_dump binary. All production endpoints documented with typed schemas (workcenters, work orders, operations, time entries, routings, downtime, component issue, FG receipt), no empty schemas. Add contract-tests validation. | OpenAPI contracts batch 1 — blocks TypeScript SDK codegen and API discovery. | No |
| 3.2.0 | 2026-04-10 | bd-i6if4 | Add `derived_status` (not_started / in_progress / complete) to work order GET and list responses. Computed at query time via LEFT JOIN + COUNT/CASE over operations table — never stored. Add `GET /api/production/work-orders` paginated list endpoint. New `DerivedStatus` enum and `WorkOrderResponse` type. 4 new integration tests covering all three status values. | Verticals had to re-read all operations and recompute aggregate WO status themselves after every operation start/complete — this generic logic is now centralized. | No |
| 3.1.0 | 2026-04-03 | bd-iqv1n | Add `updated_at` field to `RoutingStep` response. New migration adds `updated_at TIMESTAMPTZ DEFAULT now()` column to `routing_steps` table. | Fireproof uses `created_at` as fallback because `updated_at` was missing from routing step responses. | No |
| 3.0.0 | 2026-04-02 | bd-66mqv | Convert 5 sub-collection list endpoints to PaginatedResponse: list_operations, list_time_entries, find_routings_by_item, list_routing_steps, list_workcenter_downtime. All now return `{data: [...], pagination: {page, page_size, total_items, total_pages}}` instead of `{data: [...]}`. utoipa response schemas updated to match. | Response standardization — all list endpoints use the same PaginatedResponse envelope for consistent client parsing. | YES — Sub-collection list endpoints now return `{data: [...], pagination: {...}}` instead of `{data: [...]}`. Consumers must handle the additional `pagination` field. |
| 2.3.2 | 2026-03-31 | bd-fqvjh | Register production metrics with global prometheus registry instead of private Registry::new(). Remove unused metrics_handler and registry() method. | SDK's /metrics endpoint uses prometheus::gather() (global registry) — private registry meant /metrics returned empty body, failing API conformance. | No |
| 2.3.1 | 2026-03-31 | bd-7v7o4 | Replace compile-time `CARGO_MANIFEST_DIR` path with runtime `"module.toml"` in `from_manifest` call. SDK now resolves path via `MODULE_MANIFEST_PATH` env var or CWD. | Compile-time absolute host path baked into the binary does not exist inside Docker containers, causing startup crash. | No |
| 2.3.0 | 2026-03-31 | bd-ehl0p | SDK conversion with outbox publisher. Rewrite main.rs to use ModuleBuilder, extract routes into http::router(), add module.toml with bus=nats and outbox_table=production_outbox, add published_at column migration for SDK publisher compatibility. SDK auto-spawns outbox publisher that polls production_outbox and publishes events to NATS. Manual health routes replaced by SDK-provided /healthz, /api/health, /api/ready, /api/version. | P1 bug: production_outbox events were never published to NATS — inventory, QI, and maintenance consumers received nothing. | No |
| 2.2.3 | 2026-03-31 | bd-e9kw6.1 | Add missing RoutingError::ConflictingIdempotencyKey match arm in error_conversions.rs. | Compilation failure: variant added in idempotency audit but not handled in From impl. | No |
| 2.2.2 | 2026-03-31 | bd-tbnqm.2.1 | Add missing ConflictingIdempotencyKey match arm to 5 From<DomainError> for ApiError conversions (WorkcenterError, TimeEntryError, DowntimeError, ComponentIssueError, FgReceiptError). | Compilation failure: new error variant added without updating error_conversions.rs. | No |
| 2.2.1 | 2026-03-31 | bd-xs0ry.1 | Add optional idempotency_key field to 7 POST endpoints (component-issues, fg-receipt, workcenters, time-entries/start, time-entries/manual, downtime/start, routings). New production_idempotency_keys table. Duplicate key with matching hash returns cached result; different hash returns 409 Conflict. ON CONFLICT (event_id) DO NOTHING on outbox INSERT for defensive safety. | Double-submit creates duplicate outbox events causing downstream inventory consumers to double-process stock issues and FG receipts. | No |
| 2.1.2 | 2026-03-31 | bd-vnuvp.9 | Add tenant_id filter to routing_steps query (via routing_templates subquery) and operations COUNT query. Defense-in-depth tenant isolation on 2 queries. | P0 tenant isolation sweep: queries must filter by tenant_id to prevent cross-tenant data leakage. | No |
| 2.1.1 | 2026-03-31 | bd-z5rek.3 | Migrate config.rs to ConfigValidator for multi-error startup validation. All config errors reported at once in table format. | Plug-and-play wave 2: consistent startup validation across all modules. | No |
| 1.0.0 | 2026-03-28 | bd-gbbus | Initial proof. Workcenter CRUD and deactivation, work order lifecycle, routing creation/revision/release, operation initialization/start/complete with predecessor enforcement, timer and manual time entries, downtime tracking, component issue and finished-goods receipt flows, tenant-scoped queries, and outbox event publishing. 56 integration tests pass, clippy clean. | Production execution module complete and proven for shop-floor workflows. All promotion gates pass. | No |
| 1.0.1 | 2026-03-28 | bd-29c9i.1 | Add RequirePermissionsLayer to all /api/production/* routes: mutate routes (POST/PUT) require production.mutate, read routes (GET) require production.read. Operational endpoints (/healthz, /api/health, /api/ready, /api/version, /metrics) remain ungated. | Production was the only module without permission gating — security audit finding. | No |
| 1.0.2 | 2026-03-30 | bd-lgsgm.1 | Remove old routings.rs file that conflicted with refactored routings/ directory module. The routings/ directory contains the same code split into types.rs and repo.rs with ToSchema derives for OpenAPI. | Rust E0761 dual-module conflict prevented compilation after plug-and-play refactor left both file and directory. | No |
| 2.1.0 | 2026-03-31 | bd-3noyh | Standard response envelopes and OpenAPI spec. Top-level lists (workcenters, routings, active downtime) return PaginatedResponse with page/page_size/total_items/total_pages. Sub-collections (operations, time entries, routing steps, workcenter downtime) return { data: [...] }. All errors use ApiError with error, message, request_id, details. Error conversions in domain/error_conversions.rs. utoipa::path on all 28 handlers. /api/openapi.json endpoint. openapi_dump binary. Bearer JWT SecurityScheme. | Plug-and-play: consistent pagination, error formats, and machine-readable OpenAPI spec for consumers. | YES: List endpoints return { data: [...], pagination: {...} } instead of bare arrays. Error responses include request_id field. Consumers must update response parsing. |

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