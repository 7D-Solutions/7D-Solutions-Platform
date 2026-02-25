# ttp — Revision History

> **What this file is:** The complete record of every change to this module after it was proven. Agents modifying this module must add a row here before committing. Products adopting a new version read this file to understand what changed.
> **Standard:** See `docs/VERSIONING.md` for the rules governing this file.

## Revisions

| Version | Date | Bead | What Changed | Why | Breaking? |
|---------|------|------|-------------|-----|-----------|
| 2.1.5 | 2026-02-25 | bd-2wel.1 | Wired TracingContext from HTTP request extensions into envelope builders in `create_billing_run` handler. Envelopes now carry the real trace_id from the HTTP request. | Cross-module request tracing requires envelopes to carry the HTTP-originated trace_id for end-to-end correlation. | No |
| 2.1.4 | 2026-02-25 | bd-2ivp | Added connection pool metrics (size, idle, active) to `/api/ready` response via `db_check_with_pool`. | Ops needs pool saturation visibility to detect connection exhaustion before it causes request timeouts. | No |
| 2.1.3 | 2026-02-25 | bd-289r | Fixed clippy warnings: removed empty lines after doc comments, simplified borrowed expressions, removed redundant closures. | Enable cargo clippy -D warnings in CI. | No |
| 2.1.2 | 2026-02-25 | bd-1uce | Added graceful shutdown with SIGTERM/SIGINT signal handling. Server now drains in-flight requests before closing DB pool on shutdown. | Zero-downtime deploys require graceful shutdown to avoid dropping in-flight requests. | No |
| 2.1.1 | 2026-02-25 | bd-3m10 | Added integration tests: `billing_run_integration.rs` (9 tests for `collect_parties_to_bill` and `fetch_run_summary` — agreements, one-time charges, suspended exclusion, already-invoiced skip, multi-agreement sum, empty run) and `service_agreement_integration.rs` (7 tests for service agreements HTTP endpoint — status filters, sort order, 400/401 error cases). Made `billing_db` module `pub` for integration test access. | TTP is the billing backbone — untested billing and service-agreement logic is a revenue risk. These tests cover the DB query helpers and HTTP endpoint paths that were previously untested. | No |
| 2.1.0 | 2026-02-25 | bd-ia5y.2 | Removed `tenant_id` from `BillingRunRequest` JSON body and `ListQuery` query params in service_agreements. Tenant identity now derived from JWT `VerifiedClaims` in both `create_billing_run` and `list_service_agreements` handlers. Added `missing_claims_returns_401` test for billing endpoint. | Security fix C1: client-supplied `tenant_id` in billing request body and service-agreements query params allowed callers to specify an arbitrary tenant. Tenant must come from verified JWT only. | No — serde ignores unknown fields, so existing clients sending `tenant_id` in the body or query string will not error. The field is simply ignored and tenant is read from the JWT. Clients must ensure a valid JWT with `tenant_id` claim is present. |
| 2.0.3 | 2026-02-25 | bd-kjgf | Replaced hardcoded `CorsLayer` with dynamic `build_cors_layer(&config)`. Added `tracing::warn!()` at startup if `CORS_ORIGINS` is set to wildcard (`*`) and the environment (`ENV`) is not `development`. | Security audit: production should never run with wildcard CORS origins even if risk is low. | No |
| 2.0.1 | 2026-02-25 | bd-1wxy | Added `RequirePermissionsLayer` with `ttp.mutate` permission to all mutation routes (POST `/api/ttp/billing-runs`, POST `/api/metering/events`). Read routes (`GET /api/metering/trace`, `GET /api/ttp/service-agreements`) remain unenforced. | Security audit H1: mutation routes were accessible to any authenticated user without RBAC check. Now requires `ttp.mutate` permission in JWT claims. | No — existing consumers already sending JWTs with `ttp.mutate` permission are unaffected. Consumers without the permission must add it to their JWT claims. |
| 2.0.0 | 2026-02-25 | bd-2koo | Removed `tenant_id` from `IngestEventRequest` body and `TraceQuery` query params. Tenant identity now derived exclusively from JWT claims via `VerifiedClaims` extraction in request extensions. Both `ingest_events` and `get_trace` handlers now require a valid JWT with tenant_id claim. | Security fix C1: client-supplied tenant_id in request body/query allowed cross-tenant data access with a valid token. Tenant must come from verified JWT only. | YES — Consumers must stop sending `tenant_id` in metering request bodies and trace query params. Instead, include a valid JWT `Authorization` header. The middleware (wired in v1.0.1) extracts tenant_id from claims automatically. |
| 1.0.2 | 2026-02-24 | bd-2m7x | Removed legacy `AuthzLayer::from_env()` from router chain. This no-op middleware hardcoded every request as `Unauthenticated` and provided no real authorization — replaced by `optional_claims_mw` (wired in bd-217r). | Security audit finding C2: dead auth code creates false confidence. All real JWT verification is handled by `optional_claims_mw` → `RequirePermissionsLayer`. | No |
| 1.0.1 | 2026-02-24 | bd-217r | Wired `optional_claims_mw` JWT verification middleware into TTP router chain. Middleware extracts verified claims from Authorization header and makes them available to route handlers via request extensions. Layer placed after rate_limit_middleware, before AuthzLayer — matching GL's proven pattern. | Security audit finding C1: TTP had no JWT verification middleware, meaning all requests bypassed token validation. | No |
| 1.0.0 | 2026-02-22 | bd-2dq8 | Initial proof. Metering ingestion (idempotent, keyed by `idempotency_key`), price trace computation (deterministic, tenant-scoped), billing run execution (one run per tenant+period, one-time charges marked post-invoice, trace_hash linkage from metering to AR invoice line items). Health (`/healthz`) and readiness (`/api/ready`) endpoints. All E2E tests passing (metering_integration + billing_metering_integration). Proof command: `./scripts/proof_ttp.sh`. | TTP module build complete. Billing idempotency proven: billing runs are replay-safe with `was_noop:true` on re-invocation, and metering events deduplicate via `idempotency_key`. | — |

## How to read this table

- **Version:** The version in `Cargo.toml` after this change.
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
