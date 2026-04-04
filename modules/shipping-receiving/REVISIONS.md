# shipping-receiving-rs — Revision History

> **What this file is:** The complete record of every change to this module after it was proven. Agents modifying this module must add a row here before committing. Products adopting a new version read this file to understand what changed.
> **Standard:** See `docs/VERSIONING.md` for the rules governing this file.

## Revisions

| Version | Date | Bead | What Changed | Why | Breaking? |
|---------|------|------|-------------|-----|-----------|
| 3.1.2 | 2026-04-04 | bd-0clpi | SoC: extract shipments handler SQL into db/repository.rs | Separation of concerns — shipments handler mixed HTTP logic with raw SQL queries | No |
| 3.1.1 | 2026-04-04 | bd-85tso | Replace tenant_id.parse().expect() with ApiError::bad_request on 16 request paths | Unwrap on user-supplied input causes panic (500) instead of returning 400 Bad Request. | No |
| 3.1.0 | 2026-04-02 | bd-39pj0 | Adopt [platform.services] — declare peer deps in module.toml, use ctx.platform_client | VerticalBuilder adoption | No |
| 3.0.0 | 2026-04-02 | bd-4g5my | Add `#[utoipa::path]` to 4 health handlers (`healthz`, `health`, `ready`, `version`). Change `list_routings` to return `PaginatedResponse<InspectionRoutingRow>` instead of bare `Vec`. All 20 paths now registered in OpenAPI spec. | Response standardization: health endpoints lacked utoipa annotations; list_routings must use PaginatedResponse for consistency with platform standard. | YES — `GET /api/shipping-receiving/shipments/{id}/routings` response shape changed from bare `[...]` array to `{"data":[...],"pagination":{...}}`. Consumers parsing the routings list must update to the PaginatedResponse envelope. |
| 2.2.8 | 2026-04-01 | Import extract_tenant from platform-sdk instead of local copy (bd-o1a03) |
| 2.2.7 | 2026-04-01 | bd-thx8s | Fix PO approved consumer subject to ap.events.ap.po_approved (was ap.po.approved). Remove dead sales.so.released consumer — no sales module exists. | Consumer never received PO approved events; dead consumer wasted a NATS subscription. | No |
| 2.2.6 | 2026-04-01 | bd-2gyqj | Update InventoryIntegration to pass &VerifiedClaims via PlatformClient::service_claims(tenant_id). Remove reqwest::Client from Mode::Http. Constructor uses PlatformClient::new().with_bearer_token(). | New typed client API requires per-request &VerifiedClaims for tenant-scoped auth. | No |
| 2.2.5 | 2026-04-01 | bd-tbumc | Replace hand-rolled reqwest HTTP calls in inventory_client.rs with typed platform-client-inventory ReceiptsClient and IssuesClient. Remove local HTTP request/response types. | Typed client conversion — consistent with platform client generation pattern. | No |
| 2.2.4 | 2026-03-31 | bd-5vmu6 | Convert to platform-sdk ModuleBuilder with SDK consumer adapters for po_approved and so_released. | SDK batch conversion — eliminate two classes of modules. | No |
| 2.2.3 | 2026-03-31 | bd-decba | Add RequirePermissionsLayer with MODULE_READ permission to all read routes. Previously, read endpoints were accessible without JWT authentication. | P0 security: aerospace/defense requires all data endpoints gated by JWT. Read routes were unprotected since initial plug-and-play rollout. | No (consumers who already provide valid JWT + read permissions are unaffected) |
| 2.2.1 | 2026-03-30 | bd-nhmgu | Add openapi_dump utility binary for offline spec generation. | OpenAPI spec validation: offline dump needed for spec extraction and validation. | No |
| 2.2.2 | 2026-03-30 | bd-lgsgm.3 | Wire error_conversions module (From<ShipmentError> for ApiError, From<RoutingError> for ApiError). Migrate metrics_handler from ErrorBody to ApiError. Add ShipmentRepository::count_shipments for paginated list_shipments. | Plug-and-play left error_conversions.rs unlinked in mod.rs, metrics.rs referencing removed ErrorBody, and list_shipments calling nonexistent count method — all caused 29 compilation errors. | No |
| 1.0.0 | 2026-03-28 | bd-eexq4 | Initial proof. All tests passing. | Module build complete and core logic validated via tests. | No |
| 2.0.0 | 2026-03-30 | bd-y3hq2 | Replace ErrorBody with ApiError. Add PaginatedResponse to list_shipments. All error responses include request_id via TracingContext. Query params changed from limit/offset to page/page_size. | Plug-and-play Wave 2: standard response envelopes. | YES — list_shipments returns `{"data":[],"pagination":{}}` instead of bare array. Error responses now include `request_id` field. Query params changed from `limit`/`offset` to `page`/`page_size`. |
| 2.1.0 | 2026-03-30 | bd-y3hq2 | Add OpenAPI spec via utoipa. All handlers annotated with `#[utoipa::path]`. All types derive ToSchema/IntoParams. `/api/openapi.json` route serves OpenAPI 3.0 spec. SecurityAddon for Bearer JWT. | Plug-and-play Wave 2: OpenAPI documentation. | No |
| 2.2.0 | 2026-03-30 | bd-y3hq2 | Migrate config.rs to ConfigValidator. NATS_URL uses require_when for conditional validation. | Plug-and-play Wave 2: startup improvements. | No |

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
