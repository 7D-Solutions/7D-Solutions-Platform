# ap — Revision History

> **What this file is:** The complete record of every change to this module after it was proven. Agents modifying this module must add a row here before committing. Products adopting a new version read this file to understand what changed.
> **Standard:** See `docs/VERSIONING.md` for the rules governing this file.

## Required fields

Every row in the Revisions table must have these fields filled in (no placeholders):

| Field | Column | Requirement |
|-------|--------|-------------|
| 2.1.6 | 2026-04-01 | Import extract_tenant from platform-sdk instead of local copy (bd-o1a03) |
| Version | Version | Exact SemVer matching the package file |
| Date | Date | ISO date YYYY-MM-DD, not the literal placeholder |
| Bead | Bead | Active bead ID (not bd-xxxx) |
| Summary | What Changed | Concrete — name endpoints, fields, events, behaviors. Not "TODO" or "various improvements." |
| Why | Why | The problem solved or requirement fulfilled. Not "TODO." |
| Proof | (Gate 1) | `scripts/proof_ap.sh` must exist before 1.0.0 is committed. |
| Compatibility | Breaking? | "No" if consumers are unaffected. "YES: <migration note>" if breaking. Never empty. |

## Revisions

| Version | Date | Bead | What Changed | Why | Breaking? |
|---------|------|------|-------------|-----|-----------|
| 3.3.0 | 2026-04-09 | bd-q4kb8 | AP attachment consumer: bill_attachments migration (bill_id + attachment_id unique with IF NOT EXISTS on index), attachment_linked NATS consumer on docmgmt.attachment.created, links doc-mgmt attachments to vendor bills with idempotent upsert | AP needs to react to document uploads and link attachments to vendor bills for 3-way match workflows | No |
| 3.2.2 | 2026-04-04 | bd-q5efi | SoC: payment_runs, po/approve, bills/approve SQL moved to repos. Fix test unwraps in tax/mod.rs. | Separation of concerns continued — all AP domain services delegate to repos | No |
| 3.2.1 | 2026-04-04 | bd-q5efi | SoC: additional repo extraction from tax/mod.rs | Separation of concerns — SQL queries in tax module handler need to move to repo layer. | No |
| 3.2.0 | 2026-04-04 | bd-tzbk9,bd-q5efi | SoC: extract SQL from bills/service.rs (17 queries) and tax/service.rs (13 queries) into repo modules | Separation of concerns — GL exemplar pattern | No |
| 3.1.0 | 2026-04-02 | bd-i28hj | Add IntoParams to ListBillsQuery — codegen now picks up vendor_id and include_voided query params | Generated client was missing query params | No |
| 3.0.2 | 2026-04-02 | bd-azq84 | Removed local extract_tenant (now in SDK) | Plug-and-play standardization | No |
| 3.0.1 | 2026-04-02 | bd-5d6ae | Remove dead run_publisher_task and publish_batch from outbox/mod.rs. SDK publisher (ModuleBuilder) handles event publishing; this custom loop was never called and contained a double-prefix bug (ap.events.ap.*). | Dead code hygiene — eliminates confusing unused publisher that would publish to wrong NATS subjects if ever wired up. | No |
| 3.0.0 | 2026-04-02 | bd-u2io9 | Refactor match engine into repo + service + service_tests layers. `engine.rs` (718 LOC) replaced by `repo.rs` (DB queries/writes), `service.rs` (orchestration + pure matching logic), `service_tests.rs` (integration tests). All files under 500 LOC. | Single-file monolith mixed guard queries, matching computation, persistence, and outbox emission. SoC refactor separates DB access from business logic. | YES: Internal import path changed from `domain::match::engine::run_match` to `domain::match::service::run_match`. No HTTP API or event contract changes. |
| 2.1.7 | 2026-04-02 | bd-9v3vx | Add ToSchema to AgingReport, CurrencyBucket, VendorBucket, BillBalanceSummary, PaymentRun. Replace serde_json::Value body types with typed schemas on 3 GET endpoints (aging, balance, get_run). | serde_json::Value generates empty schema {} which codegen treats as Empty. | No |
| 2.1.6 | 2026-04-02 | bd-9v3vx | Add body= to utoipa response annotations on 23 AP endpoints (vendors, bills, allocations, payment_runs, payment_terms, purchase_orders, reports, tax_reports). | OpenAPI specs were missing response schemas, causing codegen to emit Result<(), ClientError> instead of typed responses. | No |
| 2.1.5 | 2026-04-01 | bd-14qer | Migrate list_allocations to PaginatedResponse. Add ToSchema to AllocationRecord. Register PaginatedResponse<AllocationRecord> in OpenAPI schema. | All 5 AP list endpoints now return consistent PaginatedResponse shape. | No |
| 2.1.4 | 2026-03-31 | bd-5vmu6 | Convert to platform-sdk ModuleBuilder. Replaces manual dotenv/tracing/pool/bus/outbox/middleware/health/shutdown boilerplate with SDK startup sequence. Bus and outbox publisher now configured via module.toml. | SDK batch conversion — eliminate two classes of modules. | No |
| 2.1.3 | 2026-03-31 | bd-vnuvp.1 | Add tenant_id filter to 23 SQL queries across 10 AP source files. Production: purchase_orders lookup in inventory consumer, count_receipt_links_for_line now joins through purchase_orders, fetch_snapshot accepts optional tenant_id. Tests: all assertion queries on vendor_bills/ap_allocations scoped by tenant_id; queries on po_receipt_links/three_way_match/po_status use subqueries through parent tables. | P0 security: 23 queries on tenant data tables lacked tenant_id in WHERE clause, allowing potential cross-tenant data leakage. | No |
| 2.1.2 | 2026-03-31 | bd-ig2rz.2 | Add #[utoipa::path] annotations to all 28 HTTP handler functions (vendors, purchase_orders, bills, allocations, payment_terms, payment_runs). Add ToSchema to 18 domain request/response types. Fix ApTaxSummaryRow missing ToSchema. Remove BillTaxQuoteRequest from OpenAPI schemas (TaxAddress in tax-core lacks utoipa). | AP was broken in HEAD: OpenAPI ApiDoc referenced 28 handlers without path annotations. Workspace build blocked. | No |
| 2.1.1 | 2026-03-30 | bd-nhmgu | Add openapi_dump utility binary for offline spec generation. | OpenAPI spec validation: offline dump needed for spec extraction and validation. | No |
| 1.0.0 | 2026-03-28 | bd-3bwil | Initial proof. Vendor CRUD, bill lifecycle (open/matched/approved/partially_paid/paid/voided), PO management (draft/approved/cancelled/closed), 2-way and 3-way match engine, append-only payment allocations with row-locking, payment runs (build/execute with idempotent disbursement), payment terms (Net-N, discount schedules), receipt-link ingestion, AP aging reports (current/30/60/90+ buckets), tax quoting/commit/void via tax-core, outbox atomicity, tenant isolation, event envelope publishing, DLQ replay drill binary. 229 unit tests, 58 integration tests (real Postgres). | AP module complete and proven against real database. All gates pass. | No |
| 1.0.1 | 2026-03-30 | bd-ctx1n | Split `po/service.rs` into `po/queries.rs` (reads) + `po/service_tests.rs` (tests). | Enforce 500 LOC file-size limit. Internal refactor only. | No |
| 1.0.2 | 2026-03-30 | bd-ctx1n | Fixed `get_po` import paths in `po/approve.rs` tests and `po_integration.rs` after queries extraction. Added 10 cohesive AP files to `.file-size-allowlist` per directive to split by concerns, not line count. | Complete the split bead — remaining files are single-concern and don't benefit from splitting. | No |
| 2.1.0 | 2026-03-30 | bd-ew9em | Added `#[utoipa::path]` annotations to all 33 public handler functions. Created `ApiDoc` OpenAPI struct in `http/mod.rs`. Added `/api/openapi.json` and `/api/schema-version` endpoints in `main.rs`. All request/response types registered as schemas. | Plug-and-play: serve a valid OpenAPI spec so API consumers and tooling can auto-discover AP endpoints. | No |
| 2.0.0 | 2026-03-30 | bd-rvdcu | All HTTP handlers use `ApiError` from `platform-http-contracts` (was: custom `ErrorBody`). List endpoints (`list_vendors`, `list_bills`, `list_pos`, `list_terms`) return `PaginatedResponse<T>` (was: bare `Vec<T>`). All error paths enriched with `request_id` from `TracingContext`. Added `From<*Error> for ApiError` for all 8 domain error types. Added `ToSchema` on `Vendor`, `VendorBill`, `PurchaseOrder`, `PaymentTerms`. | Standardize response envelopes — AP must match the contract pattern used by Inventory, Party, and other proven modules. Consumers read `.data` for items, `.pagination` for metadata. Errors use `ApiError` shape with `request_id`. | YES: List endpoints return `{data, pagination}` envelope instead of bare arrays. Error responses use `ApiError` shape (`{error, message, request_id?}`) instead of `ErrorBody` (`{error, message}`). |

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
