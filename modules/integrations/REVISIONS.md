# integrations — Revision History

> **What this file is:** The complete record of every change to this module after it was proven. Agents modifying this module must add a row here before committing. Products adopting a new version read this file to understand what changed.
> **Standard:** See `docs/VERSIONING.md` for the rules governing this file.

## Required fields

Every row in the Revisions table must have these fields filled in (no placeholders):

| Field | Column | Requirement |
|-------|--------|-------------|
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
