# Plug-and-Play Wave 1: Full Audit Report

> **Date:** 2026-03-31
> **Scope:** All 3 treated modules (Inventory, Party, BOM) + 5 platform crates
> **Method:** Exhaustive code-level review + agent verification (4 independent review agents)
> **Build/Test Status:** Cannot run in sandbox (Rust toolchain not available) — see Section 5 for commands to run on host

---

## 1. Executive Summary

| Module | Version | Overall | Critical Issues |
|--------|---------|---------|-----------------|
| **Inventory** | 2.4.5 | **PASS with issues** | 10 domain files >500 LOC not in allowlist; main.rs at 724 LOC |
| **Party** | 2.3.2 | **PASS with issues** | /api/ready doesn't check NATS (DB-only); no ConfigValidator crate |
| **BOM** | 2.2.1 | **PASS with issues** | /api/ready doesn't check NATS (DB-only); no ConfigValidator crate |
| **platform-http-contracts** | 0.1.0 | **PASS** | Clean |
| **config-validator** | 0.1.0 | **PASS** | Clean |
| **platform-contracts** | 1.0.0 | **PASS** | Clean |
| **health** | 1.0.0 | **PASS** | Clean |
| **security** | 1.0.3 | **PASS** | Clean |

---

## 2. Detailed Module Audits

### 2.1 Inventory (v2.4.5)

| # | Check | Result | Evidence |
|---|-------|--------|----------|
| 1 | PaginatedResponse on all list endpoints | **PASS** | 9 list endpoints verified: list_items, list_uoms, list_conversions, get_serials_for_item, get_lots_for_item, get_list_labels, list_reorder_policies, list_valuation_snapshots, get_list_revisions. All use `PaginatedResponse::new(items, page, page_size, total)`. 29 PaginatedResponse references across HTTP layer. |
| 2 | ApiError on all error paths | **PASS** | All handlers use `ApiError` with `with_request_id()`. 134 with_request_id calls across HTTP layer. Centralized error_conversions.rs maps 20+ domain error types to ApiError. |
| 3 | OpenAPI via utoipa | **PASS** | 52 utoipa annotations across HTTP handlers. SecurityAddon with Bearer JWT (main.rs:247). `/api/openapi.json` served (main.rs:545). ApiDoc lists all paths. |
| 4 | ConfigValidator | **PARTIAL PASS** | Does NOT use `config_validator::ConfigValidator` crate. Uses manual `Vec<String>` error collection pattern (config.rs:49). Achieves same behavior (collects all errors, returns joined). Acceptable but inconsistent with Party which uses the crate. |
| 5 | Auto-migrations | **PASS** | `sqlx::migrate!("./db/migrations").run(&pool)` at main.rs:292. |
| 6 | Event bus + graceful degradation | **PASS** | BusType enum (Nats/InMemory). BusHealth tracked in AppState. Event bus supervisor with NATS retry loop. `/api/ready` checks both DB and NATS. Reports Degraded when NATS down + DB up. 2 consumers (component_issue, fg_receipt) wired. |
| 7 | Health endpoints | **PASS** | `/healthz` (main.rs:541), `/api/health`, `/api/ready`, `/api/version` all present. |
| 8 | Files under 500 LOC | **FAIL** | 12 files exceed 500 LOC. 4 are in `.file-size-allowlist` (receipt_service, reservation_service, issue_service, adjust_service). **10 are NOT allowlisted**: main.rs (724), error_conversions.rs (627), transfer_service.rs (624), cycle_count/approve_service.rs (601), labels.rs (586), valuation/run_service.rs (548), valuation/methods.rs (538), valuation/snapshot_service.rs (512), expiry.rs (503), status/transfer_service.rs (502). |
| 9 | REVISIONS.md complete | **PASS** | 12 entries from 1.0.0 through 2.4.5, all with required fields. |
| 10 | Cargo.toml matches REVISIONS.md | **PASS** | Both at 2.4.5. |

**Inventory Action Items:**
1. Add 10 unallowlisted files >500 LOC to `.file-size-allowlist` OR split them (preferred)
2. Consider migrating config.rs to use ConfigValidator crate for consistency with Party

---

### 2.2 Party (v2.3.2)

