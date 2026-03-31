# Plug-and-Play Wave 2: Module Assessment

> **22 Remaining Modules — Code-Level Investigation**
> March 30, 2026

---

## Executive Summary

This document presents a thorough code-level investigation of all 22 remaining modules requiring plug-and-play treatment. Every finding is based on reading the actual source code — main.rs, config.rs, HTTP handler files, and domain files — not estimates or assumptions.

The investigation revealed several reclassifications from the original prompt categories. Three modules originally classified as Simple (notifications, ttp, timekeeping) are actually Medium due to consumer complexity, external dependencies, or large handler surfaces. One Medium module (integrations) is actually Heavy due to QuickBooks OAuth, CDC polling workers, and webhook verification. Conversely, workforce-competence (Medium) is actually Simple aside from two file splits, and workflow (Heavy) is actually Medium with a small handler surface.

### Critical Findings

**Modules missing auto-migrations:** consolidation, quality-inspection, workforce-competence. These three modules have migration directories but do NOT call `sqlx::migrate!()` at startup. This must be added during treatment.

**Module with panic on bad config:** quality-inspection uses `panic!()` for invalid BUS_TYPE instead of graceful error handling. Must be fixed.

**Dual-database module:** quality-inspection requires a second database pool (WORKFORCE_COMPETENCE_DATABASE_URL). Unique across all 22 modules.

**Dead event bus code:** subscriptions has an unwired consumer (ar.invoice_suspended). ttp and timekeeping have event modules that are never spawned. Production has a 704-LOC events file that is internal-only.

**TTP version anomaly:** ttp is at v2.1.8 (not 1.0.0). Response format changes would be a MAJOR bump to v3.0.0.

---

## Summary Table

| Module | Ver | Endpts | List→PR | Splits | Config | Migrate | Event Bus | Effort | Beads | Wave |
|--------|-----|--------|---------|--------|--------|---------|-----------|--------|-------|------|
| consolidation | 1.0.0 | ~36 | ~8 | 0 | FF | **MISS** | None | Simple | 1 | A |
| customer-portal | 1.0.1 | ~15 | ~2 | 0 | FF | Yes | Outbox | Simple | 1 | A |
| numbering | 1.0.0 | ~9 | 0 | 0 | FF | Yes | Outbox | Simple | 1 | A |
| pdf-editor | 1.0.0 | ~20 | ~4 | 0 | FF | Yes | Outbox | Simple | 1 | A |
| subscriptions | 1.0.0 | ~9 | 0 | 0 | FF | Yes | Outbox* | Simple | 1 | A |
| workforce-comp | 1.0.0 | ~12 | 0 | 2 | FF | **MISS** | None | Simple | 1 | A |
| notifications | 1.0.0 | ~27 | ~5 | 0 | FF+V | Yes | 3C+Pub | Medium | 2 | B |
| timekeeping | 1.0.0 | ~46 | ~12 | 0 | FF | Yes | Dead | Medium | 2 | B |
| ttp | 2.1.8 | ~11 | ~2 | 0* | FF | Yes | Dead | Medium | 2 | B |
| shipping-recv | 1.0.0 | ~21 | ~4 | 1 | FF | Yes | 2C+Pub | Medium | 2 | B |
| maintenance | 1.0.0 | ~35 | ~8 | 1 | FF | Yes | 2C+Sched | Medium | 2 | B |
| payments | 1.1.20 | ~16 | ~2 | 1 | FF | Yes | 1C+Pub | Medium | 2 | B |
| quality-insp | 1.0.0 | ~20 | ~4 | 1 | **FF!** | **MISS** | 2C | Medium | 2 | B |
| reporting | 1.0.0 | ~16 | ~2 | 1 | FF | Yes | None | Medium | 2 | B |
| fixed-assets | 1.0.0 | ~23 | ~5 | 2 | FF | Yes | 1C+Pub | Medium | 2 | C |
| production | 1.0.1 | ~36 | ~8 | 2 | FF | Yes | Dead | Medium | 2 | C |
| workflow | 1.0.0 | ~11 | ~2 | 3 | FF | Yes | Pub | Medium | 2 | C |
| integrations | 1.0.1 | ~23 | ~3 | 2 | FF | Yes | Pub+Wkrs | Heavy | 3 | C |
| treasury | 1.0.1 | ~24 | ~5 | 3 | FF | Yes | 2C+Pub | Heavy | 2 | D |
| gl | 1.0.0 | ~41 | ~6 | 7 | FF | Yes | 11C+Pub | Heavy | 3 | D |
| ap | 1.0.0 | ~39 | ~8 | 9 | FF | Yes | 2C+Pub | Heavy | 3 | E |
| ar | 1.0.64 | ~95 | ~15 | 8 | FF | Yes | 1C+Pub | Heavy | 4 | F |

