# Separation of Concerns Audit — 7D Solutions Platform

**Date:** 2026-03-31
**Scope:** All 22+ modules and 14 platform crates
**Method:** 6 parallel audit agents → 6 parallel verification agents (every finding cross-checked against source)

---

## Executive Summary

The platform has **pockets of excellence** (GL, shipping-receiving, integrations, workflow, reporting) alongside **significant SoC violations** concentrated in the financial and commerce modules. The core pattern across the codebase is: handlers that should be thin dispatch layers instead contain inline SQL, business rules, external API calls, and event publishing.

**By the numbers:**

| Severity | Count | Description |
|----------|-------|-------------|
| Critical | 5 | Handler is a god-function with DB + business logic + external APIs |
| Major | 14 | Meaningful concern mixing that impacts testability and maintainability |
| Minor | 6 | Architectural hygiene improvements |
| Refuted | 5 | Original audit claims not confirmed by verification |

---

## Module Grades

| Module | Grade | Key Issue |
|--------|-------|-----------|
| **gl** | A | Exemplary repo→service→handler. Gold standard. |
| **shipping-receiving** | A | Service layer with proper error conversions. |
| **integrations** | A | Guard→Mutation→Outbox pattern. |
| **workflow** | A | Textbook idempotency as decorator pattern. |
| **reporting** | A+ | Pure computation, trait-based plugin architecture. |
| **customer-portal** | A | Thin handlers, no inline SQL. |
| **consolidation** | A- | Clean delegation to domain engine. |
| **fixed-assets** | A- | Has CategoryRepo/AssetRepo pattern. |
| **production** | B+ | SQL in repo impl blocks (correct placement). |
| **treasury** | B | Handlers delegate, but services query DB directly (no repo layer). |
| **notifications** | B- | Event publishing embedded in handlers. |
| **timekeeping** | B- | DB queries in guard layer. |
| **workforce-competence** | B | Idempotency in service but well-extracted helper. |
| **party** | B | Contact merge logic mixed with SQL; some helper duplication. |
| **bom** | B- | In-memory pagination; correlation IDs generated on-the-fly. |
| **inventory** | B | Config leakage in DB resolver; idempotency duplicated across 8+ services. |
| **numbering** | C+ | Transaction orchestration in handler. |
| **ttp** | C+ | Env vars resolved at request time in handler. |
| **pdf-editor** | C+ | Event publishing in repository layer. |
| **ap** | C | No repo abstraction; match engine is 718-line god-function. |
| **payments** | D | 7 SQL locations in handlers; API client in types module; no domain errors. |
| **subscriptions** | D | bill_run handler is pure god-function: DB + external API + events. |
| **ar** | D | 9 of 23 handler files have inline SQL. |

---

## Verified Critical Findings

### CRIT-1: AR — Inline SQL in 9 handler files

**Status: CONFIRMED**

Nine handler files in `modules/ar/src/http/` contain direct `sqlx::query` calls inside handler functions: charges.rs, customers.rs, disputes.rs, events.rs, invoices.rs, payment_methods.rs, refunds.rs, subscriptions.rs, usage.rs.

`invoices.rs` (640 lines) is the worst offender — a single handler contains customer validation, subscription validation, party integration via external API, transaction management, event envelope construction, and outbox operations. The `finalize_invoice()` function (lines 461-639) adds payment method fetching, dual event emissions, and GL posting with complex line item construction.

14 other handler files in AR properly delegate (aging, allocation, credit_notes, dunning, reconciliation, tax, write_offs, webhooks). This split suggests the module was partially refactored — the newer domain features got services, while the original CRUD handlers didn't.

**Refactoring direction:** Extract InvoiceService, CustomerService, ChargeService etc. following GL's repo→service→handler pattern.

### CRIT-2: Subscriptions — bill_run.rs is a god-function

**Status: CONFIRMED (all 6 aspects verified)**