| # | Check | Result | Evidence |
|---|-------|--------|----------|
| 1 | PaginatedResponse on all list endpoints | **PASS** | list_parties and search_parties return `PaginatedResponse<Party>`. Sub-collection lists (contacts, addresses) use `DataResponse<T>` wrapper. 5 PaginatedResponse references. |
| 2 | ApiError on all error paths | **PASS** | All 19 handlers enrich errors with `with_request_id()`. 46 with_request_id calls. PartyError → ApiError conversion in error_conversions.rs. |
| 3 | OpenAPI via utoipa | **PASS** | 19 utoipa annotations covering all handlers. SecurityAddon with Bearer JWT. `/api/openapi.json` served (main.rs:149). All domain types have ToSchema. Query params have IntoParams. |
| 4 | ConfigValidator | **PASS** | Uses `config_validator::ConfigValidator` crate (config.rs:1,40). Calls `v.require()`, `v.optional()`, `v.require_when()`, `v.finish()`. Collects all errors. |
| 5 | Auto-migrations | **PASS** | `sqlx::migrate!("./db/migrations").run(&pool)` at main.rs:129. |
| 6 | Event bus + graceful degradation | **FAIL** | Has BusType config and event-bus dependency, but `/api/ready` (ops/ready.rs) only checks DB — does NOT check NATS health. No `nats_check()` call. No BusHealth in AppState. No Degraded status reporting. |
| 7 | Health endpoints | **PASS** | `/healthz` (http/mod.rs:77), `/api/health`, `/api/ready`, `/api/version` all present. |
| 8 | Files under 500 LOC | **PASS** | All files under 500 LOC. Largest: contact_role_service.rs at 357 LOC. |
| 9 | REVISIONS.md complete | **PASS** | 8 entries from 1.0.0 through 2.3.2, all with required fields. |
| 10 | Cargo.toml matches REVISIONS.md | **PASS** | Both at 2.3.2. |