**Legend:** FF = fail-fast config, FF+V = fail-fast + validate(), FF! = fail-fast + panic, MISS = migrations missing, C = consumers, Pub = outbox publisher, Sched = scheduler, Wkrs = background workers, Dead = unwired event code, * = near-limit files or pre-split needed, PR = PaginatedResponse.

---

## Individual Module Assessments

### Module: consolidation

- **Version:** 1.0.0 | **Category:** Simple | **Effort:** Simple | **Recommended beads:** 1
- **Handlers:** 5 files, ~29 handler functions
- **Endpoints:** ~36 routes (config CRUD, groups, entities, COA mappings, elimination rules, FX policies, intercompany, statements)
- **Response migration:** ~8 list endpoints need PaginatedResponse (list_groups, list_entities, list_coa_mappings, list_elimination_rules, list_fx_policies, etc.); all handlers need ApiError (currently custom ErrorBody inline json!())
- **File splits needed:** None needed. Largest: domain/config/tests.rs at 483 LOC.
- **Config:** Result<Self, String>, fail-fast on first error. Validates DATABASE_URL, PORT, CORS. No ConfigValidator.
- **Migrations:** MISSING. main.rs does NOT call sqlx::migrate!(). Migrations directory exists but not executed at startup.
- **Event bus:** None. No event-bus dependency, no consumers, no outbox.
- **Special concerns:** External GL service dependency via gl_base_url config. Uses optional JWT verification. No event bus at all.

### Module: customer-portal

- **Version:** 1.0.1 | **Category:** Simple | **Effort:** Simple | **Recommended beads:** 1
- **Handlers:** 6 files, ~11 handler functions
- **Endpoints:** ~15 routes (auth: login/refresh/logout, status feed, admin CRUD, docs)
- **Response migration:** Few list endpoints (status feed, admin lists); all error responses need ApiError (currently inline json!()). Auth responses are custom structs (access_token, refresh_token).
- **File splits needed:** None needed. Largest: http/auth.rs at 333 LOC.
- **Config:** Result<Self, String>, fail-fast. Validates DATABASE_URL, PORTAL_JWT keys, PORT, CORS. No ConfigValidator.
- **Migrations:** Present. sqlx::migrate!() called in main.rs.
- **Event bus:** Outbox-only publisher (enqueue_portal_event). No consumers. Uses event-bus crate.
- **Special concerns:** Argon2 password hashing. RS256 JWT token generation (portal-scoped, separate from platform JWT). Refresh token rotation with idempotency. External doc-mgmt dependency for document visibility.

### Module: notifications

- **Version:** 1.0.0 | **Category:** Simple (reclassified Medium) | **Effort:** Medium | **Recommended beads:** 2
- **Handlers:** 8 files, ~18 handler functions
- **Endpoints:** ~27 routes (DLQ: list/get/replay/abandon, Sends: send/get/query, Inbox: list/get/read/unread/dismiss/undismiss, Templates: publish/get, Admin: projection-status/consistency/list)
- **Response migration:** ~5 list endpoints need PaginatedResponse (list_dlq, query_deliveries, list_inbox, list_projections, etc.); all error responses need ApiError (currently admin_types::ErrorBody).
- **File splits needed:** None needed. Largest: http/dlq.rs at 475 LOC, consumer_tasks.rs at 441 LOC.
- **Config:** Result<Self, String> with separate .validate() method. Extensive: DATABASE_URL, BUS_TYPE, NATS_URL conditional, HTTP endpoint checks for senders, retry policy range validation. No ConfigValidator.
- **Migrations:** Present. sqlx::migrate!() called in main.rs.
- **Event bus:** 3 consumers wired: start_invoice_issued_consumer, start_payment_succeeded_consumer, start_payment_failed_consumer. Plus outbox publisher. NATS or InMemory.
- **Special concerns:** Dual sender types (Email + SMS, each Logging or HTTP). Background dispatcher loop with retry logic. DLQ for failed deliveries. Escalation rules engine. Template rendering. Most complex config of the simple candidates.

### Module: numbering

- **Version:** 1.0.0 | **Category:** Simple | **Effort:** Simple | **Recommended beads:** 1
- **Handlers:** 4 files, ~4 handler functions
- **Endpoints:** ~9 routes (allocate, confirm, policy upsert, policy get, plus ops)
- **Response migration:** No list endpoints. 4 handlers need ApiError (currently custom ErrorResponse struct).
- **File splits needed:** None needed. Largest: http/allocate.rs at 452 LOC.
- **Config:** Result<Self, String>, fail-fast. Validates DATABASE_URL, BUS_TYPE, NATS_URL conditional, PORT. No ConfigValidator.
- **Migrations:** Present. sqlx::migrate!() called in main.rs.
- **Event bus:** Outbox publisher only. No consumers. Emits NumberAllocated events.
- **Special concerns:** Advisory lock mechanism (SELECT FOR UPDATE) for gap-free sequences. Idempotency key tracking. Format templates (year/prefix/padding). Bench and DLQ replay drill binaries.

