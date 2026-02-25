# ar — Revision History

> **What this file is:** The complete record of every change to this module after it was proven. Agents modifying this module must add a row here before committing. Products adopting a new version read this file to understand what changed.
> **Standard:** See `docs/VERSIONING.md` for the rules governing this file.

## Revisions

| Version | Date | Bead | What Changed | Why | Breaking? |
|---------|------|------|-------------|-----|-----------|
| 1.0.11 | 2026-02-25 | bd-1uce | Added graceful shutdown with SIGTERM/SIGINT signal handling. Server now drains in-flight requests before closing DB pool on shutdown. | Zero-downtime deploys require graceful shutdown to avoid dropping in-flight requests and losing outbox events. | No |
| 1.0.10 | 2026-02-25 | bd-28tf | Replaced client-supplied `app_id` field with `VerifiedClaims` tenant extraction in `bill_usage_route`. Removed `app_id` from `BillUsageHttpRequest` struct. Handler now calls `super::tenant::extract_tenant(&claims)?` consistent with `capture_usage` in the same file. | Security audit finding C1: bill-usage endpoint allowed tenant spoofing via client-supplied `app_id` in request body. | No |
| 1.0.9 | 2026-02-25 | bd-ia5y.5 | Replaced client-supplied `app_id` field with `VerifiedClaims` tenant extraction in all four tax route handlers (`quote_tax_handler`, `lookup_cached_quote`, `commit_tax_handler`, `void_tax_handler`). Removed `app_id` from `QuoteTaxHttpRequest`, `LookupQuery`, `CommitTaxHttpRequest`, and `VoidTaxHttpRequest` structs. | Security audit finding C1: tax endpoints allowed tenant spoofing via client-supplied `app_id` in request body/query. | No |
| 1.0.8 | 2026-02-25 | bd-kjgf | Replaced hardcoded `CorsLayer` with dynamic `build_cors_layer(&config)`. Added `tracing::warn!()` at startup if `CORS_ORIGINS` is set to wildcard (`*`) and the environment (`ENV`) is not `development`. | Security audit: production should never run with wildcard CORS origins even if risk is low. | No |
| 1.0.6 | 2026-02-25 | bd-26f7 | Replaced `x-app-id` header extraction with `VerifiedClaims` in `credit_notes.rs:issue_credit_note_handler`. This handler was missed in the bd-27ov sweep. Now uses `super::tenant::extract_tenant(&claims)?` consistent with all other AR route handlers. | Security audit finding C1: credit note endpoint allowed tenant spoofing via client-supplied header. | No |
| 1.0.4 | 2026-02-24 | bd-2m7x | Removed legacy `AuthzLayer::from_env()` from router chain. This no-op middleware hardcoded every request as `Unauthenticated` and provided no real authorization — replaced by `optional_claims_mw` (wired in bd-217r). | Security audit finding C2: dead auth code creates false confidence. All real JWT verification is handled by `optional_claims_mw` → `RequirePermissionsLayer`. | No |
| 1.0.5 | 2026-02-24 | bd-27ov | Replaced all hardcoded `"test-app"` / `"default-tenant"` tenant strings (48 occurrences across 17 files) with JWT-based tenant extraction via `VerifiedClaims`. Added `routes::tenant::extract_tenant()` helper as single source of truth. All route handlers now derive `app_id` from the authenticated JWT `tenant_id` claim. Idempotency middleware uses request extensions. Webhook receiver uses env var / header. | Security audit finding H1: hardcoded tenant IDs meant zero tenant isolation — all requests operated on the same data scope regardless of caller identity. | No |
| 1.0.2 | 2026-02-24 | bd-217r | Wired `optional_claims_mw` JWT verification middleware into AR router chain. Middleware extracts verified claims from Authorization header and makes them available to route handlers via request extensions. Layer placed after rate_limit_middleware, before AuthzLayer — matching GL's proven pattern. | Security audit finding C1: AR had no JWT verification middleware, meaning all requests bypassed token validation. | No |
| 1.0.1 | 2026-02-22 | bd-18wm | Added exactly-once invoice lifecycle event emission: `ar.events.ar.invoice_opened` on invoice INSERT (create_invoice route) and `ar.events.ar.invoice_paid` on status→paid transition (handle_payment_succeeded). Both events emitted atomically within the domain transaction via `enqueue_event_tx_idempotent` (ON CONFLICT DO NOTHING). Idempotency key pattern: `ar.events.<event_type>:<invoice_id>` → deterministic UUID v5. Guard on `invoice_paid`: UPDATE WHERE status != 'paid' with RETURNING; no event if already paid. New contract types: `InvoiceLifecyclePayload`, `build_invoice_opened_envelope`, `build_invoice_paid_envelope` in `events/contracts/invoice_lifecycle.rs`. Integration tests prove exactly-once + replay safety against real Postgres. | Phase 51 cash flow forecasting requires real-time invoice lifecycle events on the NATS bus. | No |
| 1.0.0 | 2026-02-22 | bd-rqbr | Initial proof. Customer lifecycle (create/update/suspend/reactivate, status-gated operations), invoice lifecycle (draft → open → paid/void/written-off, aging buckets 0-30/31-60/61-90/90+), credit notes (issue credit against invoice, balance reduction + event fired), Tilled webhook ingestion (HMAC-SHA256 verification, idempotency via `event_id` deduplication), payment allocation (partial and full), write-offs, dunning scheduler, GL journal entry emission via NATS (`ar.invoice.created`, `ar.payment.received`), reconciliation, outbox/inbox pattern for event delivery, health (`/healthz`) and readiness (`/api/ready`) endpoints. All E2E tests passing. Proof command: `./scripts/proof_ar.sh`. Staging payment loop + webhook replay proof: `./scripts/staging/payment_loop.sh`. | AR module build complete. Invoice → webhook → posting path proven idempotent: duplicate Tilled events deduplicate via `event_id`, replay returns HTTP 200 with no state corruption. | — |

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
