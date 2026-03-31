# Plug-and-Play Wave 2: Conversion Plan

> Master plan for converting 22 remaining modules to plug-and-play.
> March 30, 2026

---

## How to Use This Plan

This is a single master document. For each module it specifies:
1. Which beads to create (splits are separate PATCH beads, treatment is MAJOR/MINOR beads)
2. Exactly what each bead does
3. Module-specific gotchas

The proven pattern (from Inventory/Party/BOM) is defined once in Section 1. Per-module specifics are in Section 2. Wave grouping and bead creation order are in Section 3.

---

## Section 1: The Proven Pattern

Every module gets the same treatment in the same order. The steps below are copy-paste instructions for agents.

### Step 0: Split Oversize Files (separate bead, PATCH bump)

**When:** Any `src/` file exceeds 500 LOC.
**What:** Split into logical sub-modules under a directory, re-export public API from `mod.rs`.

```
1. Identify files >500 LOC: `wc -l modules/{name}/src/**/*.rs | sort -rn | head -20`
2. For each oversize file:
   a. Create a subdirectory matching the file name (e.g., service.rs → service/)
   b. Move logical chunks into separate files (e.g., service/types.rs, service/queries.rs, service/core.rs)
   c. Create mod.rs with `pub use` re-exports so the public API is unchanged
   d. Verify all files ≤500 LOC
3. cargo-slot.sh build -p {crate-name}
4. cargo-slot.sh test -p {crate-name}
5. Bump version PATCH in Cargo.toml
6. Add REVISIONS.md entry
7. Commit: [bd-xxx] Split oversize files — PATCH to v{X.Y.Z+1}
```

### Step 1: Response Envelopes (MAJOR bump to next major, e.g., 1.0.0 → 2.0.0)

**What:** Migrate all list endpoints to `PaginatedResponse<T>` and all error responses to `ApiError`.

#### 1a. Add platform-http-contracts dependency

In `Cargo.toml`:
```toml
platform-http-contracts = { path = "../../platform/http-contracts", features = ["axum"] }
```

#### 1b. Create error_conversions.rs

Create `src/domain/error_conversions.rs` (or wherever domain errors live). For every domain error enum, implement `From<XxxError> for ApiError`:

```rust
use platform_http_contracts::ApiError;

impl From<MyDomainError> for ApiError {
    fn from(err: MyDomainError) -> Self {
        match err {
            MyDomainError::NotFound => ApiError::not_found("Resource not found"),
            MyDomainError::Duplicate(msg) => ApiError::conflict(msg),
            MyDomainError::Validation(msg) => ApiError::new(422, "validation_error", msg),
            MyDomainError::Database(e) => {
                tracing::error!(error = %e, "database error");
                ApiError::internal("Database error")
            }
        }
    }
}
```

**Key `ApiError` constructors:**
- `ApiError::not_found(msg)` → 404
- `ApiError::conflict(msg)` → 409
- `ApiError::internal(msg)` → 500
- `ApiError::new(status, error_code, message)` → custom status
- `ApiError::bad_request(msg)` → 400

#### 1c. Migrate list endpoints to PaginatedResponse

For each list handler that returns bare `Vec<T>` or `Json(json!(items))`:

```rust
use platform_http_contracts::PaginatedResponse;

// Extract page/page_size from query params (default: page=1, page_size=50)
let page = query.page.unwrap_or(1).max(1);
let page_size = query.page_size.unwrap_or(50).clamp(1, 200);

// Get total count (COUNT(*) query or items.len() for small sets)
let total = repo.count(&pool, tenant_id).await?;

// Fetch page
let items = repo.list(&pool, tenant_id, page, page_size).await?;

// Return envelope
let resp = PaginatedResponse::new(items, page, page_size, total);
Ok(Json(resp))
```

**PaginatedResponse output format:**
```json
{
  "data": [...],
  "pagination": {
    "page": 1,
    "page_size": 50,
    "total_items": 142,
    "total_pages": 3
  }
}
```

#### 1d. Migrate error responses to ApiError

Replace all inline `json!()` error construction and custom ErrorBody/ErrorResponse structs:

```rust
// BEFORE (inline json):
(StatusCode::NOT_FOUND, Json(json!({"error": "not_found", "message": "Item not found"}))).into_response()

// AFTER:
let api_err: ApiError = domain_err.into();  // uses From impl
api_err.with_request_id(&tracing_ctx).into_response()
```

The `with_request_id()` call populates request_id from TracingContext on all error responses. This is MANDATORY on every error path — PurpleCliff's Wave 1 verification caught Party missing request_id on error responses while Inventory and BOM had it. Every module must include this.

#### 1e. Build, test, version bump

```
1. cargo-slot.sh build -p {crate-name}
2. cargo-slot.sh test -p {crate-name}
3. Bump version MAJOR in Cargo.toml (e.g., 1.0.0 → 2.0.0, or 2.1.8 → 3.0.0)
4. Add REVISIONS.md entry with Breaking? = YES and consumer migration note
5. Commit: [bd-xxx] Standard response envelopes — MAJOR to v{X.0.0}
```