`modules/subscriptions/src/http/bill_run.rs` handler contains:
- Lines 79-90: Idempotency check (direct `sqlx::query_as`)
- Lines 110-121: Bill run creation (direct SQL INSERT)
- Lines 137-151: Subscription fetching (query with billing-due filter logic)
- Lines 157-158: `std::env::var("AR_BASE_URL")` read at request time
- Lines 160-274: Invoice creation loop with HTTP calls to AR API
- Lines 247-259: Subscription next_bill_date UPDATE
- Lines 299-324: Event emission
- Lines 346-367: `calculate_next_bill_date()` business rule in handler file

**Refactoring direction:** Extract BillRunService (orchestration), SubscriptionRepository (data access), ARApiClient (external integration). Handler becomes: extract tenant → call service.execute() → map error.

### CRIT-3: Payments — Checkout sessions scatter concerns everywhere

**Status: CONFIRMED (7 SQL locations across 5 handlers)**

`modules/payments/src/http/checkout_sessions/handlers.rs`:
- 7 direct `sqlx::query` calls across create, get, present, poll, and webhook handlers
- External Tilled API calls inline in handlers (lines 119-127, 242-258)
- State machine logic (status transitions) enforced inline, not in dedicated service

`session_logic.rs` mixes Tilled HTTP client calls (lines 82-160) with type definitions — API integration code in what should be a contracts/types file.

No domain error type exists. Handlers use `anyhow::Result` and manually construct `ApiError` responses.

**Refactoring direction:** Extract CheckoutSessionService, SessionStateMachine, TilledClient, SessionRepository. Add `CheckoutSessionError` with `From<...> for ApiError`.

### CRIT-4: AP — Match engine is a 718-line god-function

**Status: CONFIRMED**

`modules/ap/src/domain/match/engine.rs` (718 lines) contains a single `run_match()` function handling guard phase (load bill/PO/receipt aggregates, validate status), mutation phase (insert match records with ON CONFLICT, update bill status), and outbox phase (enqueue events). The entire AP module lacks a repository abstraction — services query DB directly via `sqlx::query`.

**Refactoring direction:** Split into MatchGuards (validation), MatchRepository (data access), MatchService (orchestration). Extract bill/vendor/PO repos from service layer.

### CRIT-5: TTP — Env vars resolved at request time

**Status: CONFIRMED**

`modules/ttp/src/http/billing.rs` lines 100-103: `TENANT_REGISTRY_URL` and `AR_BASE_URL` read via `std::env::var()` inside the `create_billing_run` handler on every request.

**Refactoring direction:** Move to Config struct loaded at startup; inject via AppState.

---

## Verified Major Findings

### MAJ-1: Inventory — Config leakage in DB resolver
**CONFIRMED.** `modules/inventory/src/db/resolver.rs` lines 17-25: `DB_MAX_CONNECTIONS` and `DB_ACQUIRE_TIMEOUT_SECS` read via `env::var()` at pool creation rather than through centralized Config.

### MAJ-2: Inventory — Idempotency duplicated across 8+ services
**CONFIRMED.** Idempotency key checking (find, hash, store pattern) is independently implemented in 8+ service files under `modules/inventory/src/domain/`. Not centralized into middleware or decorator.

### MAJ-3: Party — Contact merge logic mixed with SQL
**CONFIRMED.** `modules/party/src/domain/contact_service/mutation.rs` lines 128-158: 5+ nullable field merge logic interleaved with SQL execution in the same function.

### MAJ-4: BOM — In-memory pagination
**CONFIRMED.** `modules/bom/src/http/bom_routes.rs` lines 74-80 define `paginate()` that takes a full `Vec<T>`, computes `items.len()` as total, then does `.skip().take()`. Used in `list_boms()` (line 106), `list_revisions()` (line 237), `get_lines()` (line 386). All records loaded via `.fetch_all()` before slicing.

### MAJ-5: BOM — Correlation IDs generated on-the-fly
**CONFIRMED.** `bom_routes.rs` lines 70-72 and `eco_routes.rs` lines 19-21: new correlation IDs generated per-request instead of extracting from incoming request headers/tracing context.

### MAJ-6: BOM — Error mapping generates new request IDs
**CONFIRMED.** `bom_routes.rs` lines 66-68: Error responses create new UUIDs instead of using TracingContext.