### Module: pdf-editor

- **Version:** 1.0.0 | **Category:** Simple | **Effort:** Simple | **Recommended beads:** 1
- **Handlers:** 7 files, ~16 handler functions
- **Endpoints:** ~20 routes (templates, fields, submissions, generate, annotations)
- **Response migration:** ~4 list endpoints need PaginatedResponse; all error responses need ApiError (currently inline json!() with FormError/SubmissionError enums).
- **File splits needed:** None needed. Largest: domain/annotations/renderers.rs at 447 LOC.
- **Config:** Result<Self, String>, fail-fast. Explicitly forbids CORS wildcard. No ConfigValidator.
- **Migrations:** Present. sqlx::migrate!() called in main.rs.
- **Event bus:** Outbox publisher only. No consumers.
- **Special concerns:** 50 MB custom body limit for PDF uploads (non-standard, must be preserved during treatment).

### Module: subscriptions

- **Version:** 1.0.0 | **Category:** Simple | **Effort:** Simple | **Recommended beads:** 1
- **Handlers:** 1 file (http.rs), ~1 handler function
- **Endpoints:** ~9 routes (execute_bill_run is the core endpoint, plus health/ready/version and sub-routers)
- **Response migration:** No list endpoints. 1 handler needs ApiError (currently custom ErrorResponse with details field).
- **File splits needed:** None needed. http.rs at 456 LOC (near limit but under). cycle_gating.rs at 450 LOC.
- **Config:** Result<Self, String>, fail-fast. BUS_TYPE invalid values log warning (backward-compat). No ConfigValidator.
- **Migrations:** Present. sqlx::migrate!() called in main.rs.
- **Event bus:** Consumer code exists for ar.invoice_suspended but is NOT wired/spawned in main.rs (dead code). Outbox publisher present.
- **Special concerns:** Dead consumer code that should be evaluated: wire it in or remove it. Near-limit file sizes (456 and 450 LOC).

### Module: ttp

- **Version:** 2.1.8 | **Category:** Simple (reclassified Medium) | **Effort:** Medium | **Recommended beads:** 2
- **Handlers:** 4 files, ~4 handler functions
- **Endpoints:** ~11 routes (billing, metering, service agreements)
- **Response migration:** ~2 list endpoints need PaginatedResponse; handlers need ApiError (currently custom ErrorBody with code field).
- **File splits needed:** None needed but watch: domain/billing.rs at 488 LOC (dangerously close to 500 limit; OpenAPI annotations will push it over).
- **Config:** Result<Self, String>, fail-fast. No ConfigValidator.
- **Migrations:** Present. sqlx::migrate!() called in main.rs.
- **Event bus:** No consumers. No publisher wired (events enveloped but never published). Dead event code.
- **Special concerns:** CRITICAL: v2.1.8 means this is NOT a 1.0.0 module; already at 2.x. External dependencies: AR client (request-time env var lookup), TenantRegistry client. No timeout/retry visible on external calls. billing.rs will need a pre-split before OpenAPI work.

### Module: timekeeping

- **Version:** 1.0.0 | **Category:** Simple (reclassified Medium) | **Effort:** Medium | **Recommended beads:** 2
- **Handlers:** 11 files, ~44 handler functions
- **Endpoints:** ~46 routes (employees, projects, tasks, entries, approvals, allocations, exports, rates, billing-runs)
- **Response migration:** ~12 list endpoints need PaginatedResponse (list_employees, list_projects, list_entries, list_approvals, list_allocations, list_exports, list_rates, etc.); all handlers need ApiError (currently inline json!()).
- **File splits needed:** None needed. Largest: domain/approvals/service.rs at 450 LOC.
- **Config:** Result<Self, String>, fail-fast. No ConfigValidator.
- **Migrations:** Present. sqlx::migrate!() called in main.rs.
- **Event bus:** No consumers. No publisher wired (events/mod.rs exists but not spawned). Dead event code.
- **Special concerns:** Large handler surface (44 functions across 11 files) makes this medium effort despite no file splits. Idempotency-key header support. Dead event bus code to evaluate.

### Module: shipping-receiving

