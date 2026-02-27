# bd-15cn Platform Survey (2026-02-27)

## Scope and Method

Read-only survey of the `7D-Solutions Platform` repository to map:
- modules and responsibilities
- architectural patterns and dependency boundaries
- test coverage surface
- technical debt and inconsistencies

Evidence sources used:
- workspace manifest (`Cargo.toml`)
- crate manifests under `platform/`, `modules/`, and `products/trashtech/`
- component READMEs
- source tree layout (`src/` and `tests/`)
- TODO/FIXME/HACK markers

## Repository Topology

The repo currently contains a large Rust monorepo plus two Next.js apps:

- Tier 1 platform crates: `platform/{event-bus,identity-auth,audit,tenant-registry,control-plane,projections,security,tax-core,health}`
- Tier 2 business modules: `modules/{ap,ar,consolidation,fixed-assets,gl,integrations,inventory,maintenance,notifications,party,payments,pdf-editor,reporting,shipping-receiving,subscriptions,timekeeping,treasury,ttp}`
- Tier 3 product crates: `products/trashtech/{tt-core,tt-integrations}`
- Frontend apps: `apps/{tenant-control-plane-ui,trashtech-pro}`
- Contract surface: `contracts/` (OpenAPI + event JSON schemas)
- Cross-cutting test package: `e2e-tests`
- Operational/dev tooling: `tools/*`, `scripts/*`, docker compose overlays

Workspace source count (platform+modules+trashtech products+contract tests+e2e tests):
- `*.rs|*.ts|*.tsx` files: ~1514
- Files under `*/tests/*`: ~342

## Component Responsibilities

### Tier 1: Platform

- `event-bus`: shared envelope + publish/subscribe abstraction (in-memory and NATS), retry/outbox primitives.
- `identity-auth`: authentication service (JWT, password/refresh flows, auth routes, metrics, middleware).
- `security`: authz/rbac middleware, service auth, redaction, webhook verification.
- `tenant-registry`: tenant lifecycle/plan/summary/CRUD and related APIs.
- `control-plane`: tenant/control orchestration endpoints and integration with tenant registry + AR clients.
- `projections`: projection cursors, digest/rebuild/validation, fallback and circuit behavior.
- `audit`: audit actor/diff/policy/schema/writer stack and outbox bridge.
- `tax-core`: common tax models/provider contracts.
- `health`: common health/readiness primitives.

### Tier 2: Modules

- `ap`: vendor obligations, bills, payment runs, PO/bill flows, tax reports.
- `ar`: receivables, invoices, payments/refunds/disputes, dunning, reconciliation, tax flows.
- `gl`: journal engine, statements/trial balance, close/reopen flows, revrec, accruals, FX.
- `subscriptions`: subscription lifecycle, cycle gating, publishing/outbox.
- `payments`: payment execution + webhook processing + reconciliation.
- `notifications`: notifications delivery, scheduled dispatcher, DLQ and consumer tasks.
- `inventory`: stock movements, FIFO valuation, reservations, cycle counts, low-stock events.
- `party`: party/contact/address master and outbox events.
- `integrations`: webhook ingress, connector configs, external reference mapping.
- `maintenance`: assets/meters/work orders/plans with event subjects.
- `shipping-receiving`: inbound/outbound shipment lifecycle, event-driven inventory coupling.
- `timekeeping`: entries, approvals, allocations, rates, billing/export flows.
- `treasury`: bank/card reconciliation and reporting imports.
- `fixed-assets`: asset lifecycle, depreciation runs, disposals, AP capitalization integration.
- `consolidation`: multi-entity consolidation and elimination/FX policies.
- `reporting`: read-optimized statement/aging/KPI/forecast caches + ingestion.
- `pdf-editor`: stateless PDF processing + template/field/submission APIs.
- `ttp`: tenant tenancy/pricing/metering and billing-run orchestration.

### Tier 3: Product Layer

- `products/trashtech/tt-core`: product-specific core crate.
- `products/trashtech/tt-integrations`: external integration helpers for TrashTech.

### Frontend Layer

- `apps/tenant-control-plane-ui`: Next.js control plane frontend (React Query, RHF/Zod, Playwright e2e suite).
- `apps/trashtech-pro`: lightweight Next.js app scaffold.

## Key Architectural Patterns Observed

1. **Strong shared platform dependency shape in modules**
- Most modules depend on `platform/security`, `platform/health`, `platform/event-bus`, and often `platform/projections`.

2. **Outbox + envelope/event-driven integration pattern**
- Repeated `events/`, `outbox/`, consumer, and DLQ structures appear across modules.
- Contracts directory includes explicit event JSON schema set.

3. **Consistent service skeleton**
- Most modules expose:
  - `config.rs`
  - `db/*`
  - `domain/*` and/or `http|routes/*`
  - `metrics.rs`
  - `main.rs` + `lib.rs`