**Party Action Items:**
1. **CRITICAL:** Add NATS health check to `/api/ready` — should report Degraded when NATS is down but DB is up (match Inventory's pattern)
2. Wire BusHealth into AppState

---

### 2.3 BOM (v2.2.1)

| # | Check | Result | Evidence |
|---|-------|--------|----------|
| 1 | PaginatedResponse on all list endpoints | **PASS** | 7 list endpoints return PaginatedResponse. Explosion and Where-Used endpoints correctly return flat arrays. 28 PaginatedResponse references. |
| 2 | ApiError on all error paths | **PASS** | Centralized `into_api_error()` in bom_routes.rs (lines 22-64) maps all error types. 9 error branches, all call `.with_request_id(request_id())`. |
| 3 | OpenAPI via utoipa | **PASS** | 25 utoipa annotations across all handlers. SecurityAddon with Bearer JWT. `/api/openapi.json` served (main.rs:255). |
| 4 | ConfigValidator | **PARTIAL PASS** | Does NOT use `config_validator::ConfigValidator` crate. Uses manual `Vec<String>` error collection pattern (config.rs:19). Achieves same behavior. |
| 5 | Auto-migrations | **PASS** | `sqlx::migrate!("./db/migrations").run(&pool)` at main.rs:155. |
| 6 | Event bus + graceful degradation | **FAIL** | Has EventEnvelope integration and 13 event types, but `/api/ready` (http/health.rs) only checks DB. No NATS health check, no BusHealth, no Degraded status. |
| 7 | Health endpoints | **PASS** | `/healthz`, `/api/health`, `/api/ready`, `/api/version` all present. SCHEMA_VERSION defined as "20260305000001". |
| 8 | Files under 500 LOC | **PASS** | All files under 500 LOC. Largest: bom_routes.rs at 445 LOC. |
| 9 | REVISIONS.md complete | **PASS** | 7 entries from 1.0.0 through 2.2.1, all with required fields. |
| 10 | Cargo.toml matches REVISIONS.md | **PASS** | Both at 2.2.1. |

**BOM Action Items:**
1. **CRITICAL:** Add NATS health check to `/api/ready` — same issue as Party
2. Consider migrating config.rs to use ConfigValidator crate

---

## 3. Platform Crate Audit

### 3.1 platform-http-contracts (v0.1.0) — PASS

- **PaginatedResponse<T>**: `new(items, page, page_size, total_items)` constructor. Fields: `data` (Vec<T>), `pagination` (PaginationMeta with page, page_size, total_items, total_pages). Correct total_pages calculation. ✓
- **ApiError**: Fields: `error`, `message`, `request_id` (Option), `details` (Option<Vec<FieldError>>), `status` (skip_serializing). Constructors: `not_found()`, `conflict()`, `internal()`, `bad_request()`, `unauthorized()`, `forbidden()`, `validation_error()`, `new()`. `with_request_id()` method. `IntoResponse` impl behind `axum` feature. ✓
- **FieldError**: `field` and `message` fields. ✓
- **Tests**: 8 tests covering serialization, status codes, edge cases. ✓
- **LOC**: error.rs (202), pagination.rs (103), lib.rs (20). All under 500. ✓

### 3.2 config-validator (v0.1.0) — PASS

- **ConfigValidator API**: `new()`, `require()`, `optional()`, `require_parse()`, `optional_parse()`, `require_when()`, `finish()`. All present and correct. ✓
- **Error collection**: Non-fail-fast. Accumulates in `Vec<ConfigError>`. ✓
- **Display**: Human-readable table formatting with borders and aligned columns. ✓
- **Tests**: 12 tests covering all methods, edge cases, multiple errors. ✓
- **LOC**: lib.rs (496). Under 500. ✓

### 3.3 platform-contracts (v1.0.0) — PASS

- Re-exports EventEnvelope, MerchantContext from event-bus. ✓
- Modules: consumer, event_naming, idempotency, mutation_classes, portal_identity. ✓
- Proven module at v1.0.0. ✓

### 3.4 health (v1.0.0) — PASS

- **ReadyResponse**: service_name, version, status (ReadyStatus), degraded (bool), checks (Vec<HealthCheck>), timestamp. ✓
- **ReadyStatus**: Ready, Degraded, Down. ✓
- **Builders**: `nats_check()`, `db_check_with_pool()`, `db_check()`, `build_ready_response()`. ✓
- **Helpers**: `healthz()` (liveness probe), `ready_response_to_axum()`. ✓
- **Tests**: 8 tests. ✓

### 3.5 security (v1.0.3) — PASS

- JwtVerifier, VerifiedClaims, RequirePermissionsLayer, optional_claims_mw, rate_limit_middleware, timeout_middleware, DEFAULT_BODY_LIMIT, TracingContext, ActorType — all exported. ✓
- Webhook verification: GenericHmacVerifier, StripeVerifier, IntuitVerifier. ✓
- Service auth: generate/verify service tokens. ✓
- Permission constants present. ✓
- 12 modules, all under 500 LOC. ✓

---

## 4. Cross-Module Consistency Checks

| Check | Inventory | Party | BOM | Consistent? |
|-------|-----------|-------|-----|-------------|
| ConfigValidator crate | No (manual) | **Yes** | No (manual) | **INCONSISTENT** — Party uses crate, others use manual pattern |
| NATS health in /api/ready | **Yes** | No | No | **INCONSISTENT** — only Inventory checks NATS |
| with_request_id on errors | Yes (134 calls) | Yes (46 calls) | Yes (9 branches) | ✓ Consistent |
| PaginatedResponse for lists | Yes (9 endpoints) | Yes (2 endpoints) | Yes (7 endpoints) | ✓ Consistent |
| OpenAPI annotations | Yes (52) | Yes (19) | Yes (25) | ✓ Consistent |
| SecurityAddon Bearer JWT | Yes | Yes | Yes | ✓ Consistent |
| /api/openapi.json route | Yes | Yes | Yes | ✓ Consistent |
| Auto-migrations at startup | Yes | Yes | Yes | ✓ Consistent |
| /healthz endpoint | Yes | Yes | Yes | ✓ Consistent |
| REVISIONS.md tracking | Yes (12 entries) | Yes (8 entries) | Yes (7 entries) | ✓ Consistent |

---

## 5. Build & Test Commands (Run on Host)

Rust is not available in the Cowork sandbox. These commands must be run on the host machine:

```bash
cd "7D-Solutions Platform"

# Platform crates
./scripts/cargo-slot.sh test -p platform-http-contracts
./scripts/cargo-slot.sh test -p config-validator

# Treated modules
./scripts/cargo-slot.sh test -p inventory-rs
./scripts/cargo-slot.sh test -p party-rs
./scripts/cargo-slot.sh test -p bom-rs

# Full workspace (all 37 modules)
./scripts/cargo-slot.sh test --workspace
```

---

## 6. Findings Summary

### Critical (must fix before Wave 2)

1. **Party /api/ready missing NATS check** — Reports only DB health. Should report Degraded when NATS is down but DB is up. Match Inventory's pattern.

2. **BOM /api/ready missing NATS check** — Same issue as Party. Both modules have event bus integration but don't monitor NATS health.

3. **Inventory: 10 files >500 LOC not in allowlist** — These files will fail CI's file size check. Either add to `.file-size-allowlist` with tracking notes or split them:
   - main.rs (724), error_conversions.rs (627), transfer_service.rs (624)
   - cycle_count/approve_service.rs (601), labels.rs (586)
   - valuation/run_service.rs (548), valuation/methods.rs (538), valuation/snapshot_service.rs (512)
   - expiry.rs (503), status/transfer_service.rs (502)

### Important (should fix for consistency)

4. **ConfigValidator inconsistency** — Party uses the `config_validator` crate. Inventory and BOM use manual `Vec<String>` collection. All three achieve the same behavior, but Wave 2 modules should use the crate (as the plan specifies). Consider migrating Inventory and BOM to the crate for consistency.

### Verified Working

5. **All 5 platform crates pass audit** — http-contracts, config-validator, platform-contracts, health, security are all correct, well-tested, and under 500 LOC.

6. **Response envelopes are correct across all 3 modules** — PaginatedResponse and ApiError with request_id are used consistently.

7. **OpenAPI is complete across all 3 modules** — Every handler has utoipa annotations, SecurityAddon is present, /api/openapi.json is served.

8. **Auto-migrations run at startup for all 3 modules**.

9. **REVISIONS.md is complete and version-consistent for all 3 modules**.

10. **Party and BOM have zero files over 500 LOC**.
