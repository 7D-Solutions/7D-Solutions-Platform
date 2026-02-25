# payments — Revision History

> **What this file is:** The complete record of every change to this module after it was proven. Agents modifying this module must add a row here before committing. Products adopting a new version read this file to understand what changed.
> **Standard:** See `docs/VERSIONING.md` for the rules governing this file.

## Revisions

| Version | Date | Bead | What Changed | Why | Breaking? |
|---------|------|------|-------------|-----|-----------|
| 1.1.11 | 2026-02-25 | bd-2ivp | Added connection pool metrics (size, idle, active) to `/api/ready` response via `db_check_with_pool`. | Ops needs pool saturation visibility to detect connection exhaustion before it causes request timeouts. | No |
| 1.1.10 | 2026-02-25 | bd-289r | Fixed clippy warnings: removed unused imports, simplified borrowed expressions, removed redundant closures. | Enable cargo clippy -D warnings in CI. | No |
| 1.1.8 | 2026-02-25 | bd-1uce | Added graceful shutdown with SIGTERM/SIGINT signal handling. Server now drains in-flight requests before closing DB pool on shutdown. | Zero-downtime deploys require graceful shutdown to avoid dropping in-flight requests. | No |
| 1.1.7 | 2026-02-25 | bd-3fvu | Added 16 new edge-case integration tests for webhook signature verification in `tests/webhook_signature_edge_tests.rs`. Covers: HMAC correctness (case-insensitive header, empty body, secret rotation old/new/unknown), replay attack prevention (boundary 299s/301s, future timestamp, non-numeric timestamp), malformed payload handling (missing t=, missing v1=, empty header, empty v1, extra components), and webhook source dispatch (Internal always-pass, Stripe unsupported). | Test coverage for financial webhook signature verification — failed verification = missed payments or accepted fraudulent callbacks. | No |
| 1.1.6 | 2026-02-25 | bd-1qme | Replaced client-supplied `tenant_id` with JWT `VerifiedClaims` extraction in all checkout session handlers (`create`, `get`, `present`, `poll_status`) and `get_payment`. All SQL queries now scope by tenant_id from JWT. Added `extract_tenant` helper. Removed `tenant_id` from `PaymentQuery` query params. | Security audit C1: client-supplied tenant_id allows cross-tenant data access. Tenant must always come from verified JWT claims. | No |
| 1.1.5 | 2026-02-25 | bd-f813 | Added `RequirePermissionsLayer` with `PAYMENTS_MUTATE` permission to all mutation routes (checkout-session and admin endpoints). Webhook endpoint remains ungated. Moved mutation routes into a merged sub-router with `route_layer`. | Security audit H1: mutation routes lacked RBAC enforcement, allowing any authenticated user to perform writes. | No |
| 1.1.4 | 2026-02-25 | bd-5r2t | [PLACEHOLDER] | Internal update. | No |
| 1.1.3 | 2026-02-24 | bd-smuk | Added a `tracing::warn!()` at startup if `CORS_ORIGINS` is set to wildcard (`*`) and the environment (`ENV`) is not `development`. Added `env` field to `Config`. | Security audit: production should never run with wildcard CORS origins even if risk is low. | No |
| 1.1.2 | 2026-02-24 | bd-3kae | Replaced `allow_origin(Any)` CORS with configurable `CORS_ORIGINS` env var. Added `cors_origins` field to Config, `build_cors_layer()` function, and unit tests. Default `*` preserves dev behavior; production can set explicit origins. | Security audit finding C3: wildcard CORS allows any website to make cross-origin requests. | No |
| 1.1.1 | 2026-02-24 | bd-2m7x | Removed legacy `AuthzLayer::from_env()` from router chain. This no-op middleware hardcoded every request as `Unauthenticated` and provided no real authorization — replaced by `optional_claims_mw` (wired in bd-217r). | Security audit finding C2: dead auth code creates false confidence. All real JWT verification is handled by `optional_claims_mw` → `RequirePermissionsLayer`. | No |
| 1.1.0 | 2026-02-22 | bd-x0rt | Expanded checkout_session state machine: `pending` → `created→presented→completed\|failed\|canceled\|expired`. Migration backfills existing rows. Added `presented_at` column and status check constraint. New endpoints: POST `.../present` (idempotent created→presented on hosted page load) and GET `.../status` (lightweight status poll without client_secret). Webhook handler now updates sessions in `created` or `presented` state (was `pending`); status values are now `completed`/`canceled` (were `succeeded`/`cancelled`). Idempotency proven: webhook replay against terminal session updates 0 rows. 7 integration tests green. | Hosted pay portal lifecycle requires status transitions visible to both server and client. Polling endpoint avoids exposing client_secret in status checks. | No — new endpoints; status value changes apply only to sessions created after migration. Existing consumers checking for `succeeded`/`cancelled` must update to `completed`/`canceled`. |
| 1.0.0 | 2026-02-22 | bd-1b1x | Initial proof. Checkout session lifecycle (POST /api/payments/checkout-sessions, GET /api/payments/checkout-sessions/:id), Tilled.js payment intent creation, webhook ingestion with HMAC-SHA256 signature verification (tilled-signature header, timestamp replay-window enforcement ±5 min, rotation overlap: two secrets accepted simultaneously), payment collection consumer (ar.payment.collection.requested → Tilled charge attempt), retry window discipline (attempt 0/1/2 at +0/+3/+7 days anchored on attempted_at, no AR cross-module dependency), UNKNOWN blocking protocol (status=unknown excluded from retry scheduling until reconciled), idempotency keys (deterministic per app_id+payment_id+attempt_no), reconciliation workflow (unknown → succeeded/failed_* via PSP poll), metrics (Prometheus: request latency, attempt counts, consumer lag), health (/healthz) and readiness (/api/ready) endpoints. Proof command: ./scripts/proof_payments.sh. Signature vectors: 11 proven (positive + negative + rotation). UNKNOWN protocol: 5 integration vectors proven. | Payments module build complete. Tilled webhook → session status update path proven idempotent: duplicate webhooks update no rows (status guard on pending only). Signature replay rejected after 5 minutes. Key rotation overlap: both old and new secrets accepted during transition window. | — |

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
