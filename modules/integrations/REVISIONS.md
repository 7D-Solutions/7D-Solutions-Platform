# integrations — Revision History

> **What this file is:** The complete record of every change to this module after it was proven. Agents modifying this module must add a row here before committing. Products adopting a new version read this file to understand what changed.
> **Standard:** See `docs/VERSIONING.md` for the rules governing this file.


## 2.20.0
- feat: route CDC and full-resync flows into observations with high-watermark tracking, tombstone handling for deleted entities, and `source_channel` column; migration adds columns to observations table ([bd-5uf61] / Stream D Phase 1.5)

## 2.19.0
- feat: add `resolve_payment` handler module dispatching accept/reject resolution strategies for payment conflicts through `resolve_service`; integration tests against real Postgres ([bd-whva2] / Stream D Phase 1.5)

## 2.18.0
- feat: add `resolve_customer` handler module dispatching accept/reject/remap resolution strategies through `resolve_service`, with per-entity handler trait and customer-specific duplicate-remap validation; integration tests against real Postgres ([bd-6tb53] / Stream D Phase 1.5)

## 2.17.1
- fix: add field-level intent guard (`update_entity_with_guard`) that detects third-party edits during stale SyncToken retries by comparing touched business fields against a pre-read baseline; raises `ConflictDetected` when another party changed a field the caller intends to write, preventing silent overwrites ([bd-dd2x6] / Stream D Phase 1.5)

## 2.17.0
- feat: add `integrations_sync_observations` schema with comparable projection columns (`projected_hash`, `observed_at_millis`), `dedupe` module for fingerprint computation and millisecond timestamp normalization, and `observations` repo with insert/upsert/list primitives; migration creates table with `(app_id, provider, entity_type, entity_id)` unique index ([bd-fc7xl] / Stream D Phase 1.5)

## 2.16.0
- feat: extend QBO client with `create_customer`, `update_customer`, `create_payment`, `update_payment` operations, rate-limit response header parsing (`X-RateLimit-Remaining`, `Retry-After`), and deterministic `RequestId` propagation from platform request fingerprints; updates existing tests and adds idempotency coverage ([bd-hv1e2] / Stream D Phase 1.5)

## 2.15.0
- feat: add `integrations_sync_jobs` operational health table with `upsert_job_success` / `upsert_job_failure` repo primitives, `GET /api/integrations/sync/jobs` paginated endpoint, and health instrumentation in the OAuth token refresh loop; migration creates the jobs table with `(app_id, provider, job_name)` unique constraint and failure streak tracking ([bd-cjdst] / Stream D Phase 1.5)

## 2.14.0
- feat: add pre-call authority version check (`pre_call_version_check`) that short-circuits to `superseded` when authority advances before dispatch, and post-call stale-authority reconciliation (`post_call_reconcile`) that atomically transitions inflight attempts to `completed_under_stale_authority` then auto-closes equivalent values or opens a conflict row for divergent ones; migration adds both terminal statuses to the push_attempts status check constraint ([bd-w6e21] / Stream D Phase 1.5)

## 2.13.0
- feat: implement authority flip service with Postgres advisory lock on `(app_id, provider, entity_type)`, atomic `authority_version` bump, `integrations.sync.authority.changed` event emission via outbox, and real `POST /api/integrations/sync/authority` handler ([bd-y7np7] / Stream D Phase 1.5)
- feat: add `GET /api/integrations/sync/dlq` (failed outbox rows filtered by `failure_reason` + app/time bounds + pagination) and `GET /api/integrations/sync/push-attempts` (ledger reader with provider/entity/status/request_id/time filters) + supporting `outbox::list_failed` primitive ([bd-xvdvh] / Stream D Phase 1.5)

## 2.12.1
- feat: add push-attempt watchdog worker (`run_watchdog_task`) spawned from `main`, running every 60s and transitioning `inflight` rows older than 10 minutes to `failed` with `error_code='inflight_timeout'` so the partial unique index cannot be permanently blocked by a stuck row ([bd-nmvd6] / Stream D Phase 1.5)

## 2.12.0
- feat: add `integrations_sync_push_attempts` ledger migration (partial unique index on `(app_id, provider, entity_type, entity_id, operation, authority_version, request_fingerprint) WHERE status IN ('accepted','inflight','succeeded')`, result-marker columns for detector correlation, scan-friendly indexes) and `domain/sync/push_attempts` repo primitives with typed `PushAttemptRow` / `PushStatus` ([bd-bh65z] / Stream D Phase 1.5)

