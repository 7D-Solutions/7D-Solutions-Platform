# subscriptions — Revision History

> **What this file is:** The complete record of every change to this module after it was proven. Agents modifying this module must add a row here before committing. Products adopting a new version read this file to understand what changed.
> **Standard:** See `docs/VERSIONING.md` for the rules governing this file.

## Required fields

Every row in the Revisions table must have these fields filled in (no placeholders):

| Field | Column | Requirement |
|-------|--------|-------------|
| 3.0.1 | 2026-04-01 | Import extract_tenant from platform-sdk instead of local copy (bd-o1a03) |
| Version | Version | Exact SemVer matching the package file |
| Date | Date | ISO date YYYY-MM-DD, not the literal placeholder |
| Bead | Bead | Active bead ID (not bd-xxxx) |
| Summary | What Changed | Concrete — name endpoints, fields, events, behaviors. Not "TODO" or "various improvements." |
| Why | Why | The problem solved or requirement fulfilled. Not "TODO." |
| Proof | (Gate 1) | `scripts/proof_subscriptions.sh` must exist before 1.0.0 is committed. |
| Compatibility | Breaking? | "No" if consumers are unaffected. "YES: <migration note>" if breaking. Never empty. |

## Revisions

| Version | Date | Bead | What Changed | Why | Breaking? |
|---------|------|------|-------------|-----|-----------|
| 5.2.0 | 2026-04-04 | bd-lyhp3 | SoC: move bill_run_repo and bill_run_service from http/ to domain layers. Add utoipa annotations to health endpoints for G4 compliance. | Separation of concerns + HTTP ergonomics | No |
| 5.1.0 | 2026-04-02 | bd-39pj0 | Adopt [platform.services] — declare peer deps in module.toml, use ctx.platform_client | VerticalBuilder adoption | No |
| 5.0.0 | 2026-04-02 | bd-cmgbw | Add ToSchema to 7 domain model structs (SubscriptionPlan, CreateSubscriptionPlanRequest, Subscription, CreateSubscriptionRequest, PauseSubscriptionRequest, CancelSubscriptionRequest, BillRun). Register all schemas in OpenAPI components. | Complete utoipa coverage: domain models were missing ToSchema, blocking typed client generation and OpenAPI spec completeness. | YES: OpenAPI spec now includes 7 additional schema definitions. No runtime API behavior changes — same endpoints, same request/response shapes. Consumers regenerating from the spec will see new types. |
| 4.0.0 | 2026-04-02 | bd-owpvo | Refactor bill_run.rs into handler + service + repo layers. Handler (bill_run.rs) extracts HTTP concerns only. Service (bill_run_service.rs) contains business logic, orchestration, and event emission. Repo (bill_run_repo.rs) contains all SQL operations. No behavior change. | Separation of concerns: bill_run.rs mixed HTTP extraction, SQL queries, business logic, and event emission in a single 329-LOC function. Refactoring into layers improves testability and follows the pattern established in AR invoices. | YES: Internal module structure changed. `http::bill_run_repo` and `http::bill_run_service` are new public modules. No runtime API behavior changes — same endpoint, same request/response shapes, same transaction boundaries. |
| 3.0.0 | 2026-04-01 | bd-6y3bn | Add utoipa::path annotations to all admin endpoints (projection-status, consistency-check, list-projections). Register admin paths in OpenAPI spec. All handlers now have complete OpenAPI coverage. | Subscriptions response standardization: admin endpoints were missing from OpenAPI spec, blocking spec-driven tooling and documentation. | YES: OpenAPI spec now includes admin endpoints under the "Admin" tag. Consumers parsing the spec will see new paths. No runtime API behavior changes. |
| 2.2.8 | 2026-04-01 | bd-2gyqj | Update gated_invoice_creation to pass &VerifiedClaims via PlatformClient::service_claims(tenant_id). bill_run uses PlatformClient::new() constructor (no bearer token for service-to-service). | New typed client API requires per-request &VerifiedClaims for tenant-scoped auth. | No |
| 2.2.7 | 2026-04-01 | bd-aw020 | Replace inline reqwest HTTP calls in gated_invoice_creation.rs with platform-client-ar InvoicesClient. Remove local AR API model duplicates (CreateInvoiceRequest, FinalizeInvoiceRequest, Invoice) from models.rs. | Typed client conversion: eliminates manual HTTP wiring and duplicate type definitions, uses generated client for type safety. | No |
| 2.2.6 | 2026-03-31 | bd-5vmu6 | Convert to platform-sdk ModuleBuilder with SDK consumer adapter for ar.invoice_suspended. Replaces hand-rolled inline consumer. | SDK batch conversion — eliminate two classes of modules. | No |
| 2.2.5 | 2026-03-31 | bd-dwb41 | Add tenant_id column to bill_runs table (NOT NULL, indexed). Migration backfills existing rows. Bill run INSERT writes tenant_id from auth context. Idempotency SELECT and completion UPDATE both filter by tenant_id. New tenant boundary test proves cross-tenant isolation. | bill_runs table had no tenant_id — records from different tenants were commingled without attribution, same class as the P0 tenant isolation sweep. | No |
| 2.2.4 | 2026-03-31 | bd-k2w4b | Wire ar.invoice_suspended NATS consumer in main.rs. Subscribes to ar.events.ar.invoice_suspended, validates envelope, deserializes payload, calls handle_invoice_suspended with idempotency, sends failures to DLQ. | Consumer handler existed in consumer.rs but was never registered as a NATS subscriber — invoices reaching Suspended in AR left associated subscriptions Active (data inconsistency). | No |
| 2.2.3 | 2026-03-31 | bd-f1zwy | Wire gated invoice creation (advisory lock + UNIQUE constraint + attempt ledger) into execute_bill_run endpoint. Bill run retries for the same cycle now return idempotent skip instead of creating duplicate invoices. | Bill run endpoint bypassed cycle gating, allowing duplicate invoices on retries with different bill_run_id. | No |
| 2.2.2 | 2026-03-31 | bd-vnuvp.5 | Add tenant_id filter to 4 lifecycle queries (fetch_current_status, update_status, fetch_current_status_tx, update_status_tx). Public transition functions now require tenant_id parameter. Removes 2 redundant unscoped tenant_id fetches. | Tenant isolation: queries on subscriptions table used id without tenant_id, allowing cross-tenant data access in lifecycle transitions. | YES: transition_to_past_due, transition_to_suspended, transition_to_active now require tenant_id: &str parameter. |
| 2.2.1 | 2026-03-31 | bd-z5rek.3 | Migrate config.rs to ConfigValidator for multi-error startup validation. All config errors reported at once in table format. | Plug-and-play wave 2: consistent startup validation across all modules. | No |
| 2.2.0 | 2026-03-30 | bd-nhmgu | Export http module from lib.rs; add openapi_dump utility binary. | OpenAPI spec validation: offline dump needed for spec extraction and validation. | No |
| 2.1.0 | 2026-03-31 | bd-f97fk | OpenAPI spec at /api/openapi.json via utoipa 5.x on execute_bill_run. Bearer JWT SecurityScheme. Split http.rs (456 LOC) into http/bill_run.rs + http/health.rs + http/mod.rs. | Plug-and-play: OpenAPI + startup standardization. | No |
| 2.0.0 | 2026-03-31 | bd-f97fk | All errors migrated from ErrorResponse/ErrorBody to ApiError (platform-http-contracts). Admin endpoints also migrated. Error responses now include request_id field. | Plug-and-play: standard response envelopes. | YES: Error format changed to ApiError (error, message, request_id). Admin errors changed from ErrorBody to ApiError. |
| 1.0.0 | 2026-03-28 | bd-4zxqk | Initial proof. Recurring billing plan lifecycle (create/activate/pause/cancel/renew), invoice generation, usage-based metering, proration, trial management, plan versioning, admin endpoints, event publishing. 29 unit tests pass, clippy clean. | Subscriptions module complete and proven. All gates pass. | No |

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