- **Version:** 1.0.0 | **Category:** Medium | **Effort:** Medium | **Recommended beads:** 2
- **Handlers:** 4+ files, ~21 handler functions
- **Endpoints:** ~21 routes (shipments CRUD, inspection routing, refs)
- **Response migration:** ~4 list endpoints need PaginatedResponse (list_shipments uses limit/offset); all handlers need ApiError (currently custom ErrorBody).
- **File splits needed:** 1 file: domain/shipments/guards.rs at 501 LOC (just over limit, needs split).
- **Config:** Result<Self, String>, fail-fast. Validates DATABASE_URL, BUS_TYPE, NATS_URL, PORT. No ConfigValidator.
- **Migrations:** Present. sqlx::migrate!() called in main.rs.
- **Event bus:** 2 consumers wired: start_po_approved_consumer, start_so_released_consumer. Plus outbox publisher.
- **Special concerns:** Inventory integration supports dual-mode (HTTP or deterministic). External HTTP dependency on config.inventory_url (optional).

### Module: maintenance

- **Version:** 1.0.0 | **Category:** Medium | **Effort:** Medium | **Recommended beads:** 2
- **Handlers:** 10 files, ~31 handler functions
- **Endpoints:** ~35 routes (assets, calibration, downtime, meters, plans, work orders, labor, parts)
- **Response migration:** ~8 list endpoints need PaginatedResponse; all handlers need ApiError (currently custom error handler functions per domain).
- **File splits needed:** 1 file: domain/work_orders/service.rs at 551 LOC (needs split into service + state_machine or similar).
- **Config:** Result<Self, String>, fail-fast. Unique: MAINTENANCE_SCHED_INTERVAL_SECS config (defaults 60). No ConfigValidator.
- **Migrations:** Present. sqlx::migrate!() called in main.rs.
- **Event bus:** 2 consumers: production_workcenter_bridge, production_downtime_bridge. Plus outbox publisher. Plus unique scheduler polling task.
- **Special concerns:** Scheduler polling task runs every N seconds (config-driven). Production event bridges require NATS even if HTTP-only.

### Module: payments

- **Version:** 1.1.20 | **Category:** Medium | **Effort:** Medium | **Recommended beads:** 2
- **Handlers:** 6 files, ~8 handler functions
- **Endpoints:** ~16 routes (checkout sessions, payments, admin, webhook)
- **Response migration:** No bare Vec returns; all responses are custom-typed. Still need ApiError migration (currently ErrorBody with sanitized errors). Admin uses projections platform types.
- **File splits needed:** 1 file: http/checkout_sessions.rs at 599 LOC (needs split into handlers + session_logic).
- **Config:** Result<Self, String>, fail-fast. Conditional: PAYMENTS_PROVIDER=tilled requires TILLED_API_KEY, TILLED_ACCOUNT_ID, TILLED_WEBHOOK_SECRET. No ConfigValidator.
- **Migrations:** Present. sqlx::migrate!() called in main.rs.
- **Event bus:** 1 consumer: start_payment_collection_consumer. Outbox publisher. InMemory bus initialized; NATS branch exists but unused.
- **Special concerns:** Tilled webhook integration with HMAC-SHA256 signature verification and secret rotation support. Admin endpoints require X-Admin-Token header (separate authz layer). Webhook endpoint is unauthenticated (signature validation instead).

### Module: quality-inspection

- **Version:** 1.0.0 | **Category:** Medium | **Effort:** Medium | **Recommended beads:** 2
- **Handlers:** 4 files, ~16 handler functions
- **Endpoints:** ~20 routes (inspection plans, receiving/in-process/final inspections, accept/reject/hold/release)
- **Response migration:** ~4 list endpoints need PaginatedResponse (by_lot, by_part_rev, by_receipt, by_wo); all handlers need ApiError (currently custom error responses).
- **File splits needed:** 1 file: domain/service.rs at 683 LOC (needs split into service + inspection_logic or by inspection type).
- **Config:** Result<Self, String>, fail-fast. CRITICAL: BUS_TYPE uses panic!() instead of Result for invalid values. Requires WORKFORCE_COMPETENCE_DATABASE_URL (dual DB). No ConfigValidator.
- **Migrations:** MISSING. main.rs does NOT call sqlx::migrate!(). Migrations directory exists with 4 files but not executed at startup.
- **Event bus:** 2 consumers: start_receipt_event_bridge, start_production_event_bridge. No outbox (read-only for events; publishes nothing). NATS hardcoded as default BUS_TYPE.
- **Special concerns:** CRITICAL ISSUES: (1) No migration auto-run, (2) Dual database pool (main + workforce-competence DB), (3) panic!() on invalid BUS_TYPE instead of graceful error. Config uses String for bus_type instead of enum.

### Module: reporting