### Step 2: OpenAPI via utoipa (MINOR bump)

**What:** Add `#[utoipa::path]` annotations to every handler, `ToSchema` derives to all types, `SecurityAddon` for Bearer JWT, and `/api/openapi.json` route.

#### 2a. Add utoipa dependencies

In `Cargo.toml`:
```toml
utoipa = { version = "5", features = ["chrono", "uuid"] }
utoipa-axum = "0.2"
```

#### 2b. Annotate handlers

Every handler gets `#[utoipa::path(...)]`:

```rust
#[utoipa::path(
    get,
    path = "/api/{module}/items",
    tag = "{Module}",
    params(ListItemsQuery),
    responses(
        (status = 200, description = "Paginated items", body = PaginatedResponse<Item>),
        (status = 401, description = "Unauthorized"),
        (status = 500, description = "Internal error", body = ApiError),
    ),
    security(("bearer" = ["{MODULE}_READ"]))
)]
pub async fn list_items(...) -> ... { ... }
```

#### 2c. Derive ToSchema on all types

Every request/response/domain type used in handler signatures:
```rust
#[derive(Serialize, Deserialize, utoipa::ToSchema)]
pub struct Item { ... }
```

For query params:
```rust
#[derive(Deserialize, utoipa::IntoParams)]
#[into_params(parameter_in = Query)]
pub struct ListItemsQuery { ... }
```

#### 2d. Create OpenApi struct and serve spec

In `main.rs`:
```rust
use utoipa::OpenApi;

#[derive(OpenApi)]
#[openapi(
    paths(
        // list ALL handler functions here
    ),
    components(schemas(
        // list ALL ToSchema types here
    )),
    modifiers(&SecurityAddon),
)]
struct ApiDoc;

struct SecurityAddon;
impl utoipa::Modify for SecurityAddon {
    fn modify(&self, openapi: &mut utoipa::openapi::OpenApi) {
        let components = openapi.components.get_or_insert_with(Default::default);
        components.add_security_scheme(
            "bearer",
            utoipa::openapi::security::SecurityScheme::Http(
                utoipa::openapi::security::Http::new(
                    utoipa::openapi::security::HttpAuthScheme::Bearer,
                ),
            ),
        );
    }
}
```

Add routes:
```
.route("/api/openapi.json", get(|| async { Json(ApiDoc::openapi()) }))
.route("/healthz", get(health::healthz))  // legacy compat
```

Note: `/healthz` is a legacy health check endpoint kept for backward compatibility across all modules.

#### 2e. Add openapi_dump binary (optional but recommended)

Create `src/bin/openapi_dump.rs`:
```rust
fn main() {
    let spec = serde_json::to_string_pretty(&ApiDoc::openapi()).unwrap();
    println!("{}", spec);
}
```

This allows offline spec generation without needing DB/NATS.

#### 2f. Build, test, version bump

```
1. cargo-slot.sh build -p {crate-name}
2. cargo-slot.sh test -p {crate-name}
3. Bump version MINOR in Cargo.toml
4. Add REVISIONS.md entry
5. Commit: [bd-xxx] OpenAPI via utoipa — MINOR to v{X.Y+1.0}
```

### Step 3: Startup Improvements (MINOR bump)

**What:** Switch to ConfigValidator, add auto-migrations, add NATS graceful degradation (if applicable).

#### 3a. Migrate config.rs to ConfigValidator

Add dependency:
```toml
config-validator = { path = "../../platform/config-validator" }
```

Rewrite `Config::from_env()`:
```rust
use config_validator::ConfigValidator;

pub fn from_env() -> Result<Self, String> {
    let mut v = ConfigValidator::new("{module-name}");

    let database_url = v.require("DATABASE_URL").unwrap_or_default();
    let host = v.optional("HOST").or_default("0.0.0.0");
    let port = v.optional_parse::<u16>("PORT").unwrap_or(80xx);
    let env_name = v.optional("ENV").or_default("development");

    // CORS
    let cors_raw = v.optional("CORS_ORIGINS").or_default("*");
    let cors_origins: Vec<String> = cors_raw.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect();

    // Bus type (if module uses event bus)
    let bus_type_str = v.optional("BUS_TYPE").or_default("inmemory");
    let bus_type = BusType::from_str(&bus_type_str).map_err(|e| { /* push error */ }).unwrap_or(BusType::InMemory);
    let nats_url = v.require_when("NATS_URL", || bus_type == BusType::Nats, "required when BUS_TYPE=nats");

    // Production CORS check
    if env_name == "production" && cors_origins.iter().any(|o| o == "*") {
        // push custom error
    }

    v.finish().map_err(|e| e.to_string())?;

    Ok(Config { database_url, host, port, env: env_name, cors_origins, bus_type, nats_url })
}
```

#### 3b. Add auto-migrations (if missing)

