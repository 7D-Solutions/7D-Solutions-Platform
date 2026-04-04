# Consolidated audit report — 7D Solutions Platform

**Report date:** 2026-04-03  
**Scope:** This repository only (platform + modules + platform crates + e2e/tools as cited).  
**Method:** Multi-pass audit (surface metrics → deep file verification → integration notes → re-verification → doc alignment where cross-repo). **No automated test suite or UBS run** as part of this report.  
**Audience:** Engineering leads and agents picking up remediation beads.

---

## 1. Executive summary

The platform shows **strong conventions** (thin AR HTTP without raw SQL in `modules/ar/src/http`, heavy use of `domain/` + repos elsewhere), but **several HTTP modules embed substantial SQL and transaction logic**, especially **numbering**, **shipping-receiving shipments**, **customer-portal**, **GL** checklist/period routes, **notifications** DLQ, and **smoke-test**. **Compile-time `sqlx::query!` usage is minimal** (single-digit occurrences repo-wide). **Mock/stub and TODO debt** remains in **payments**, **notifications**, **AR lifecycle events**, **identity-auth rate limiting**, **tools**, and **reporting cashflow v1**, as verified by grep and spot reads.

---

## 2. Methodology (passes)

| Pass | Focus (this repo) |
|------|-------------------|
| **1 — Surface** | `rg` inventories: `sqlx::query!`, dynamic `sqlx::query`/`query_as`/`query_scalar`, SQL inside `modules/**/http/**/*.rs`. |
| **2 — Deep** | Read `numbering/src/http/allocate.rs`, `shipping-receiving/.../shipments/handlers.rs`; confirmed `ar/src/http` has no `sqlx::query*` matches. |
| **3 — Integration** | Vertical consumers (Fireproof, TrashTech) depend on stable HTTP + DB contracts; SQL in `http/` increases coupling to schema and e2e breakage risk. |
| **4 — Verify** | Re-ran `http/` SQL grep; counts stable vs Pass 1. |
| **5 — Doc alignment** | Compared `docs/PLATFORM-SERVICE-CATALOG.md` to `docker-compose.*` for **pdf-editor (8102)** and **doc-mgmt** service gaps (see §6). |

---

## 3. Scale metrics (approximate)

| Metric | Value (order of magnitude) |
|--------|----------------------------|
| Rust `*.rs` files (repo) | ~2,238 |
| `sqlx::query!` occurrences | **5** |
| Dynamic `sqlx::query` / `query_as` / `query_scalar` line matches (repo-wide, incl. tests) | ~6,093 |

---

## 4. Separation of concerns — SQL in `modules/**/http/**/*.rs`

**Finding:** Any `sqlx::query*` under `http/` mixes **transport** with **persistence** unless limited to trivial health checks.

### 4.1 Files with dynamic SQL under `http/` (verified via grep)

Non-exhaustive grouping:

| Category | Examples |
|----------|----------|
| **Heavy / business** | `numbering/src/http/allocate.rs`, `numbering/src/http/confirm.rs`, `shipping-receiving/src/http/shipments/handlers.rs`, `customer-portal/src/http/{auth,status,admin,docs}.rs`, `gl/src/http/{period_close,close_checklist}.rs`, `notifications/src/http/dlq.rs`, `ap/src/http/payment_runs.rs`, `ttp/src/http/service_agreements.rs`, `payments/src/http/checkout_sessions/repo.rs` (repo nested under `http/`), `smoke-test/src/http/items.rs` |
| **Trivial** | Many `**/http/health.rs` and some `**/http/mod.rs` — `SELECT 1` style probes |

### 4.2 AR module HTTP layer

**Verified:** `modules/ar/src/http/**/*.rs` — **no** `sqlx::query` / `query_as` / `query_scalar` matches on audit grep. AR HTTP appears to **delegate** persistence to domain/db layers for those patterns.

### 4.3 Priority remediation targets (SoC)

1. `modules/numbering/src/http/allocate.rs` (~430 lines) — transactions + idempotency SQL in HTTP module.  
2. `modules/shipping-receiving/src/http/shipments/handlers.rs` (~694 lines) — e.g. INSERT via `sqlx::query_as` inside handler despite domain/repo types.  
3. `modules/customer-portal/src/http/*` — large SQL surface in portal HTTP (policy choice vs technical debt).  