- **Version:** 1.0.0 | **Category:** Medium | **Effort:** Medium | **Recommended beads:** 2
- **Handlers:** 8 files, ~10 handler functions
- **Endpoints:** ~16 routes (PL, balance sheet, cashflow, AR aging, AP aging, KPIs, forecast, rebuild, admin)
- **Response migration:** No bare Vec returns; all responses are custom typed (StatementResponse, AgingResponse, etc.). Still need ApiError migration. Admin uses standard pattern.
- **File splits needed:** 1 file: domain/statements/cashflow.rs at 649 LOC (needs split into calculation + formatting).
- **Config:** Result<Self, String>, fail-fast. Minimal config (no bus_type, no nats_url). No ConfigValidator.
- **Migrations:** Present. sqlx::migrate!() called in main.rs.
- **Event bus:** None. Read-only reporting module. No consumers, no publisher, no event bus initialization. Projection-based.
- **Special concerns:** Read-only module consuming pre-built projection views. App-ID scoped database resolver seam (db::resolve_pool()). No event bus integration needed.

### Module: fixed-assets

- **Version:** 1.0.0 | **Category:** Medium | **Effort:** Medium | **Recommended beads:** 2
- **Handlers:** 5 files, ~19 handler functions
- **Endpoints:** ~23 routes (categories, assets, depreciation schedules/runs, disposals, admin)
- **Response migration:** ~5 list endpoints need PaginatedResponse (list_categories, list_assets, list_runs, list_disposals, etc.); all handlers need ApiError (currently inline json!()).
- **File splits needed:** 2 files: domain/assets/models.rs at 536 LOC, domain/depreciation/service.rs at 530 LOC. Both need splits.
- **Config:** Result<Self, String>, fail-fast. Validates DATABASE_URL, BUS_TYPE, NATS_URL, PORT, CORS. No ConfigValidator.
- **Migrations:** Present. sqlx::migrate!() called in main.rs.
- **Event bus:** 1 consumer: start_ap_bill_approved_consumer (asset capitalization from AP). Plus outbox publisher.
- **Special concerns:** AP bill approved consumer triggers asset capitalization workflow. Two files over 500 LOC need pre-splits.

### Module: production

- **Version:** 1.0.1 | **Category:** Medium | **Effort:** Medium | **Recommended beads:** 2
- **Handlers:** 10 files, ~32 handler functions
- **Endpoints:** ~36 routes (workcenters, work orders, operations, time entries, routings, downtime, FG receipt, component issue)
- **Response migration:** ~8 list endpoints need PaginatedResponse (list_workcenters, list_routings, list_operations, list_time_entries, etc.); all handlers need ApiError (currently inline json!()).
- **File splits needed:** 2 files: events/mod.rs at 704 LOC (split into event types + publishing), domain/routings.rs at 541 LOC (split into routing_service + step_management).
- **Config:** Result<Self, String>, fail-fast. Minimal (DATABASE_URL, PORT, CORS). No ConfigValidator. No BUS_TYPE config.
- **Migrations:** Present. sqlx::migrate!() called in main.rs.
- **Event bus:** No consumers. No event bus initialized in main.rs. events/mod.rs defines domain events but they are internal-only (not published to NATS).
- **Special concerns:** Complex workflow state machines for work orders and routings. Large events file (704 LOC). Permission gating via RequirePermissionsLayer (PRODUCTION_READ/PRODUCTION_MUTATE).

### Module: integrations

- **Version:** 1.0.1 | **Category:** Medium (reclassified Heavy) | **Effort:** Heavy | **Recommended beads:** 3
- **Handlers:** 6 files, ~17 handler functions
- **Endpoints:** ~23 routes (connectors, external refs, OAuth, QBO invoice, webhooks)
- **Response migration:** ~3 list endpoints need PaginatedResponse (list_by_entity, list_connectors, list_connector_types); handlers need ApiError (currently custom ErrorBody struct).
- **File splits needed:** 2 files: domain/external_refs/service.rs at 640 LOC, domain/qbo/client.rs at 510 LOC. Also domain/webhooks/qbo_normalizer.rs at 490 LOC (close).
- **Config:** Result<Self, String>, fail-fast. Validates DATABASE_URL, BUS_TYPE, NATS_URL. Conditional: QBO_CLIENT_ID, QBO_CLIENT_SECRET, QBO_TOKEN_URL for QuickBooks. No ConfigValidator.
- **Migrations:** Present. sqlx::migrate!() called in main.rs.
- **Event bus:** No consumers. Outbox publisher only.
- **Special concerns:** HEAVY EXTERNAL DEPS: QuickBooks OAuth integration (token refresh worker, 30s interval), CDC polling worker (15m interval), Stripe + GitHub webhook ingest, HMAC-SHA256 signature verification. Two background workers. EDI transaction processing. Highest risk medium candidate.

