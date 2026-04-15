# Notifications — Revision History

> **What this file is:** The complete record of every change to this module after it was proven. Agents modifying this module must add a row here before committing. Products adopting a new version read this file to understand what changed.
> **Standard:** See `docs/VERSIONING.md` for the rules governing this file.

## Revisions

| Version | Date | Bead | What Changed | Why | Breaking? |
|---------|------|------|-------------|-----|-----------|
| 3.3.5 | 2026-04-15 | bd-nb4gy | Add `GET /api/inbox/mine` for self-scoped inbox listing. The handler derives tenant and user from `VerifiedClaims` instead of requiring `user_id` in the query string, and the router now exposes the new endpoint. Added a real Postgres-backed Axum test to prove the authenticated user only sees their own inbox rows. | Removes the need for callers to supply their own user_id on the inbox list route and gives a safer, self-scoped read path for the authenticated user. | No |
| 3.3.4 | 2026-04-15 | bd-g7zzj | Remove `envelope.payload.clone()` from 11 consumer dispatch sites in `main.rs` (5 arms), `consumer_tasks.rs` (5 arms), and `consumers/mod.rs` generic wrapper. Consumers now move or borrow the JSON Value rather than duplicating it per handler arm. | Hot-path memory: every dispatch doubled payload memory regardless of handler body size. | No |
| 3.3.3 | 2026-04-15 | bd-c25kf | Replace dynamic `format!` SQL assembly in `inbox::list_messages` with eight fixed query templates covering the unread/dismissed/category filter matrix. Added a real-db regression test that exercises all eight combinations. | Keeps inbox listing on static SQL strings only, while preserving the existing filter behavior and pagination semantics. | No |
| 3.3.2 | 2026-04-14 | bd-tjgby | Replace `PgPool::connect_lazy("postgres://localhost/fake")` in `test_admin_router_builds` with a real `setup_db()` using `PgPoolOptions::connect` + `sqlx::migrate!`. DB URL from `DATABASE_URL` env var, defaulting to `postgresql://notifications_user:notifications_pass@localhost:5437/notifications_db`. | Fake DSN meant the router test never touched the real schema — schema drift or missing columns would not fail at test time. | No |
| 3.3.1 | 2026-04-10 | bd-w1o7f | Fix tracking URL population in order_shipped notification — tracking_url helper was defined but not called in the shipping consumer dispatch path | Shipped notifications were missing clickable tracking links despite the helper existing | No |
| 3.2.0 | 2026-04-09 | bd-mozdb | Shipping notification templates (order_shipped, delivery_confirmed) + NATS consumers for sr.outbound.shipped and sr.outbound.delivered events. Seed migration for template rows. 8 integrated tests. | Verticals need platform-level shipping notifications with carrier tracking URLs instead of reimplementing per-vertical. | No |
| 3.1.2 | 2026-04-04 | bd-1d3a4,bd-nyb7t | Replace mock payment notification handler with real implementation + envelope schema validation | Mock handler returned static responses — real implementation processes payment events and validates envelope schema | No |
| 3.1.1 | 2026-04-04 | bd-0clpi | SoC: extract DLQ SQL into db/dlq_repo.rs | Separation of concerns — DLQ handler mixed HTTP logic with raw SQL queries | No |
| 3.1.0 | 2026-04-03 | bd-0kx54 | Add SendGridEmailSender implementing NotificationSender trait. Posts to SendGrid v3/mail/send API. Supports dynamic templates (via payload_json.sendgrid_template_id) and direct content mode (via payload_json.subject + body). New EMAIL_SENDER_TYPE=sendgrid config variant. SENDGRID_API_KEY env var. Config validation requires API key when sendgrid selected. | TrashTech Phase G needs real email delivery via SendGrid — HttpEmailSender format incompatible with SendGrid API. | No |
| 3.3.0 | 2026-04-09 | bd-owywe | Add tracking_url(carrier_code, tracking_number) -> Option<String> helper for UPS/FedEx/USPS tracking URLs. Integrated into order_shipped template. | Shipping notifications need clickable tracking links — Huber Power built a vertical-specific version that should be platform-level | No |
| 3.0.1 | 2026-04-02 | bd-azq84 | Fixed response model derives for OpenAPI schema generation | Plug-and-play standardization | No |
| 3.0.0 | 2026-04-02 | bd-y5v9j | All list endpoints (deliveries, inbox, DLQ) return PaginatedResponse<T> envelope with {data, pagination} shape. All error responses use ApiError (was per-handler ErrorResponse/InboxError/DlqError). utoipa annotations updated for all handlers. | Plug-and-play response standardization — consistent paginated envelopes and error shapes across all notification endpoints. | YES — List endpoints now return `{data: [...], pagination: {page, page_size, total_items, total_pages}}` instead of `{receipts: [...]}` / `{items: [...], total: N}`. Error responses use `ApiError` shape (`{error, message, request_id}`). Consumers must update response parsing. |
| 2.1.6 | 2026-04-01 | bd-thx8s | Fix event subject mismatch: subscribe to ar.events.ar.invoice_opened (was ar.events.invoice.issued). Align InvoiceIssuedPayload fields (amount_cents, due_at) with AR's InvoiceLifecyclePayload. | Consumer never received events — subject and payload schema both wrong. | No |
| 2.1.5 | 2026-04-01 | bd-manm4 | Add ApiDoc struct in http/mod.rs and openapi_dump binary for standalone spec generation. Info-only spec (handler annotations pending). | OpenAPI spec hygiene — all modules must emit complete specs for client codegen. | No |
| 2.1.4 | 2026-03-31 | bd-5vmu6.5 | Convert main.rs to platform-sdk ModuleBuilder. SDK handles DB, bus, outbox, CORS, JWT, health, metrics. 3 consumer adapters (invoice.issued, payment.succeeded, payment.failed). Dispatch loop + orphan recovery spawn in routes closure. SLO metrics registered with global registry. | SDK batch conversion — eliminate two classes of modules. | No |
| 2.1.3 | 2026-03-31 | — | Bump for module.toml + Cargo.toml SDK prep. | SDK conversion prep. | No |
| 2.1.2 | 2026-03-31 | bd-vnuvp.6 | Add tenant_id filter to 5 queries: close_calendar_reminders_sent lookup, escalation_sends idempotency checks (2x), mark_sent, reschedule_or_fail. Thread tenant_id parameter into mark_sent and reschedule_or_fail signatures. | Tenant isolation sweep — prevent cross-tenant data leakage in notification queries. | No |
| 2.1.1 | 2026-03-31 | bd-z5rek.3 | Migrate config.rs to ConfigValidator for multi-error startup validation. All config errors reported at once in table format. | Plug-and-play wave 2: consistent startup validation across all modules. | No |
| 2.1.0 | 2026-03-30 | bd-dhozf | Standard response envelopes. All list endpoints (deliveries, inbox, DLQ) wrapped in PaginatedResponse. All error responses migrated from custom ErrorResponse/ErrorBody to ApiError. Templates handlers converted from tuple error returns to ApiError. Added count_receipts repo function for delivery pagination. Added ToSchema derive to DeliveryReceipt. Enabled utoipa chrono+uuid features. | Plug-and-play envelope standard — consistent paginated responses and error shapes across all endpoints. | YES — list endpoints now return `{data, pagination}` envelope instead of bare arrays. Error responses use `ApiError` shape (`{status, error, message, request_id}`). Consumers update response parsing accordingly. |
| 1.0.0 | 2026-03-28 | bd-4ym3v | Initial proof. All unit tests passing (32/32). Clippy clean. Proof script created. Covers: scheduled dispatch with retry, DLQ replay/abandon, inbox CRUD, broadcast fan-out, escalation rules, template rendering, event consumers (invoice issued, payment succeeded/failed, low stock), outbox publishing, Prometheus metrics. | Module build complete — event-driven notification delivery ready for production. | — |

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