4. **Layering mostly enforced, with HTTP integration instead of source-level coupling**
- Integration points usually via events or HTTP clients (e.g., TTP clients to AR/tenant-registry).

5. **Dual runtime/testing strategy**
- In-memory paths for local/test plus NATS-backed infrastructure paths for realistic e2e.

## Dependencies and Boundary Observations

### Positive boundary alignment

- Modules generally avoid direct module-to-module Cargo deps.
- Cross-module behavior is mostly events/contracts/HTTP.

### Notable exception

- `modules/gl/Cargo.toml` has a **dev-dependency on `modules/ap`**:
  - `ap = { path = "../ap" }`
- This is test-scope only, but it is still a direct module import that weakens the “no cross-module imports” rule.

## Test Coverage Surface (Structural)

Rust crate-level file counts (source vs files in `tests/`):

- Platform:
  - audit `rs=12 tests=5`
  - control-plane `rs=14 tests=2`
  - event-bus `rs=14 tests=4`
  - health `rs=2 tests=1`
  - identity-auth `rs=42 tests=10`
  - projections `rs=13 tests=2`
  - security `rs=15 tests=3`
  - tax-core `rs=11 tests=4`
  - tenant-registry `rs=13 tests=3`

- Modules:
  - higher test density: `gl (133/40)`, `ar (120/30)`, `inventory (90/17)`, `maintenance (42/11)`, `payments (37/10)`
  - lower relative density: `party (28/3)`, `treasury (47/3)`, `integrations (37/3)`, `subscriptions (25/4)`, `ttp (26/4)`

- Product crates:
  - `tt-core (6/0)`
  - `tt-integrations (6/0)`

- Integration/e2e:
  - `e2e-tests/tests` contains a broad cross-module matrix (138 test files listed), including tenant lifecycle, security/rbac, replay/certification, outbox atomicity, reporting, tax, inventory, treasury, and more.

- Frontend:
  - `apps/tenant-control-plane-ui/tests` contains Playwright suites.
  - `apps/trashtech-pro` currently appears scaffold-level with no explicit test suite in-tree.

## Technical Debt and Inconsistency Signals

1. **Stale backup artifacts committed in source trees**
- Examples:
  - `modules/ar/src/routes.rs.bak`
  - `modules/notifications/src/main.rs.bak{,2,3}`
  - `modules/payments/src/main.rs.bak{,2,3}`
  - `modules/payments/src/lifecycle.rs.bak2`
  - `modules/payments/tests/reconciliation_tests.rs.bak`
- Risk: accidental drift/confusion during maintenance and code search noise.

2. **Documented version drift between README and Cargo metadata**
- `platform/identity-auth` README says `v1.4` while Cargo is `1.3.6`.
- `modules/ar` README lists `1.0.5` while Cargo is `1.0.16`.
- `modules/payments` README lists `1.1.5` while Cargo is `1.1.11`.
- `modules/ttp` README lists `2.1.1` while Cargo is `2.1.5`.
- Risk: incorrect operational assumptions and release communication mismatch.

3. **Deferred external integration work concentrated in AR/payments paths**
- TODOs reference pending Tilled integration, event emissions, and middleware extraction.
- Risk: behavior gaps between local abstractions and real provider semantics.

4. **Rate limit hardening partially deferred in identity-auth**
- TODO comments indicate previously disabled/rework-needed rate limiting paths.
- Risk: production hardening gap under abusive traffic.

5. **Readme/architecture narrative lag**
- Root README still contains planned/baseline language that no longer reflects current repository fullness.
- Risk: onboarding friction and misunderstanding of what is production-grade vs planned.

6. **Uneven testing confidence by area**
- Core accounting/event paths are heavily tested.
- Some modules/product crates have noticeably thinner direct test coverage, relying more on system e2e.

## Overall Assessment

- The platform has evolved into a substantial, coherent Rust-first multi-module system with strong event-driven patterns and extensive end-to-end verification.
- Architecture is mostly consistent with tiered boundaries, with one clear test-time cross-module dependency exception.
- Primary risks are **operational/documentation drift** and **cleanup debt** (backup files + pending TODOs), not absence of foundational architecture.

## Suggested Follow-up Beads (Derived from Survey)

1. Remove `.bak*` artifacts from module source trees and enforce guardrail against reintroduction.
2. Reconcile module README version/status metadata with Cargo/package versions.
3. Resolve known TODO hotspots for provider integrations and event emission in AR/payments.
4. Review/restore robust rate limiting path in `identity-auth`.
5. Evaluate whether `gl` dev-dependency on `ap` should be replaced by fixture/contracts-based test inputs.
6. Add explicit tests for product crates (`tt-core`, `tt-integrations`) if they are expected to carry business logic.