### MAJ-7: Timekeeping — DB queries in guard layer
**CONFIRMED.** `modules/timekeeping/src/domain/entries/guards.rs`: `check_period_lock()` (lines 80-113) and `check_overlap()` (lines 121-156) execute `sqlx::query_as`. Also `clock/guards.rs`: `check_no_open_session()` (lines 14-37) and `require_open_session()` (lines 41-61) have SQL. Guards should validate, not query.

### MAJ-8: Notifications — Event publishing in handlers
**CONFIRMED.** `modules/notifications/src/http/sends.rs` lines 132-186 and `templates.rs` lines 55-83: `enqueue_event()` calls directly in HTTP handler functions.

### MAJ-9: PDF-Editor — Event publishing in repository layer
**CONFIRMED.** `modules/pdf-editor/src/domain/submissions/repo.rs` line 7 imports, lines 107-167: `SubmissionRepo::submit()` creates and enqueues events, mixing persistence with event publishing.

### MAJ-10: Numbering — Transaction orchestration in handler
**CONFIRMED.** `modules/numbering/src/http/allocate.rs`: idempotency check (lines 94-105), transaction begin (lines 130-134), allocation call (137-142), mutations (155-191), event enqueue (203-215), commit (218) — all in the handler. Helper `allocate_next_value()` exists but is co-located in the handler file.

### MAJ-11: Treasury — No repo abstraction
**CONFIRMED.** Handlers delegate to services, but services query DB directly. Example: `accounts/service.rs` has inline `sqlx::query_as` at lines 33, 57, 72. Same pattern as AP.

### MAJ-12: Security crate depends on event-bus
**CONFIRMED.** `platform/security/Cargo.toml` line 25. Used for audit logging in `tracing.rs`. Creates coupling from security cross-cutting concern to messaging infrastructure.

### MAJ-13: Security crate exposes Axum without feature gating
**CONFIRMED.** Axum is a mandatory dependency (Cargo.toml line 22). Public API re-exports ClaimsLayer, ClaimsMiddleware, RequirePermissionsLayer — all Axum-dependent. Cannot use JwtVerifier without pulling in Axum.

### MAJ-14: Platform crate misplacements — doc-mgmt and control-plane
**CONFIRMED.** Both have `[[bin]]` targets, `main.rs` with HTTP routes, DB pools, and service startup. These are services in `platform/` that should be in `modules/`.

---

## Verified Minor Findings

### MIN-1: Security re-exports 37+ items from lib.rs
**CONFIRMED.** Lines 29-71 of `platform/security/src/lib.rs`.

### MIN-2: Projections re-exports 28 items from lib.rs
**CONFIRMED.** Lines 55-69 of `platform/projections/src/lib.rs`. Leaks `sqlx::PgConnection` in public API.

### MIN-3: Party — extract_tenant/with_request_id duplication
**PARTIALLY CONFIRMED.** `extract_tenant` and `with_request_id` are centralized, but `correlation_from_headers` is duplicated across HTTP modules.

### MIN-4: Inconsistent pool initialization across platform services
**PARTIALLY CONFIRMED.** 3 approaches found: inline `PgPoolOptions` (control-plane), `db::create_pool()` (doc-mgmt, identity-auth), `resolve_pool` (modules). Inconsistency exists but scope is narrower than claimed.

### MIN-5: No centralized outbox relay infrastructure
**PARTIALLY CONFIRMED.** `event-bus/src/outbox.rs` provides shared validation (`validate_and_serialize_envelope()`), but each module implements its own relay polling loop (doc-mgmt/outbox_relay.rs, audit/outbox_bridge.rs, etc.).

### MIN-6: Workforce-competence — Idempotency in service
**PARTIALLY CONFIRMED but acceptable.** Idempotency logic is in service/core.rs but cleanly extracted into `find_idempotency_key()` helper (lines 368-384). Pattern is reasonable: guard→idempotency→mutation→outbox in transaction.

---

## Refuted Findings