## 2.11.0
- feat: gate legacy QBO outbound consumers behind `QBO_LEGACY_CONSUMERS_ENABLED=1` feature flag (default OFF) so `spawn_outbound_consumer` and `spawn_order_ingested_consumer` cannot race with the new authority-gated sync path during cutover ([bd-c3ghe] / Stream D Phase 1)

## 2.10.0
- feat: add `integrations_sync_conflicts` migration (per-tenant drift rows with `external_value`/`internal_value` jsonb, 256 KB cap, class/status enums) and `domain/sync/{conflicts,conflicts_repo}` persistence layer with `ConflictRow`, `ConflictClass`, `ConflictStatus`, `ConflictError` types and re-exports ([bd-bnvqs] / Stream D Phase 1)

## 2.9.0
- feat: add `integrations_sync_authority` migration (unique `(app_id, provider, entity_type)`, monotonic `authority_version BIGINT`, flip audit columns) and `domain/sync/{authority,authority_repo}` module primitives with typed repository operations ([bd-txdkm] / Stream D Phase 1)

## 2.8.1
- feat: normalize QBO env contract (`QBO_BASE_URL` canonical; empty values treated as missing), add `validate_qbo_env()` fail-fast startup check when `QBO_CLIENT_ID` is set, wire 5 QBO vars through `docker-compose.services.yml`, and retire `QBO_API_BASE` in favor of `qbo_base_url()` ([bd-t0ach] / Stream D Phase 1)

## 2.8.0
- feat: add `failure_reason` column to `integrations_outbox` (enum: `bus_publish_failed` | `retry_exhausted` | `needs_reauth` | `authority_superseded`) with partial index; relay now records the reason on publish failure so `/sync/dlq` filters deterministically ([bd-y5zol] / Stream D Phase 1)