In `main.rs`, after pool creation:
```rust
sqlx::migrate!("./db/migrations")
    .run(&pool)
    .await
    .expect("{Module}: failed to run migrations");
```

#### 3c. Add NATS graceful degradation (if module has event bus)

Follow Inventory's pattern:
- BusType enum (Nats/InMemory) from config
- `connect_nats()` with error handling (don't panic if NATS unavailable)
- BusHealth tracking in AppState
- `/api/ready` reports both DB and NATS health
- If NATS is down but DB is up → Degraded (not Down)

For modules WITHOUT event bus (consolidation, reporting, workforce-competence): skip this entirely.

#### 3d. Add health/ready/version endpoints (if missing or outdated)

Use the `health` platform crate:
```rust
use health::{build_ready_response, db_check_with_pool, nats_check, ready_response_to_axum, PoolMetrics, ReadyResponse, ReadyStatus};
```

Endpoints:
- `GET /api/health` — basic status + uptime
- `GET /api/ready` — verifies DB (and NATS if applicable) connectivity
- `GET /api/version` — module name, version, schema version (SCHEMA_VERSION is a migration timestamp in YYYYMMDDHHMMSS format, e.g., "20260218000001")
- `GET /healthz` — legacy health check (returns basic JSON, kept for backward compat)

#### 3e. Build, test, version bump

```
1. cargo-slot.sh build -p {crate-name}
2. cargo-slot.sh test -p {crate-name}
3. Bump version MINOR in Cargo.toml
4. Add REVISIONS.md entry
5. Commit: [bd-xxx] ConfigValidator + auto-migrations + startup — MINOR to v{X.Y+1.0}
```

### Verification Checklist (every module)

After all steps, verify:
- [ ] `cargo-slot.sh build -p {crate-name}` — clean build, no warnings
- [ ] `cargo-slot.sh test -p {crate-name}` — all tests pass
- [ ] `/api/health` returns 200 with service name and version
- [ ] `/api/ready` returns 200 with DB check (and NATS check if applicable)
- [ ] `/api/version` returns module name, version, schema version
- [ ] `/api/openapi.json` returns valid OpenAPI spec with all endpoints
- [ ] All list endpoints return `{"data": [...], "pagination": {...}}`
- [ ] All error responses return `{"error": "...", "message": "...", "request_id": "..."}`
- [ ] No source files exceed 500 LOC
- [ ] REVISIONS.md has entries for every version bump
- [ ] All version bumps match Cargo.toml

---

## Section 2: Per-Module Specifics

Each module below lists only what differs from the standard pattern. If a field isn't mentioned, follow the standard pattern exactly.

---

### consolidation (v1.0.0 → v2.1.0)

**Split bead:** None needed (all files under 500 LOC).

**Treatment bead (1 bead):**
- **Migrations:** MUST ADD `sqlx::migrate!()` to main.rs (currently missing).
- **List endpoints to migrate:** list_groups, list_entities, list_coa_mappings, list_elimination_rules, list_fx_policies, list_projections (~6 total).
- **Error types:** Replace inline `json!()` errors with ApiError. Create error_conversions.rs for domain errors.
- **Event bus:** None. Skip NATS graceful degradation entirely.
- **Special:** Preserve `gl_base_url` config for external GL service dependency. Keep `optional_claims_mw` (optional JWT verification).
- **Version sequence:** 1.0.0 → 2.0.0 (envelopes) → 2.1.0 (OpenAPI + startup).

---

### customer-portal (v1.0.1 → v2.1.0)

**Split bead:** None needed.

**Treatment bead (1 bead):**
- **List endpoints:** Few — status feed and admin lists only (~2).
- **Auth responses:** Keep custom AuthResponse struct (access_token, refresh_token, token_type). These are NOT list endpoints; don't wrap in PaginatedResponse.
- **Error types:** Replace inline json!() with ApiError.
- **Event bus:** Outbox publisher exists, keep it. No consumers, no NATS graceful degradation needed (or add if you want consistency).
- **Special:** Preserve Argon2 hashing, RS256 JWT generation, refresh token rotation. Preserve external doc-mgmt dependency. Preserve PORTAL_JWT key config.
- **Version sequence:** 1.0.1 → 2.0.0 → 2.1.0.

---

### numbering (v1.0.0 → v2.1.0)

**Split bead:** None needed.

**Treatment bead (1 bead):**
- **List endpoints:** None. Only single-resource lookups.
- **Error types:** Replace custom ErrorResponse struct with ApiError (~4 handlers).
- **Event bus:** Outbox publisher exists, keep it. No consumers.
- **Special:** Preserve advisory lock mechanism (SELECT FOR UPDATE). Preserve idempotency key tracking. Preserve format templates.
- **Version sequence:** 1.0.0 → 2.0.0 → 2.1.0.

---

### pdf-editor (v1.0.0 → v2.1.0)

**Split bead:** None needed.

**Treatment bead (1 bead):**
- **List endpoints:** ~3 need PaginatedResponse (list_templates, list_fields, list_submissions).
- **Error types:** Replace inline json!() and FormError/SubmissionError with ApiError.
- **Special:** PRESERVE the 50 MB custom body limit for PDF uploads. Do NOT replace it with DEFAULT_BODY_LIMIT. Ensure the custom limit is maintained in the route layer.
- **Version sequence:** 1.0.0 → 2.0.0 → 2.1.0.

---

### subscriptions (v1.0.0 → v2.1.0)

**Split bead:** None needed (http.rs at 456, cycle_gating.rs at 450 — both under limit, but OpenAPI annotations may push them over; monitor during treatment).

**Treatment bead (1 bead):**
- **List endpoints:** None. execute_bill_run is the core endpoint.
- **Error types:** Replace custom ErrorResponse (with details field) with ApiError.
- **Dead consumer:** Consumer code exists for `ar.invoice_suspended` but is NOT wired in main.rs. Decision: either wire it in (if it should run) or remove the dead code. Evaluate with team.
- **Event bus:** Outbox publisher exists, keep it.
- **Watch:** If OpenAPI annotations push http.rs past 500 LOC, split it (handlers.rs + types.rs).
- **Version sequence:** 1.0.0 → 2.0.0 → 2.1.0.

---

### workforce-competence (v1.0.0 → v2.1.0)

**Split bead (1 bead, PATCH):**
- domain/service.rs (609 LOC) → service/core.rs + service/queries.rs
- domain/acceptance_authority.rs (542 LOC) → acceptance_authority/grants.rs + acceptance_authority/checks.rs
- Version: 1.0.0 → 1.0.1

**Treatment bead (1 bead):**
- **Migrations:** MUST ADD `sqlx::migrate!()` to main.rs (currently missing).
- **List endpoints:** None (single-item lookups only).
- **Error types:** Replace inline json!() with ApiError (~7 handlers).
- **Event bus:** None. Skip NATS graceful degradation entirely.
- **Version sequence:** 1.0.1 → 2.0.0 → 2.1.0.

---

### notifications (v1.0.0 → v2.1.0)

**Split bead:** None needed.

**Treatment beads (2 beads):**

**Bead 1 — Envelopes:**
- **List endpoints:** ~5 need PaginatedResponse (list_dlq, query_deliveries, list_inbox, list_projections, etc.).
- **Error types:** Replace admin_types::ErrorBody with ApiError.
- **Consumers:** PRESERVE all 3 consumers (invoice_issued, payment_succeeded, payment_failed). Do not modify consumer logic.
- Version: 1.0.0 → 2.0.0

**Bead 2 — OpenAPI + Startup:**
- **Config:** Complex — has .validate() method with sender HTTP endpoint checks, retry policy range validation. Migrate to ConfigValidator carefully.
- **Special:** Preserve dual sender types (Email + SMS), background dispatcher loop, DLQ, escalation rules, template rendering.
- Version: 2.0.0 → 2.1.0

---

### timekeeping (v1.0.0 → v2.1.0)

**Split bead:** None needed (largest file is 450 LOC, but 44 handler functions across 11 files means OpenAPI annotations will be spread out).

**Treatment beads (2 beads):**

**Bead 1 — Envelopes:**
- **List endpoints:** ~12 need PaginatedResponse (list_employees, list_projects, list_tasks, list_entries, list_approvals, list_allocations, list_exports, list_rates, etc.). This is the most list-heavy module of the simples.
- **Error types:** Replace inline json!() with ApiError across all 11 handler files.
- **Dead event code:** events/mod.rs exists but is never spawned. Evaluate: wire it in or remove.
- Version: 1.0.0 → 2.0.0

**Bead 2 — OpenAPI + Startup:**
- 44 handler functions to annotate with utoipa. Large surface area but mechanical.
- Preserve idempotency-key header support.
- Version: 2.0.0 → 2.1.0

---

### ttp (v2.1.8 → v3.1.0)

**Split bead (1 bead, PATCH):**
- domain/billing.rs (488 LOC) — not over 500 yet but OpenAPI annotations WILL push it over. Pre-split: billing/service.rs + billing/types.rs
- Version: 2.1.8 → 2.1.9

**Treatment beads (1 bead — can combine since handler surface is small):**
- **NOTE:** Already at v2.x. Response envelope change is a MAJOR bump to v3.0.0.
- **List endpoints:** ~2 need PaginatedResponse.
- **Error types:** Replace custom ErrorBody (with code field) with ApiError.
- **Dead event code:** Events enveloped but never published. Wire in outbox publisher or remove dead code.
- **External deps:** AR client (request-time env var lookup) and TenantRegistry client. No timeout/retry visible. Consider adding timeouts during startup treatment.
- **Version sequence:** 2.1.9 → 3.0.0 (envelopes) → 3.1.0 (OpenAPI + startup).

---

### shipping-receiving (v1.0.0 → v2.1.0)

**Split bead (1 bead, PATCH):**
- domain/shipments/guards.rs (501 LOC) → guards/validation.rs + guards/authorization.rs
- Version: 1.0.0 → 1.0.1

**Treatment beads (1 bead — moderate handler count):**
- **List endpoints:** ~4 need PaginatedResponse (list_shipments uses limit/offset currently).
- **Error types:** Replace custom ErrorBody with ApiError.
- **Consumers:** PRESERVE 2 consumers (po_approved, so_released) and outbox publisher.
- **Special:** Preserve inventory dual-mode integration (HTTP or deterministic). Preserve optional inventory_url config.
- **Version sequence:** 1.0.1 → 2.0.0 → 2.1.0.

---

### maintenance (v1.0.0 → v2.1.0)

**Split bead (1 bead, PATCH):**
- domain/work_orders/service.rs (551 LOC) → service/core.rs + service/state_machine.rs (or service/lifecycle.rs)
- Version: 1.0.0 → 1.0.1

**Treatment beads (1 bead):**
- **List endpoints:** ~8 need PaginatedResponse.
- **Error types:** Replace custom per-domain error handler functions (asset_error_response, work_order_error_response, etc.) with unified ApiError From impls.
- **Consumers:** PRESERVE 2 consumers (workcenter_bridge, downtime_bridge) and outbox publisher.
- **Special:** PRESERVE scheduler polling task (runs every MAINTENANCE_SCHED_INTERVAL_SECS). Preserve unique config field.
- **Version sequence:** 1.0.1 → 2.0.0 → 2.1.0.

---

### payments (v1.1.20 → v2.1.0)

**Split bead (1 bead, PATCH):**
- http/checkout_sessions.rs (599 LOC) → checkout_sessions/handlers.rs + checkout_sessions/session_logic.rs
- Version: 1.1.20 → 1.1.21

**Treatment beads (1 bead):**
- **List endpoints:** ~2 custom-typed responses. No bare Vec returns; still need consistent ApiError.
- **Error types:** Replace sanitized ErrorBody with ApiError. Preserve sanitization (don't leak internal errors).
- **Consumer:** PRESERVE start_payment_collection_consumer and outbox publisher.
- **Special:** PRESERVE Tilled webhook HMAC-SHA256 signature verification and secret rotation. PRESERVE X-Admin-Token header authz for admin endpoints. Webhook endpoint MUST remain unauthenticated (signature validation only).
- **Conditional config:** PRESERVE PAYMENTS_PROVIDER=tilled conditional validation (TILLED_API_KEY, TILLED_ACCOUNT_ID, TILLED_WEBHOOK_SECRET).
- **Version sequence:** 1.1.21 → 2.0.0 → 2.1.0.

---

### quality-inspection (v1.0.0 → v2.1.0)

**Split bead (1 bead, PATCH):**
- domain/service.rs (683 LOC) → service/inspection_service.rs + service/plan_service.rs (split by receiving vs. in-process vs. final inspection types)
- Version: 1.0.0 → 1.0.1

**Treatment beads (1 bead):**
- **Migrations:** MUST ADD `sqlx::migrate!()` to main.rs (currently missing).
- **Config FIX:** Replace `panic!()` for invalid BUS_TYPE with graceful error handling (Result). This is a bug, not a design choice.
- **Dual DB pool:** PRESERVE the second pool for WORKFORCE_COMPETENCE_DATABASE_URL. ConfigValidator must validate both DATABASE_URL and WORKFORCE_COMPETENCE_DATABASE_URL.
- **List endpoints:** ~4 need PaginatedResponse (by_lot, by_part_rev, by_receipt, by_wo).
- **Error types:** Replace custom error responses with ApiError.
- **Consumers:** PRESERVE 2 consumers (receipt_event_bridge, production_event_bridge). No outbox.
- **BUS_TYPE default:** Currently hardcoded to "nats" as default. Consider aligning to "inmemory" default like other modules, or document the deviation.
- **Version sequence:** 1.0.1 → 2.0.0 → 2.1.0.

---

### reporting (v1.0.0 → v2.1.0)

**Split bead (1 bead, PATCH):**
- domain/statements/cashflow.rs (649 LOC) → cashflow/calculation.rs + cashflow/formatting.rs
- Version: 1.0.0 → 1.0.1

**Treatment beads (1 bead):**
- **List endpoints:** ~2 custom-typed responses. No bare Vec returns.
- **Error types:** Migrate to ApiError.
- **Event bus:** None. Read-only module. Skip NATS graceful degradation entirely.
- **Config:** Minimal — no bus_type, no nats_url. Just DATABASE_URL and PORT.
- **Special:** Preserve app-ID scoped database resolver seam (db::resolve_pool()).
- **Version sequence:** 1.0.1 → 2.0.0 → 2.1.0.

---

### fixed-assets (v1.0.0 → v2.1.0)

**Split bead (1 bead, PATCH):**
- domain/assets/models.rs (536 LOC) → models/types.rs + models/validations.rs
- domain/depreciation/service.rs (530 LOC) → service/schedule.rs + service/calculations.rs
- Version: 1.0.0 → 1.0.1

**Treatment beads (1 bead):**
- **List endpoints:** ~5 need PaginatedResponse.
- **Error types:** Replace inline json!() with ApiError.
- **Consumer:** PRESERVE start_ap_bill_approved_consumer (asset capitalization from AP) and outbox publisher.
- **Version sequence:** 1.0.1 → 2.0.0 → 2.1.0.

---

### production (v1.0.1 → v2.1.0)

**Split bead (1 bead, PATCH):**
- events/mod.rs (704 LOC) → events/types.rs + events/publishing.rs + events/work_order_events.rs
- domain/routings.rs (541 LOC) → routings/service.rs + routings/step_management.rs
- Version: 1.0.1 → 1.0.2

**Treatment beads (1 bead):**
- **List endpoints:** ~8 need PaginatedResponse.
- **Error types:** Replace inline json!() with ApiError.
- **Event bus:** Currently no bus initialized in main.rs. events/mod.rs defines domain events internally. Evaluate: either wire in event bus + outbox or leave as internal-only and document.
- **Special:** Preserve RequirePermissionsLayer (PRODUCTION_READ/PRODUCTION_MUTATE). Preserve workflow state machines.
- **Config:** No BUS_TYPE config exists. If wiring event bus, add it.
- **Version sequence:** 1.0.2 → 2.0.0 → 2.1.0.

---

### workflow (v1.0.0 → v2.1.0)

**Split bead (1 bead, PATCH):**
- domain/routing.rs (672 LOC) → routing/engine.rs + routing/step_definitions.rs
- domain/instances.rs (603 LOC) → instances/state_machine.rs + instances/persistence.rs
- domain/escalation.rs (561 LOC) → escalation/rules.rs + escalation/timeout_handler.rs
- Version: 1.0.0 → 1.0.1

**Treatment beads (1 bead):**
- **List endpoints:** ~2 need PaginatedResponse.
- **Error types:** Replace custom error responses with ApiError.
- **Event bus:** Outbox publisher only (emits events, other modules consume). Keep it.
- **Special:** Preserve durable execution engine, sequential/parallel/conditional step routing, instance state machine.
- **Version sequence:** 1.0.1 → 2.0.0 → 2.1.0.

---

### integrations (v1.0.1 → v2.1.0)

**Split bead (1 bead, PATCH):**
- domain/external_refs/service.rs (640 LOC) → service/crud.rs + service/resolution.rs
- domain/qbo/client.rs (510 LOC) → client/api.rs + client/types.rs
- Version: 1.0.1 → 1.0.2

**Treatment beads (2 beads):**

**Bead 1 — Envelopes:**
- **List endpoints:** ~3 need PaginatedResponse.
- **Error types:** Replace custom ErrorBody struct with ApiError.
- Version: 1.0.2 → 2.0.0

**Bead 2 — OpenAPI + Startup:**
- **Background workers:** PRESERVE QuickBooks OAuth token refresh worker (30s interval, conditional on QBO_CLIENT_ID) and CDC polling worker (15m interval). These are long-running tokio tasks that must NOT be disrupted.
- **Webhook verification:** PRESERVE HMAC-SHA256 signature verification for Stripe and GitHub.
- **Config:** Conditional QBO_CLIENT_ID, QBO_CLIENT_SECRET, QBO_TOKEN_URL. Migrate carefully to ConfigValidator.
- **Special:** PRESERVE EDI transaction processing, file job processing. Preserve webhook normalization logic (qbo_normalizer.rs at 490 LOC — close to limit, monitor).
- Version: 2.0.0 → 2.1.0

---

### treasury (v1.0.1 → v2.1.0)

**Split bead (1 bead, PATCH):**
- domain/accounts/service.rs (691 LOC) → service/bank_accounts.rs + service/credit_cards.rs + service/shared.rs
- domain/import/service.rs (656 LOC) → service/orchestrator.rs + service/parsers.rs (Chase/AMEX parsers)
- domain/reports/forecast.rs (552 LOC) → forecast/calculation.rs + forecast/projection.rs
- Version: 1.0.1 → 1.0.2

**Treatment beads (1 bead):**
- **List endpoints:** ~5 need PaginatedResponse.
- **Error types:** Replace custom ErrorBody with ApiError.
- **Consumers:** PRESERVE 2 consumers (payment reconciliation). PRESERVE outbox publisher.
- **CRITICAL:** PRESERVE rust_decimal::Decimal arithmetic everywhere. Do NOT introduce any f64 for financial calculations. The v1.0.1 fix specifically replaced f64 with Decimal to prevent IEEE 754 rounding errors. Regression here would be a financial accuracy bug.
- **Special:** Preserve CSV parsers (Chase, AMEX proprietary formats).
- **Version sequence:** 1.0.2 → 2.0.0 → 2.1.0.

---

### gl (v1.0.0 → v2.1.0)

**Split bead (1 bead, PATCH):**
- repos/revrec_repo.rs (857 LOC) → revrec_repo/queries.rs + revrec_repo/mutations.rs + revrec_repo/types.rs
- accruals.rs (763 LOC) → accruals/engine.rs + accruals/schedule.rs + accruals/types.rs
- consumers/gl_inventory_consumer.rs (667 LOC) → gl_inventory_consumer/handler.rs + gl_inventory_consumer/posting_logic.rs
- consumers/fixed_assets_depreciation.rs (527 LOC) → split if over 500 after annotations
- services/balance_sheet_service.rs (510 LOC) → balance_sheet/aggregation.rs + balance_sheet/formatting.rs
- consumers/ar_tax_liability.rs (518 LOC) → ar_tax_liability/commit.rs + ar_tax_liability/void.rs
- consumers/ap_vendor_bill_approved.rs (498 LOC) → leave as-is (under 500)
- Version: 1.0.0 → 1.0.1

**Treatment beads (2 beads):**

**Bead 1 — Envelopes:**
- **List endpoints:** ~6 need PaginatedResponse (account activity already has limit/offset, close checklist items, etc.).
- **Error types:** Replace custom ErrorBody with ApiError.
- **Consumers:** PRESERVE all 11 consumers. Do not modify consumer logic. These are the most critical event handlers in the platform.
- Version: 1.0.1 → 2.0.0

**Bead 2 — OpenAPI + Startup:**
- 35 handler functions to annotate.
- **Config:** Preserve DLQ validation flag (dlq_validation_enabled).
- **Special:** Preserve CurrencyConfigRegistry (in-memory, per-tenant). Preserve revenue recognition engine. Preserve accruals engine. Preserve period close DLQ validation gate.
- Version: 2.0.0 → 2.1.0

---

### ap (v1.0.0 → v2.1.0)

**Split bead (1 bead, PATCH — largest split job in the platform, 11 files):**
- domain/po/service.rs (716 LOC) → service/create.rs + service/lifecycle.rs
- domain/match/engine.rs (711 LOC) → engine/two_way.rs + engine/three_way.rs + engine/scoring.rs
- domain/bills/service.rs (691 LOC) → service/create.rs + service/approval.rs + service/queries.rs
- domain/vendors/service.rs (632 LOC) → service/crud.rs + service/validation.rs
- domain/tax/service.rs (618 LOC) → service/quoting.rs + service/commit_void.rs
- domain/reports/aging.rs (609 LOC) → aging/buckets.rs + aging/calculation.rs
- domain/payment_runs/builder.rs (572 LOC) → builder/selection.rs + builder/assembly.rs
- domain/payment_runs/execute.rs (566 LOC) → execute/disbursement.rs + execute/reconciliation.rs
- domain/po/approve.rs (558 LOC) → approve/workflow.rs + approve/validation.rs
- domain/bills/mod.rs (541 LOC) → bills/types.rs + bills/queries.rs
- domain/bills/approve.rs (526 LOC) → approve/checks.rs + approve/execute.rs
- Version: 1.0.0 → 1.0.1

**Treatment beads (2 beads):**

**Bead 1 — Envelopes:**
- **List endpoints:** ~8 need PaginatedResponse.
- **Error types:** Replace ErrorBody{code, message} with ApiError.
- **Consumers:** PRESERVE 2 consumers (inventory_item_received). PRESERVE outbox publisher.
- Version: 1.0.1 → 2.0.0

**Bead 2 — OpenAPI + Startup:**
- 33 handler functions to annotate.
- **Special:** Preserve tax-core dependency. Preserve 2-way/3-way matching engine logic. Preserve payment run builder/executor pattern.
- Version: 2.0.0 → 2.1.0

---

### ar (v1.0.64 → v2.1.0)

**Split bead (1 bead, PATCH — 10 files):**
- http/webhooks.rs (1256 LOC) → webhooks/signature.rs + webhooks/customer_events.rs + webhooks/payment_events.rs + webhooks/subscription_events.rs + webhooks/invoice_events.rs + webhooks/mod.rs
- credit_notes.rs (822 LOC) → credit_notes/lifecycle.rs + credit_notes/calculations.rs + credit_notes/queries.rs
- http/invoices.rs (793 LOC) → invoices/handlers.rs + invoices/request_types.rs
- http/payment_methods.rs (780 LOC) → payment_methods/handlers.rs + payment_methods/tilled_ops.rs
- finalization.rs (737 LOC) → finalization/engine.rs + finalization/validation.rs
- tilled/types.rs (595 LOC) → types/requests.rs + types/responses.rs + types/webhooks.rs
- http/credit_notes.rs (582 LOC) → credit_notes_handlers/handlers.rs + credit_notes_handlers/types.rs
- http/subscriptions.rs (576 LOC) → subscriptions/handlers.rs + subscriptions/types.rs
- progress_billing.rs (557 LOC) → progress_billing/engine.rs + progress_billing/types.rs
- http/charges.rs (531 LOC) → charges/handlers.rs + charges/types.rs
- Version: 1.0.64 → 1.0.65

**Treatment beads (3 beads):**

**Bead 1 — Envelopes:**
- **List endpoints:** ~15 need PaginatedResponse. Largest migration surface of any module.
- **Error types:** Replace ErrorResponse{code, message} with ApiError.
- **CRITICAL:** Webhook event processing responses (to Tilled) must NOT change. The external contract must be preserved. Only internal API responses get the new envelope.
- **Consumer:** PRESERVE payment_succeeded_consumer and outbox publisher.
- Version: 1.0.65 → 2.0.0

**Bead 2 — OpenAPI:**
- 62 handler functions to annotate. Largest annotation surface in the platform.
- Document Tilled webhook endpoints separately (external contract, different auth model).
- Version: 2.0.0 → 2.1.0

**Bead 3 — Startup + Hardening:**
- **Config:** PRESERVE TILLED_WEBHOOK_SECRET with fallback order. PRESERVE PARTY_MASTER_URL config.
- **HMAC verification:** PRESERVE replay window guard (±5 minutes). PRESERVE constant-time comparison. Do NOT modify signature verification logic.
- **Webhook event types (15 total, all must be preserved):** customer.created, customer.updated, payment_intent.succeeded, payment_intent.failed, payment_method.attached, payment_method.detached, subscription.created, subscription.updated, subscription.canceled, charge.succeeded, charge.failed, charge.refunded, invoice.created, invoice.payment_succeeded, invoice.payment_failed.
- **Special:** Preserve Party Master integration for customer verification. Preserve idempotent customer sync (upsert by tilled_customer_id).
- Version: 2.1.0 → 2.2.0

---

## Section 3: Wave Grouping and Bead Creation Order

### Wave A — Quick Wins (6 modules, 8 beads total)

Execute first. All are simple copy-paste pattern with minimal deviations.

| # | Module | Split Bead | Treatment Beads | Total |
|---|--------|-----------|----------------|-------|
| 1 | consolidation | — | 1 | 1 |
| 2 | customer-portal | — | 1 | 1 |
| 3 | numbering | — | 1 | 1 |
| 4 | pdf-editor | — | 1 | 1 |
| 5 | subscriptions | — | 1 | 1 |
| 6 | workforce-competence | 1 (PATCH) | 1 | 2 |
| | **Wave A Total** | **1** | **6** | **8** |

All 6 treatment beads can run in parallel after the workforce-competence split completes.

### Wave B — Medium Core (8 modules, 18 beads total)

| # | Module | Split Bead | Treatment Beads | Total |
|---|--------|-----------|----------------|-------|
| 7 | notifications | — | 2 | 2 |
| 8 | timekeeping | — | 2 | 2 |
| 9 | ttp | 1 (PATCH) | 2 | 3 |
| 10 | shipping-receiving | 1 (PATCH) | 1 | 2 |
| 11 | maintenance | 1 (PATCH) | 1 | 2 |
| 12 | payments | 1 (PATCH) | 1 | 2 |
| 13 | quality-inspection | 1 (PATCH) | 1 | 2 |
| 14 | reporting | 1 (PATCH) | 1 | 2 |
| | **Wave B Total** | **5** | **11** | **18** |

Split beads first (all can run in parallel), then treatment beads (all can run in parallel).

### Wave C — Medium Complex (4 modules, 11 beads total)

| # | Module | Split Bead | Treatment Beads | Total |
|---|--------|-----------|----------------|-------|
| 15 | fixed-assets | 1 (PATCH) | 1 | 2 |
| 16 | production | 1 (PATCH) | 1 | 2 |
| 17 | workflow | 1 (PATCH) | 1 | 2 |
| 18 | integrations | 1 (PATCH) | 2 | 3 |
| | **Wave C Total** | **4** | **5** | **11** |

### Wave D — Heavy Financial (2 modules, 7 beads total)

| # | Module | Split Bead | Treatment Beads | Total |
|---|--------|-----------|----------------|-------|
| 19 | treasury | 1 (PATCH) | 1 | 2 |
| 20 | gl | 1 (PATCH) | 2 | 3 |
| | **Wave D Total** | **2** | **3** | **7** |

### Wave E — AP (1 module, 3 beads total)

| # | Module | Split Bead | Treatment Beads | Total |
|---|--------|-----------|----------------|-------|
| 21 | ap | 1 (PATCH) | 2 | 3 |

### Wave F — AR (1 module, 4 beads total)

| # | Module | Split Bead | Treatment Beads | Total |
|---|--------|-----------|----------------|-------|
| 22 | ar | 1 (PATCH) | 3 | 4 |

---

## Grand Totals

| Metric | Count |
|--------|-------|
| Modules | 22 |
| Split beads (PATCH) | 14 |
| Treatment beads (MAJOR + MINOR) | 37 |
| **Total beads** | **51** |
| Files needing splits | 46 |
| Missing migrations to add | 3 |
| Panic-to-Result fixes | 1 |
| Dead event code to evaluate | 4 modules |