### Module: workforce-competence

- **Version:** 1.0.0 | **Category:** Medium (reclassified Simple) | **Effort:** Simple | **Recommended beads:** 1
- **Handlers:** 3 files, ~8 handler functions
- **Endpoints:** ~12 routes (artifacts, assignments, acceptance authorities, authorization queries)
- **Response migration:** No list endpoints (single-item lookups only). 7 handlers need ApiError (currently inline json!()).
- **File splits needed:** 2 files: domain/service.rs at 609 LOC, domain/acceptance_authority.rs at 542 LOC. Both need splits.
- **Config:** Result<Self, String>, fail-fast. Minimal (DATABASE_URL, PORT, CORS). No ConfigValidator.
- **Migrations:** MISSING. main.rs does NOT call sqlx::migrate!(). No event bus either.
- **Event bus:** None. No event bus dependency, no consumers, no publisher.
- **Special concerns:** No migrations auto-run. No event bus. Simplest handler surface but 2 files need splits. Idempotency key handling present.

### Module: gl

- **Version:** 1.0.0 | **Category:** Heavy | **Effort:** Heavy | **Recommended beads:** 3
- **Handlers:** 20 files, ~35 handler functions
- **Endpoints:** ~41 routes (accounts, journal entries, trial balance, P&L, balance sheet, cashflow, period close, accruals, revrec, FX rates, close checklist, exports, reporting currency, admin)
- **Response migration:** ~6 list endpoints need PaginatedResponse (account activity with limit/offset, close checklist items, etc.); all handlers need ApiError (currently custom ErrorBody).
- **File splits needed:** 7 files over 500 LOC (all allowlisted): repos/revrec_repo.rs (857), accruals.rs (763), consumers/gl_inventory_consumer.rs (667), consumers/fixed_assets_depreciation.rs (527), services/balance_sheet_service.rs (510), consumers/ar_tax_liability.rs (518), consumers/ap_vendor_bill_approved.rs (498). Split strategy: separate repo layers, split consumers by event type, extract calculation logic from services.
- **Config:** Result<Self, String>, fail-fast. Validates DATABASE_URL, BUS_TYPE, NATS_URL, PORT, CORS. DLQ validation flag. No ConfigValidator.
- **Migrations:** Present. sqlx::migrate!() called in main.rs.
- **Event bus:** 11 consumers wired: gl_posting, gl_reversal, gl_writeoff, gl_inventory (COGS), ar_tax_committed, ar_tax_voided, fixed_assets_depreciation, gl_credit_note, ap_vendor_bill_approved, gl_fx_realized, timekeeping_labor_cost. Plus outbox publisher.
- **Special concerns:** Most consumers of any module (11). In-memory CurrencyConfigRegistry for per-tenant reporting currency. Period close DLQ validation gate. Prometheus GlMetrics. Revenue recognition engine. 25K+ total LOC.

### Module: ap

- **Version:** 1.0.0 | **Category:** Heavy | **Effort:** Heavy | **Recommended beads:** 3
- **Handlers:** 12 files, ~33 handler functions
- **Endpoints:** ~39 routes (vendors, purchase orders, bills, allocations, payment terms, payment runs, reports, tax reports, admin)
- **Response migration:** ~8 list endpoints need PaginatedResponse (list vendors, POs, bills, allocations, payment terms, payment runs, etc.); all handlers need ApiError (currently ErrorBody{code, message}).
- **File splits needed:** 9 files over 500 LOC: domain/po/service.rs (716), domain/match/engine.rs (711), domain/bills/service.rs (691), domain/vendors/service.rs (632), domain/tax/service.rs (618), domain/reports/aging.rs (609), domain/payment_runs/builder.rs (572), domain/payment_runs/execute.rs (566), domain/po/approve.rs (558). Split strategy: extract match algorithm, separate builder/executor patterns, split service layers.
- **Config:** Result<Self, String>, fail-fast. Validates DATABASE_URL, BUS_TYPE, NATS_URL, PORT, CORS. No ConfigValidator.
- **Migrations:** Present. sqlx::migrate!() called in main.rs.
- **Event bus:** 2 consumers: inventory_item_received (bill line matching). Plus outbox publisher.
- **Special concerns:** 9 files over 500 LOC (most of any module). Tax-core dependency for tax quoting/commit/void. 2-way/3-way matching engine (711 LOC). Payment run builder + executor pattern. 16K+ total LOC.

### Module: ar

