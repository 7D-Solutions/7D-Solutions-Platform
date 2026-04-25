# shipping-receiving-rs — Revision History

> **What this file is:** The complete record of every change to this module after it was proven. Agents modifying this module must add a row here before committing. Products adopting a new version read this file to understand what changed.
> **Standard:** See `docs/VERSIONING.md` for the rules governing this file.


## 3.7.2
- test(bd-a2rrm): real-sandbox integration tests for UPS, FedEx, and USPS — skip when credentials absent, assert UPS Ground / FedEx Ground / Priority Mail in rate responses, assert tracking starts with "1Z" / is 12 digits, assert unknown tracking numbers are rejected. USPS tests call the new OAuth REST API (api.usps.com) directly.

## 3.7.1
- fix(bd-1n4am.2): test compilation errors — make `create_label` a pub mod so integration tests can import `record_label_cost_tx` (was E0603); annotate two `let event_id: Uuid` bindings in shipping_cost_event_emission_test.rs (was E0282).

## 3.7.0
- feat(bd-1n4am): wire cost-event emission end-to-end — event contract file moved to src/events/contracts/shipping_cost.rs (submodule); routes.rs registers POST /api/shipping-receiving/shipments/{id}/label; integration test shipping_cost_event_emission_test.rs covers outbox emission path. Cross-module with AP 3.8.0 + AR 6.9.0 consumers.

## 3.6.0
- feat(bd-1n4am): add `shipping_receiving.shipping_cost.incurred` event contract + `POST /api/shipping-receiving/shipments/{id}/label` endpoint. Emits one canonical cost event per label into the outbox; AP and AR consume downstream.

## 3.5.0
- feat: R&L Carriers LTL provider — `RlCarrierProvider` registered as "rl". API-key auth, rate quote, BOL creation, tracking. `NotFound` variant added to `CarrierProviderError` for 404 tracking responses.

## 3.4.4
- chore: rustfmt reflow + regenerate typed clients (no behavior change)

## Revisions

| Version | Date | Bead | What Changed | Why | Breaking? |
|---------|------|------|-------------|-----|-----------|
| 3.5.0 | 2026-04-25 | bd-gaqqv | R&L Carriers LTL `CarrierProvider` impl (`rl.rs`). API-key auth (`X-API-Key`), rate quote (`POST /api/RateQuote`), BOL creation (`POST /api/BillOfLading`), tracking (`GET /api/Shipments/{pro}`). `NotFound` variant added to `CarrierProviderError` for 404 tracking responses. Integration test skips when `RL_SANDBOX_API_KEY` absent. | LTL carrier support for R&L per 2026-04-24 shipping direction. | No |
| 3.4.3 | 2026-04-15 | bd-p3duh | Move envelope.payload by value in on_po_approved consumer — replace serde_json::from_value(envelope.payload.clone()) with from_value(envelope.payload). | Payload was heap-copied on every dispatch even though it is consumed once. Same perf fix applied by bd-g7zzj to notifications and maintenance. | No |
| 3.4.2 | 2026-04-14 | bd-6jhh8 | Add `docs/guides/carrier-adapters.md` with FedEx/UPS/USPS opt-in guidance, surface the guide from the module README, and warn at startup when `StubCarrierProvider` is used outside development. | Verticals were shipping with the stub carrier path by accident and there was no documented handoff for enabling the real carrier adapters. | No |
| 3.4.1 | 2026-04-13 | bd-1my6.2 | Skip inventory call for shipment lines with no warehouse_id. Previously `unwrap_or(Uuid::nil())` caused 404 from inventory service for non-inventory-tracked lines (manufactured goods shipped directly). Now skips with `continue`. | Lines without warehouse are not inventory-tracked and should not trigger inventory issues. | No |
| 3.4.0 | 2026-04-10 | bd-6pyqw | Add `POST /api/shipping-receiving/shipments/{id}/outbound` composite endpoint. Orchestrates: validate outbound+packed, collect source WO IDs, quality gate check (QI service for "held" final inspections), permission-gated override via `quality_inspection.mutate`, then atomic packed→shipped transition. Add `QualityGateIntegration` (Platform/Permissive/AlwaysHold modes). 7 integration tests. | Verticals were re-implementing the ship flow (~200 LOC each). Single platform endpoint eliminates duplication and enforces the QI gate as a security boundary. | No |
| 3.3.2 | 2026-04-10 | bd-e5yna | Generate contracts/shipping-receiving/openapi.json from openapi_dump binary. All 21 endpoints documented with typed schemas (shipments, inspection routing, PO/source refs), no empty schemas. Add contract-tests validation. | OpenAPI contracts batch 1 — blocks TypeScript SDK codegen and API discovery. | No |
| 3.3.1 | 2026-04-09 | bd-wti4f | Fix outbox_table in module.toml: events_outbox -> sr_events_outbox to match migration DDL. Fix INVENTORY_URL -> INVENTORY_BASE_URL in docker env (SDK reads _BASE_URL suffix). Add INTEGRATIONS_SERVICE_URL for carrier credential lookups. | SDK publisher was polling non-existent events_outbox table (constant error logs). Inventory calls unreachable inside Docker. | No |
| 3.3.0 | 2026-04-09 | bd-1z8bl,bd-ttdso,bd-2xl19 | USPS, FedEx, and UPS carrier adapters implementing CarrierProvider trait. USPS: XML-based Rate V4, eVS label, Track V2 with HTML entity-safe strip_html_tags. FedEx: OAuth2 client-credentials with token caching, Rate/Ship/Track REST APIs, Ground+Express response handling. UPS: OAuth2, Rating/Shipping/Track JSON APIs. All three registered in get_provider() dispatch. Integration tests for each carrier (sandbox credentials in CI). quick-xml dependency added. | Three production carrier integrations enabling rate shopping, label creation, and tracking across major US carriers | No |
| 3.2.1 | 2026-04-09 | bd-or9z8 | 4 edge-case tests for CarrierProvider dispatch (unknown carrier → failed, concurrent dispatch lock, missing config, unreachable service) | Verify carrier provider handles failure modes gracefully without panics | No |
| 3.2.0 | 2026-04-08 | bd-w9mu5 | CarrierProvider trait + async dispatch consumer + credential facade | Platform needs a carrier abstraction so verticals don't implement carrier integrations directly. StubCarrierProvider for testing, NATS consumer for sr.carrier_request.created, reqwest facade to integrations internal endpoint for credentials. | No |
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