| Original Claim | Verification Result |
|----------------|-------------------|
| Maintenance: 8 handler files have unreadable single-line formatting | **REFUTED** — All 8 files are properly formatted Rust code |
| Production: Raw SQL in domain layer (not repos) | **REFUTED** — SQL is correctly in OperationRepo, WorkOrderRepo, DowntimeRepo impl blocks |
| Inventory: Status code selection pattern-matched in every handler | **REFUTED** — Correct idempotent create pattern (201 vs 200) |
| Inventory: Event envelope interleaved with ledger inserts | **REFUTED** — Correct separation within transaction boundary |
| AppState reimplemented 12+ times | **REFUTED** — 10 State structs found, many narrowly scoped (MetricsState, JwksState etc.) |

---

## Exemplar Modules (Use as Templates)

### GL — Repository Pattern
17 dedicated repo files (account_repo, journal_repo, balance_repo, period_repo, etc.). Handlers call repos. Services call repos. No inline SQL in handlers. Clean error mapping.

### Shipping-Receiving — Service Layer
`ShipmentService` owns state transitions. `ShipmentRepository` owns data access. `error_conversions.rs` maps domain→API errors via `From` trait. Handlers only extract, delegate, respond.

### Integrations — Guard→Mutation→Outbox
Explicit three-phase pattern with section comments in service.rs. Guards in `guards.rs` (stateless validation). Mutations within transaction. Outbox enqueue in same transaction.

### Workflow — Idempotency as Decorator
Idempotency implemented as early-return check at framework level (lines 170-179, 293-302). Business logic is never contaminated. Response stored for replay. All mutations + outbox in committed transaction.

### Reporting — Plugin Architecture
`StreamHandler` trait (ingest/mod.rs lines 51-66) defines clean plugin interface. Pure computation functions (KPIs, balance sheets, forecasts) are data-in/data-out. Handlers implement trait to adapt events to cache operations.

---

## Recommended Refactoring Priority

### Tier 1 — Critical SoC Violations (High Impact)

| Module | Work | Approach |
|--------|------|----------|
| **ar** | Extract services for 9 inline-SQL handlers | Follow GL's repo→service pattern. Start with invoices.rs (largest). |
| **subscriptions** | Extract BillRunService | Move orchestration, AR API client, and billing rules out of handler. |
| **payments** | Extract CheckoutSessionService + TilledClient | Add SessionRepository, SessionStateMachine, domain error type. |
| **ap** | Split match engine + add repo layer | Break 718-line god-function into guards/repo/service. Add repos for bills, vendors, POs. |

### Tier 2 — Major Violations (Medium Impact)

| Module | Work |
|--------|------|
| **bom** | Replace in-memory pagination with SQL LIMIT/OFFSET. Fix correlation ID and request ID handling. |
| **timekeeping** | Move DB queries out of guards into service layer. |
| **notifications** | Move event publishing from handlers to service layer. |
| **pdf-editor** | Move event publishing from repo to service layer. |
| **numbering** | Extract AllocationService from handler. |
| **treasury** | Add repository layer between services and DB. |
| **inventory** | Centralize idempotency into middleware/decorator. Move DB config to Config struct. |
| **ttp** | Move env var reads to Config struct loaded at startup. |

### Tier 3 — Platform Architecture

| Item | Work |
|------|------|
| **Move doc-mgmt, control-plane to modules/** | These are services, not platform libraries. |
| **Feature-gate Axum in security crate** | Split JwtVerifier/RBAC (framework-agnostic) from ClaimsLayer (Axum-specific). |
| **Remove security→event-bus dependency** | Audit logging should use a trait, not depend on messaging infra. |
| **Extract shared outbox relay** | Centralize the relay polling loop used by 3+ modules. |
| **Reduce re-export surface** | security (37→~10 core items), projections (28→~8 core items). |

---

## Cross-Cutting Patterns to Extract

1. **Idempotency middleware/decorator** — Currently reimplemented in 8+ inventory services, numbering, workforce-competence, workflow. Workflow's pattern is the cleanest template.

2. **Middleware stack builder** — Every module's main.rs stacks timeout, rate_limit, claims, cors identically. Extract into `platform/security::build_middleware_stack()`.

3. **DB pool resolver** — 3 different approaches exist. Standardize on one.

4. **Event consumer startup** — Each module spawns consumers identically. Extract shared helper.