- **Version:** 1.0.64 | **Category:** Heavy | **Effort:** Heavy | **Recommended beads:** 4
- **Handlers:** 23 files, ~62 handler functions
- **Endpoints:** ~95 routes (invoices, customers, charges, payments, subscriptions, credit notes, refunds, disputes, reconciliation, aging, usage, tax, tax config, tax rules, dunning, webhooks, allocation, events, admin)
- **Response migration:** ~15 list endpoints need PaginatedResponse; all handlers need ApiError (currently ErrorResponse{code, message}). Webhook event processing responses must NOT change (external contract).
- **File splits needed:** 8 files over 500 LOC (all allowlisted): http/webhooks.rs (1256), credit_notes.rs (822), http/invoices.rs (793), http/payment_methods.rs (780), tilled/types.rs (595), http/subscriptions.rs (562), http/charges.rs (531), finalization.rs (737). Split strategy: webhooks by event type, credit_notes into lifecycle + calculations, handler files into handlers + request_types.
- **Config:** Result<Self, String>, fail-fast. TILLED_WEBHOOK_SECRET required (with fallback order). PARTY_MASTER_URL for customer verification. No ConfigValidator.
- **Migrations:** Present. sqlx::migrate!() called in main.rs.
- **Event bus:** 1 consumer: payment_succeeded_consumer. Plus outbox publisher.
- **Special concerns:** HIGHEST COMPLEXITY: Tilled payment integration with HMAC-SHA256 webhook verification (replay window guard, constant-time comparison). 6+ webhook event types with dedicated processors. Party Master integration. v1.0.64 indicates heavy iteration (62 patch versions). 8 files over 500 LOC. 27K+ total LOC. webhooks.rs alone is 1256 LOC. MUST BE LAST.

### Module: treasury

- **Version:** 1.0.1 | **Category:** Heavy | **Effort:** Heavy | **Recommended beads:** 2
- **Handlers:** 8 files, ~20 handler functions
- **Endpoints:** ~24 routes (bank accounts, transactions, import, reconciliation, GL posting, reports, admin)
- **Response migration:** ~5 list endpoints need PaginatedResponse; all handlers need ApiError (currently custom ErrorBody).
- **File splits needed:** 3 files over 500 LOC: domain/accounts/service.rs (691), domain/import/service.rs (656), domain/reports/forecast.rs (552). Split strategy: separate account types, split import orchestrator from parsers, extract forecast calculations.
- **Config:** Result<Self, String>, fail-fast. Validates DATABASE_URL, BUS_TYPE, NATS_URL, PORT, CORS. No ConfigValidator.
- **Migrations:** Present. sqlx::migrate!() called in main.rs.
- **Event bus:** 2 consumers (payment reconciliation triggers). Plus outbox publisher.
- **Special concerns:** Decimal arithmetic fix in v1.0.1 (replaced f64 with rust_decimal::Decimal for financial accuracy). CSV parsers for Chase and AMEX proprietary formats. Financial accuracy is critical.

### Module: workflow

- **Version:** 1.0.0 | **Category:** Heavy (reclassified Medium) | **Effort:** Medium | **Recommended beads:** 2
- **Handlers:** 4 files, ~8 handler functions
- **Endpoints:** ~11 routes (definitions CRUD, instances lifecycle, health)
- **Response migration:** ~2 list endpoints need PaginatedResponse; handlers need ApiError (currently custom error responses).
- **File splits needed:** 3 files over 500 LOC: domain/routing.rs (672), domain/instances.rs (603), domain/escalation.rs (561). Split strategy: extract routing engine from step definitions, separate instance state machine from persistence, split escalation rules from timeout handling.
- **Config:** Result<Self, String>, fail-fast. Validates DATABASE_URL, BUS_TYPE, NATS_URL, PORT, CORS. No ConfigValidator.
- **Migrations:** Present. sqlx::migrate!() called in main.rs.
- **Event bus:** No consumers. Outbox publisher only (workflow emits events, other modules consume them).
- **Special concerns:** Durable workflow execution engine with sequential/parallel/conditional step routing. State machine for instance lifecycle. 3 domain files need splits but handler surface is small.

---

## Recommended Wave Grouping

Modules are grouped into waves based on complexity and dependency order. Each wave can have multiple beads running in parallel. Waves should be executed sequentially since later waves may depend on patterns established in earlier ones.

### Wave A — Quick Wins (6 modules, 6 beads)

Simple modules with no file splits needed (or splits only). Copy the proven pattern from Inventory/Party/BOM.

- **consolidation** — 1 bead. Add migrations call, response envelopes, OpenAPI, ConfigValidator.
- **customer-portal** — 1 bead. Standard treatment. Auth response structs stay custom.
- **numbering** — 1 bead. Standard treatment. Only 4 handlers, no list endpoints.
- **pdf-editor** — 1 bead. Standard treatment. Preserve 50MB body limit.
- **subscriptions** — 1 bead. Standard treatment. Evaluate dead consumer code.
- **workforce-competence** — 1 bead. Pre-split 2 files, add migrations call, standard treatment.