---

## 5. Mock code, stubs, and TODO debt (verified)

**Method:** `rg` + spot reads; **re-verified 2026-04-03** where noted.

### 5.1 Production-relevant items

| Area | Location / pattern | Status |
|------|-------------------|--------|
| Payments read fallback | `modules/payments/src/http/payments.rs` — `query_write_service` delay + mock `PaymentResponse` | Open |
| Notifications | `modules/notifications/src/handlers.rs` — mock payment-succeeded path | Open |
| Scheduled sender | `modules/notifications/src/scheduled/sender.rs` — `LoggingSender` | Open (documented stub) |
| Checkout without Tilled | `modules/payments/src/http/checkout_sessions/handlers.rs` — `mock_pi_*` | Open (config-gated) |
| Webhook / lifecycle events | `modules/payments/src/webhook_handler.rs` TODO; `modules/payments/src/lifecycle.rs` doc | Open |
| AR invoice lifecycle | `modules/ar/src/lifecycle.rs` — emit TODOs | Open |
| Stripe webhooks | `modules/payments/src/webhook_signature.rs` — unsupported | Open |
| Usage API | `modules/ar/src/http/usage.rs` — `subscription_id` placeholder mapping | Review |
| Envelope schema validation | `modules/{ar,notifications,payments,subscriptions}/**/envelope_validation.rs` | Open |
| Compliance CLI | `tools/compliance-export/src/main.rs` — `--from`/`--to` ignored | Open |
| Simulation tool | `tools/simulation/src/main.rs` — tenant/subscription TODOs | Open |
| Identity rate limit | `platform/identity-auth/src/main.rs` — tower_governor TODOs | Open |
| Reporting cashflow v1 | `modules/reporting/src/domain/statements/cashflow.rs` — investing/financing stubs | Documented limitation |

### 5.2 Intentional / not debt

- `modules/ar/src/tax/providers.rs` — `ZeroTaxProvider` for tests/dev.  
- `modules/inventory/src/http/*` — `placeholder_*_compiles` tests.  
- Doc examples `/// let pool = todo!()` in AR — documentation only, not live `todo!()`.

---

## 6. Documentation and compose alignment (this repo)

- **`docs/PLATFORM-SERVICE-CATALOG.md`** — marked generated; treat as **catalog** of module ports/versions.  
- **`docker-compose.services.yml`** — contains `DOC_MGMT_BASE_URL: http://7d-doc-mgmt:8095` with comment **TODO: 7d-doc-mgmt service does not exist yet** (snippet audited). **pdf-editor** is published at **8102** (`7d-pdf-editor`).  
- **Drift risk for consumers:** Fireproof’s templates/docs may point `DOC_MGMT_BASE_URL` at **`7d-doc-mgmt:8102`** — see **Fireproof-ERP** report `docs/audits/consolidated-audit-report-2026-04-03.md` §6.

---

## 7. Out of scope (this report)

- Full `cargo test` / `./scripts/cargo-slot.sh test` execution.  
- UBS or other static analyzers.  
- Line-by-line review of every module.  
- Secrets / credential audit beyond placeholder patterns in docs.  
- Frontend (this repo is backend-first per README).

---

## 8. Suggested next actions (no work done here)

1. Bead epic: **HTTP layer SQL extraction** (numbering, shipping-receiving, customer-portal as phases).  
2. Bead track: **payments + notifications + AR** event/outbox completion.  
3. Bead: **doc-mgmt HTTP service** definition and **single canonical `DOC_MGMT_BASE_URL`** story (compose + catalog + vertical templates).

---

## 9. References (in-repo)

- `docs/PLATFORM-SERVICE-CATALOG.md`  
- `docs/consumer-guide/PLATFORM-CONSUMER-GUIDE.md`  
- `docker-compose.services.yml`, `docker-compose.modules.yml`  
- `AGENTS.md` / `CLAUDE.md` — beads and `cargo-slot` for any remediation work  

---

*End of report — 7D Solutions Platform*