## 2.7.7
- feat: scaffold /api/integrations/sync/* route tree with 501 stubs and wire dedicated sync sub-capabilities from platform/security ([bd-n68o6] / Stream D Phase 1)

## 2.7.6
- fix: OAuth reconnect upsert on (app_id, provider), partial unique on (provider, realm_id) scoped to connected rows, and mandatory `state` CSRF guard on callback ([bd-apj1n] / Stream D Phase 0)

## 2.7.5
- chore: rustfmt reflow + regenerate typed clients (no behavior change)

## Required fields

Every row in the Revisions table must have these fields filled in (no placeholders):

| Field | Column | Requirement |
|-------|--------|-------------|
| 2.7.4 | 2026-04-14 | bd-sw5ds.1 | Stabilize qbo_outbound sandbox tests: clear any stale quickbooks OAuth row for the sandbox realm before seeding, use shorter synthetic invoice/order IDs, and include a valid QBO ItemRef in the direct create_invoice sandbox check. | The QBO sandbox rejected long doc numbers, bare line items, and duplicate realm rows, causing verification failures after the bead was closed. | No |
| 2.7.3 | 2026-04-14 | bd-sw5ds | Replace local mock/stub integration harnesses with real QBO and eBay sandbox-backed helpers in qbo_outbound.rs, cross_module_flow.rs, and ebay_connector.rs. Keep the SyncToken retry test on a local axum server with an explanatory comment because the sandbox cannot deterministically reproduce the 5xx sequence. | Integration smoke tests should exercise the real provider sandboxes instead of local fakes so connector behavior matches production wiring. | No |
| 2.3.2 | 2026-04-01 | Import extract_tenant from platform-sdk instead of local copy (bd-o1a03) |
| Version | Version | Exact SemVer matching the package file |
| Date | Date | ISO date YYYY-MM-DD, not the literal placeholder |
| Bead | Bead | Active bead ID (not bd-xxxx) |
| Summary | What Changed | Concrete — name endpoints, fields, events, behaviors. Not "TODO" or "various improvements." |
| Why | Why | The problem solved or requirement fulfilled. Not "TODO." |
| Proof | (Gate 1) | `scripts/proof_integrations.sh` must exist before 1.0.0 is committed. |
| Compatibility | Breaking? | "No" if consumers are unaffected. "YES: <migration note>" if breaking. Never empty. |

## Revisions

| Version | Date | Bead | What Changed | Why | Breaking? |
|---------|------|------|-------------|-----|-----------|
| 2.7.2 | 2026-04-14 | bd-5ea4y.1 | Add structured fields to bare tracing::error! calls in HTTP handler files (connectors.rs, external_refs.rs, oauth.rs, qbo_invoice.rs). Config-error calls get `error_code = "OPERATION_FAILED"`; DB errors get `error = %e`. | Structured logging standard (bd-5ea4y) requires at least one field before the message string in all HTTP handler log calls. CI check-log-fields.sh now passes. | No |
| 2.7.1 | 2026-04-10 | bd-e5yna | Generate contracts/integrations/openapi.json from openapi_dump binary. All 17 integrations endpoints documented with typed schemas (external refs, connectors, OAuth, webhooks, QBO invoice), no empty schemas. Add contract-tests validation. | OpenAPI contracts batch 1 — blocks TypeScript SDK codegen and API discovery. | No |
| 2.7.0 | 2026-04-10 | bd-4ec8i | eBay fulfillment write-back module + QBO outbound marketplace order sync (OrderIngestedPayload consumer creates QBO invoices from marketplace orders with customer upsert and external ref tracking). New ebay_fulfillment.rs, expanded qbo_outbound.rs and tests. | Marketplace orders need to flow into QBO as invoices for accounting; eBay fulfillment write-back prepares shipment confirmation | No |
| 2.6.0 | 2026-04-09 | bd-4ec8i | eBay marketplace connector (connector_type="ebay"): eBayConnector implementing Connector trait, registered in get_connector/all_connectors, validate_config checks client_id/client_secret/ru_name/environment. ebay_poller.rs: OAuth2 client-credentials token exchange, GetOrders cursor-based polling, order normalization to OrderIngestedPayload (source="ebay"), file_job per order with idempotency_key="ebay-fj-{order_id}". NATS consumer on integrations.poll.ebay. 23 tests. | Third marketplace connector — eBay requires polling (no webhooks for sales). OrderIngestedPayload schema identical to Shopify/Amazon. | No |
| 2.5.1 | 2026-04-09 | bd-1z8bl,bd-ttdso,bd-2xl19 | Seed migration for carrier sandbox credentials (USPS, FedEx, UPS connector configs for CI integration tests) | Carrier integration tests need pre-seeded connector configs to hit sandbox APIs | No |
| 2.5.0 | 2026-04-09 | bd-g6hyd | Amazon SP-API marketplace connector (connector_type="amazon_sp"): AmazonConnector implementing Connector trait, registered in get_connector/all_connectors, validate_config checks seller_id/refresh_token/client_id/client_secret/marketplace_id. amazon_poller.rs: LWA OAuth token exchange, SP-API GetOrders polling with exponential backoff on 429, order normalization to platform OrderIngestedPayload (source="amazon_sp"), file_job per order with idempotency_key="amazon-fj-{order_id}", last_poll_timestamp stored atomically in connector config. NATS consumer on integrations.poll.amazon_sp. 16 tests (8 connector unit + 8 poller). | Second marketplace connector proves push+pull patterns coexist. Amazon SP-API requires polling (no webhooks) — OrderIngestedPayload schema is identical to Shopify so downstream consumers are source-agnostic. | No |
| 2.4.2 | 2026-04-09 | bd-x73ze,bd-4w97z,bd-afha3 | Edge-case tests: qbo_outbound cleanup_tenant() prevents OAuth cross-pollution, disabled connector 404 test for carrier credentials endpoint, 3 Shopify normalizer edge-case tests (orders/updated topic, invalid HMAC, empty line_items), broader OAuth refresh test cleanup (DELETE all connections vs only expired) | Test hardening — prevent cross-suite pollution and verify edge-case behavior | No |
| 2.4.1 | 2026-04-09 | bd-0urmb | Add 3 Shopify webhook routing unit tests (orders/create, orders/updated, unknown topic) | Verify webhook routing covers Shopify topics after connector was added | No |
| 2.4.0 | 2026-04-08 | bd-w9mu5,bd-yvc71,bd-0urmb | Internal carrier credentials endpoint, QBO outbound invoice creation from AR events, Shopify marketplace connector with webhook handler and order ingestion | Carrier providers need credential lookup; AR invoices must sync to QBO; Shopify orders must flow into the platform via normalized webhooks | No |
| 2.3.6 | 2026-04-04 | bd-p54p6 | SoC: extract domain service SQL into repo modules (connectors, edi_transactions, external_refs, file_jobs, oauth, outbound_webhooks, qbo, webhooks) | Separation of concerns — isolate persistence from business logic so domain services are testable and DB access is centralized. | No |
| 2.3.5 | 2026-04-04 | bd-7tv0x | Remove unused Router and Config imports from main.rs | Dead import cleanup | No |
| 2.3.4 | 2026-04-02 | bd-vcly8 | Delete dead health.rs stub (unreferenced after SDK conversion) | Dead code cleanup | No |
| 2.3.3 | 2026-04-02 | bd-azq84 | Removed local extract_tenant, cleaned oauth/qbo_invoice imports | Plug-and-play standardization | No |
| 2.3.2 | 2026-04-02 | bd-9v3vx | Add body= to utoipa response annotation on inbound_webhook endpoint. | OpenAPI spec was missing response schema, causing codegen to emit Result<(), ClientError>. | No |
| 2.3.1 | 2026-03-31 | bd-5vmu6 | Convert to platform-sdk ModuleBuilder. Background workers spawn in routes closure. Bus accessed via ctx.bus_arc(). | SDK batch conversion — eliminate two classes of modules. | No |
| 1.0.0 | 2026-03-28 | bd-1rdqj | Initial proof. External refs CRUD with outbox events. Webhook ingest (Stripe, GitHub, QuickBooks, internal). QBO CDC/webhook normalization with realm→tenant resolution. OAuth connection management with encrypted tokens. Outbox relay with retry/DLQ. EDI transactions. File jobs. Outbound webhooks with delivery logging. 145 integrated tests against real Postgres. | Platform integrations layer ready for production. Handles external system connections, webhook routing, and event publishing. | No |
| 1.0.1 | 2026-03-29 | bd-ym43b | Add `POST /api/integrations/qbo/invoice/{invoice_id}/update` endpoint for sparse-updating QBO invoice shipping fields (ShipDate, TrackingNum, ShipMethodRef). Uses platform OAuth connection, handles SyncToken concurrency via QboClient retry loop. Gated by `integrations.mutate` permission. | Huber Power Phase 1 write-back requires outbound QBO invoice updates with shipping data. | No |
| 2.0.0 | 2026-03-30 | bd-hmoua | Migrate all HTTP handlers from ErrorBody to ApiError (platform-http-contracts). Wrap 3 list endpoints (list_by_entity, list_connector_types, list_connectors) in PaginatedResponse envelopes. Add platform-http-contracts and utoipa dependencies. Remove unused imports. | Plug-and-play standardization: uniform error shapes and paginated list responses across all platform modules. | YES: Error responses change shape from `{"error":"..."}` to `{"status":N,"code":"...","message":"..."}`. List endpoints now return `{"items":[...],"page":N,"page_size":N,"total":N}` instead of bare arrays. |
| 2.1.1 | 2026-03-30 | bd-lgsgm.2 | Create openapi_dump binary for OpenAPI spec generation. Add missing imports in connectors.rs (ConnectorCapabilities, ConnectorConfig, TestActionResult) and oauth.rs (OAuthConnectionInfo) needed by utoipa body attributes. | Plug-and-play left openapi_dump.rs unreferenced in Cargo.toml but not created, and utoipa body attributes referenced types not in scope — both caused build failures. | No |
| 2.3.0 | 2026-03-30 | bd-l5sg9 | Add OpenAPI spec via utoipa (`/api/openapi.json`) with all 17 endpoints documented. Add `#[utoipa::path]` and `ToSchema` annotations to all handlers and domain types. Convert `Config::from_env()` to use `ConfigValidator` for structured multi-error reporting at startup. Add `config-validator` dependency. QBO OAuth refresh worker (30s), CDC polling worker (15m), and HMAC-SHA256 webhook verification preserved unchanged. | Plug-and-play: integrations module needs discoverable API spec and consistent startup validation matching other platform modules. | No |
| 2.2.1 | 2026-03-30 | bd-lgsgm | Add ExternalRef import to external_refs.rs for utoipa body attributes. Add utoipa::path annotations to external_refs, qbo_invoice, and webhooks handlers that were missed in 2.2.0. | E2E test suite compilation required all utoipa body type references to be in scope. | No |


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