### Wave B — Medium Core (8 modules, 16 beads)

Modules with file splits, consumer wiring, or larger handler surfaces. Some have event bus integration that must be preserved.

- **notifications** — 2 beads. 3 consumers to preserve, complex config with sender validation.
- **timekeeping** — 2 beads. Large handler surface (44 functions, 12 list endpoints). Evaluate dead event code.
- **ttp** — 2 beads. Pre-split billing.rs (488 LOC, will exceed with OpenAPI). External AR/TenantRegistry deps. NOTE: v2.1.8 means MAJOR bump goes to v3.0.0.
- **shipping-receiving** — 2 beads. Split guards.rs (501 LOC). 2 consumers + outbox. Inventory dual-mode integration.
- **maintenance** — 2 beads. Split work_orders/service.rs (551 LOC). 2 consumers + scheduler task.
- **payments** — 2 beads. Split checkout_sessions.rs (599 LOC). Tilled webhook signature verification must be preserved. X-Admin-Token authz.
- **quality-inspection** — 2 beads. Split service.rs (683 LOC). Fix panic!() on BUS_TYPE. Add migrations. Dual DB pool.
- **reporting** — 2 beads. Split cashflow.rs (649 LOC). Read-only module, no event bus needed.

### Wave C — Medium Complex (4 modules, 9 beads)

Modules with multiple file splits or significant external dependencies.

- **fixed-assets** — 2 beads. Split 2 files. 1 consumer (AP bill approved).
- **production** — 2 beads. Split 2 files (events/mod.rs at 704 LOC is the big one). Evaluate dead event code.
- **workflow** — 2 beads. Split 3 domain files. Small handler surface offsets split work.
- **integrations** — 3 beads. Split 2 files. QuickBooks OAuth + CDC workers + webhook verification. Highest-risk medium module.

### Wave D — Heavy Financial Core (2 modules, 5 beads)

GL and Treasury. Core financial modules with complex consumer topologies and decimal precision requirements.

- **treasury** — 2 beads. Split 3 files. 2 consumers. Preserve rust_decimal arithmetic (no f64 regression).
- **gl** — 3 beads. Split 7 files. 11 consumers (most of any module). CurrencyConfigRegistry. Revenue recognition + accruals engines. DLQ validation gate. 25K+ LOC.

### Wave E — AP (1 module, 3 beads)

Accounts Payable has the most files needing splits (9) and complex domain logic (matching engine, payment runs).

- **ap** — 3 beads. Split 9 files. 2 consumers. Tax-core dependency. 2-way/3-way matching engine. 16K+ LOC.

### Wave F — AR (1 module, 4 beads) — LAST

Accounts Receivable is the most complex module in the platform: 95 endpoints, 27K+ LOC, Tilled payment integration with HMAC webhook verification, 8 files over 500 LOC (including webhooks.rs at 1,256 LOC), and 64 patch versions of iteration.

- **ar** — 4 beads. Split 8 files. 1 consumer. Tilled HMAC-SHA256 webhook verification with replay window guard. Party Master integration. 6+ webhook event types with dedicated processors. External payment contract must not be broken. 62 handler functions.

---

## Totals

| Wave | Modules | Beads | File Splits | Key Risks |
|------|---------|-------|-------------|-----------|
| Wave A (Quick Wins) | 6 | 6 | 0 | 3 missing migrations |
| Wave B (Medium Core) | 8 | 16 | 6 | 3 consumers, panic fix, dead code |
| Wave C (Medium Complex) | 4 | 9 | 9 | OAuth workers, 704-LOC split |
| Wave D (Heavy Financial) | 2 | 5 | 10 | 11 GL consumers, decimal precision |
| Wave E (AP) | 1 | 3 | 9 | 9 file splits, matching engine |
| Wave F (AR) | 1 | 4 | 8 | Tilled webhooks, 1256-LOC file |
| **TOTAL** | **22** | **43** | **42** | |

**Total beads:** 43 across 22 modules (vs. 3 modules / ~12 beads in Wave 1).

**Total file splits:** 42 files over 500 LOC need splitting as prerequisites.

**Modules missing migrations:** 3 (consolidation, quality-inspection, workforce-competence).

**Modules with dead event code:** 4 (subscriptions, ttp, timekeeping, production).

**Estimated bead order:** A1–A6, then B1–B8 (parallel within wave), then C1–C4, then D1–D2, then E (AP), then F (AR last).
