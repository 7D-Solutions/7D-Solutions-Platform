# Service Patterns — Canonical Reference

> **Scope:** All 25 services in `modules/*/src/main.rs` plus `platform/security/src/`.
> **Gold-standard references:** `modules/gl/src/main.rs` and `modules/inventory/src/main.rs`.
> **Deviations** are flagged with `file:line` in the per-service table at the end.

---

## 1. Middleware Layer Order

Axum layers are applied bottom-to-top (innermost first, outermost last). The canonical order, read from innermost to outermost, is:

```
.layer(DefaultBodyLimit::max(DEFAULT_BODY_LIMIT))               // 2 MiB — gl:290, inv:317
.layer(axum::middleware::from_fn(security::tracing::tracing_context_middleware))
.layer(axum::middleware::from_fn(timeout_middleware))           // 30 s
.layer(axum::middleware::from_fn(rate_limit_middleware))        // 200 req / 60 s
.layer(Extension(default_rate_limiter()))                       // governor state
.layer(axum::middleware::from_fn_with_state(
    maybe_verifier, optional_claims_mw,                        // JWT extraction (optional)
))
.layer(build_cors_layer(&config))                              // CORS — outermost
```

Constants live in `platform/security/src/middleware.rs`:

| Constant | Value | Source |
|---|---|---|
| `DEFAULT_BODY_LIMIT` | 2 097 152 bytes (2 MiB) | `security/src/middleware.rs` |
| `DEFAULT_REQUEST_TIMEOUT` | 30 seconds | `security/src/middleware.rs` |
| `DEFAULT_RATE_LIMIT` (capacity) | 200 requests | `security/src/middleware.rs` |
| Rate limit refill period | 60 seconds | `security/src/middleware.rs` |

### Router merge order (canonical)

```rust
let app = Router::new()
    // 1. Health / ops routes (no auth required)
    .route("/healthz", get(health::healthz))
    .route("/api/health", get(health))
    .route("/api/ready", get(ready))
    .route("/api/version", get(version))
    .route("/metrics", get(metrics_handler))
    .with_state(app_state)
    // 2. Read sub-router (RequirePermissionsLayer gated)
    .merge(reads_router)
    // 3. Mutation sub-router (RequirePermissionsLayer gated)
    .merge(mutations_router)
    // 4. Admin router (separate pool, internal-only)
    .merge(admin_router(pool.clone()))
    // 5. Security middleware stack (bottom-to-top)
    .layer(...)
```

---

## 2. Health Endpoints

Every service must expose exactly these five routes:

| Route | Purpose | HTTP verb |
|---|---|---|
| `/healthz` | Kubernetes liveness probe | GET |
| `/api/health` | Docker Compose health check (used in `healthcheck:`) | GET |
| `/api/ready` | Readiness / dependency check | GET |
| `/api/version` | Build version info | GET |
| `/metrics` | Prometheus scrape endpoint | GET |

The Docker Compose health check uses `/api/health`, not `/healthz`:

```yaml
healthcheck:
  test: ["CMD-SHELL", "curl -f http://localhost:${PORT}/api/health || exit 1"]
  interval: 10s
  timeout: 5s
  retries: 5
  start_period: 30s
```

---

## 3. Auth Middleware Conventions

### JWT extraction (global, optional)

Applied to the entire router so all handlers can read claims if present:

```rust
let maybe_verifier = JwtVerifier::from_env_with_overlap().map(Arc::new);
// ...
.layer(axum::middleware::from_fn_with_state(maybe_verifier, optional_claims_mw))
```

`optional_claims_mw` never rejects a request — it only populates `Extension<VerifiedClaims>` when a valid token is present. Permission enforcement is done separately.

### RBAC enforcement (per sub-router)

Read routes and mutation routes live in separate sub-routers, each gated by `RequirePermissionsLayer`:

```rust
// Reads
let reads = Router::new()
    .route(...)
    .route_layer(RequirePermissionsLayer::new(&[permissions::INVENTORY_READ]))
    .with_state(app_state.clone());

// Mutations
let mutations = Router::new()
    .route(...)
    .route_layer(RequirePermissionsLayer::new(&[permissions::INVENTORY_MUTATE]))
    .with_state(app_state.clone());
```

Permission constants are defined in `platform/security/src/permissions.rs`.

### Claims extraction in handlers

Handlers access tenant identity via:

```rust
pub async fn create_item(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Json(mut req): Json<CreateItemRequest>,
) -> impl IntoResponse {
    let tenant_id = match extract_tenant(&claims) { ... };
```

`VerifiedClaims` is optional — handlers must tolerate absent claims (service-to-service paths).

---

## 4. Request Struct Conventions

- Named `{Action}{Resource}Request` (e.g., `CreateItemRequest`, `UpdateItemRequest`).
- Derived: `#[derive(serde::Deserialize)]`.
- Extracted via `Json<XxxRequest>` in handler signature.
- `tenant_id` field is **overwritten from JWT claims** inside the handler — never trusted from the body.

```rust
Json(mut req): Json<CreateItemRequest>,
// ...
req.tenant_id = tenant_id;   // always override with verified identity
```

---

## 5. Response ID Naming

- Primary key: `id` (UUID).
- Foreign keys: domain-qualified name (`invoice_id`, `tenant_id`, `customer_id`, etc.).
- Successful creation responses return `StatusCode::CREATED` (201).
- Successful retrieval/update responses return `StatusCode::OK` (200).
- Responses serialized via `serde_json::json!()` or a typed struct with `#[derive(serde::Serialize)]`.

---

## 6. Error Handling

Canonical pattern: domain error enum → match → `(StatusCode, Json(json!({...})))`:

```rust
fn item_error_response(err: ItemError) -> impl IntoResponse {
    match err {
        ItemError::NotFound => (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "not_found", "message": "Item not found" })),
        ),
        ItemError::Validation(msg) => (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({ "error": "validation_error", "message": msg })),
        ),
        ItemError::Database(e) => {
            tracing::error!(error = %e, "database error");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": "internal_error", "message": "Database error" })),
            )
        }
    }
}
```

Error JSON shape:

```json
{ "error": "<snake_case_code>", "message": "<human readable>" }
```

Database errors are logged with `tracing::error!` before returning a generic 500 — the raw error is **never** forwarded to the client.

---

## 7. Outbox Pattern

Services that publish domain events use a transactional outbox table. The publisher task is spawned at startup:

```rust
// Canonical form (used by AR, AP, Fixed Assets, Workflow, Treasury, etc.)
let publisher_pool = pool.clone();
let publisher_bus  = bus.clone();
tokio::spawn(async move {
    outbox::run_publisher_task(publisher_pool, publisher_bus).await;
});
tracing::info!("ServiceName: outbox publisher task started");
```

Services using outbox: AR, AP, Fixed Assets, Shipping-Receiving, Subscriptions, Workflow, Numbering, Maintenance, Treasury, Payments, Notifications, PDF Editor (12 of 25).

Services without outbox (no domain events published): BOM, Consolidation, Customer Portal, GL (consumes events via NATS, does not publish via outbox), Integrations, Inventory, Party, Production, Quality Inspection, Reporting, Timekeeping, TTP, Workforce Competence.

### Event bus initialisation

Services that need an event bus initialise it from `BUS_TYPE` env var:

```rust
let bus: Arc<dyn EventBus> = match config.bus_type.as_str() {
    "inmemory" => Arc::new(InMemoryBus::new()),
    "nats" => {
        let client = event_bus::connect_nats(&config.nats_url).await
            .expect("Failed to connect to NATS");
        Arc::new(NatsBus::new(client))
    }
    _ => panic!("Invalid BUS_TYPE: {}. Must be 'inmemory' or 'nats'", config.bus_type),
};
```

`config.bus_type` is a `String` matched with `.as_str()` in most services. GL uses a typed enum match (see GL deviation note).

---

## 8. Docker Compose Environment Variables

Canonical env vars per service in `docker-compose.modules.yml`:

```yaml
environment:
  DATABASE_URL: postgres://${SVC_USER}:${SVC_PASSWORD}@7d-svc-postgres:5432/${SVC_DB}?sslmode=require
  NATS_URL: nats://platform:${NATS_AUTH_TOKEN:-dev-nats-token}@7d-nats:4222
  BUS_TYPE: nats
  HOST: 0.0.0.0
  PORT: <service-port>
  JWT_PUBLIC_KEY: ${JWT_PUBLIC_KEY_PEM}
  RUST_LOG: ${RUST_LOG:-info}
```

Optional (present only on some services):

```yaml
  CORS_ORIGINS: ${SVC_CORS_ORIGINS}
```

Health check — always targets `/api/health`:

```yaml
healthcheck:
  test: ["CMD-SHELL", "curl -f http://localhost:<PORT>/api/health || exit 1"]
  interval: 10s
  timeout: 5s
  retries: 5
  start_period: 30s
```

---

## 9. Database Migration Conventions

Migrations run at startup before the HTTP server binds:

```rust
sqlx::migrate!("./db/migrations")   // note: "./" prefix is required
    .run(&pool)
    .await
    .expect("Failed to run migrations");
```

Migration files live at `modules/<svc>/db/migrations/`.

Services that skip `sqlx::migrate!` in main.rs: BOM, Consolidation, Inventory, Quality Inspection, Workforce Competence — these may rely on external migration tooling or run migrations inside `resolve_pool`.

---

## 10. Database Pool Initialisation

Canonical: use `resolve_pool` from the service's `db::resolver` module:

```rust
let pool = resolve_pool(&config.database_url)
    .await
    .expect("Failed to connect to database");
```

`resolve_pool` encapsulates pool sizing, connection timeout, and ssl handling. Using raw `PgPoolOptions::new().max_connections(10)` is a deviation (see table below).

---

## 11. CORS Construction

Canonical `build_cors_layer` function in `main.rs`:

```rust
fn build_cors_layer(config: &Config) -> CorsLayer {
    let is_wildcard = config.cors_origins.len() == 1 && config.cors_origins[0] == "*";

    if is_wildcard && config.env != "development" {
        tracing::warn!("CORS_ORIGINS is set to wildcard — restrict to specific origins in production");
    }

    let layer = if is_wildcard {
        CorsLayer::new().allow_origin(AllowOrigin::any())
    } else {
        let origins: Vec<_> = config.cors_origins.iter()
            .filter_map(|o| o.parse().ok())
            .collect();
        CorsLayer::new().allow_origin(origins)
    };

    layer
        .allow_methods(tower_http::cors::Any)
        .allow_headers(tower_http::cors::Any)
}
```

Notable: no `.allow_credentials(false)` call — the default is fine. Services that add `.allow_credentials(false)` are redundant but not wrong; services that are **missing** the production wildcard warning are a configuration hazard.

---

## 12. AppState Shape

Canonical `AppState` is defined in `src/lib.rs`, not `main.rs`:

```rust
pub struct AppState {
    pub pool: PgPool,
    pub metrics: Arc<ServiceMetrics>,
}
```

Passed to routers as `Arc<AppState>`. Services that embed AppState in `main.rs` or omit `metrics` are deviating.

---

## 13. Server Bind Address

Canonical: reads host from config, binds to all interfaces in practice:

```rust
let addr = SocketAddr::from(([0, 0, 0, 0], config.port));
```

Some services use `format!("{}:{}", config.host, config.port).parse()` — functionally equivalent but deviates from the concise form.

---

## Per-Service Deviation Table

`✓` = canonical. `⚠` = deviation with reference.

| Pattern | gl | inventory | ar | ap | bom | consolidation | customer-portal | fixed-assets | integrations | maintenance | notifications | numbering | party | payments | pdf-editor | production | quality-inspection | reporting | shipping-receiving | subscriptions | timekeeping | treasury | ttp | workflow | workforce-competence |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| resolve_pool | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ⚠ | ✓ | ✓ | ⚠ | ✓ | ⚠ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ⚠ | ✓ | ✓ | ✓ | ✓ | ⚠ | ✓ |
| sqlx::migrate! `./db/…` | ✓ | — | ✓ | ✓ | — | — | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ⚠ | — | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | — |
| Full health route set | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ⚠ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ⚠ | ✓ | ✓ | ✓ | ✓ | ✓ | ⚠ |
| Full middleware stack | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ⚠ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ |
| Read routes under RequirePermissionsLayer | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | — | ⚠ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ⚠ | ✓ | ✓ | ✓ | ✓ | ✓ | ⚠ | ✓ | ⚠ | ✓ |
| Separate read/mutate permissions | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | — | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ⚠ | ✓ | ✓ | ✓ | ⚠ | ✓ | ✓ | ✓ | ⚠ | ✓ |
| AppState in lib.rs | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ⚠ | ✓ | ✓ | ✓ | ✓ | ✓ |
| CORS wildcard warning | ✓ | ✓ | ✓ | — | — | — | — | — | — | ✓ | — | — | — | — | — | — | — | — | — | — | — | — | — | — | — |
| admin_router merge | ✓ | ✓ | — | — | — | — | — | — | — | — | — | — | — | — | — | — | — | — | — | — | ✓ | — | — | — | — |

---

## Detailed Deviation Notes

### customer-portal — CRITICAL

**File:** `modules/customer-portal/src/main.rs`

- **Line 25** — uses `sqlx::postgres::PgPoolOptions::new().max_connections(10)` instead of `resolve_pool`. Pool sizing is hardcoded.
- **Missing entire security middleware stack** — no `tracing_context_middleware`, no `timeout_middleware`, no `rate_limit_middleware`, no `optional_claims_mw`, no `build_cors_layer`. All security guarantees provided by the platform are absent from this service.
- **Missing health routes** — `/healthz`, `/api/health`, `/api/ready`, `/api/version`, `/metrics` not visible in main.rs; all routing delegated to `build_router(state)` in lib which may or may not expose them.

### fixed-assets — SECURITY

**File:** `modules/fixed-assets/src/main.rs`

- **Lines ~85-120** — Read routes are placed directly in the main `Router::new()` **without** a `RequirePermissionsLayer`. Only mutation routes are permission-gated. Any authenticated (or unauthenticated) caller can read fixed asset records.

### treasury — SECURITY

**File:** `modules/treasury/src/main.rs`

- **Lines ~85-130** — Read routes have no `RequirePermissionsLayer`. Data is publicly accessible to any caller that passes the middleware stack.
- **Extra `metrics::latency_layer`** applied to the main router before merges — not part of the canonical stack.

### production — MIGRATION PATH

**File:** `modules/production/src/main.rs`

- **Line 53** — `sqlx::migrate!("db/migrations")` — missing `./` prefix. This resolves relative to the working directory at runtime; the canonical form `"./db/migrations"` is explicit.
- No `RequirePermissionsLayer` on any route.
- No CORS wildcard production warning.

### workflow — POOL + PERMISSIONS

**File:** `modules/workflow/src/main.rs`

- **Line ~35** — `PgPoolOptions::new()` instead of `resolve_pool`.
- Both read and mutation routes are gated under a single `WORKFLOW_MUTATE` permission — there is no separate `WORKFLOW_READ` permission.

### shipping-receiving — HEALTH ROUTES

**File:** `modules/shipping-receiving/src/main.rs`

- **Line ~35** — `PgPoolOptions::new()` instead of `resolve_pool`.
- **Missing** `/api/health`, `/api/ready`, `/api/version` routes. Only `/healthz` and `/metrics` are present. The Docker Compose health check (`/api/health`) will fail against the running service.

### numbering — ROUTE PREFIX + POOL

**File:** `modules/numbering/src/main.rs`

- **Line ~35** — `PgPoolOptions::new()` instead of `resolve_pool`.
- Route paths lack the `/api/numbering/` namespace prefix — routes registered as `/allocate`, `/confirm`, `/policies/{entity}`. All other services prefix routes with `/api/<module>/`.
- Has extra `/api/schema-version` route not present in any other service.
- Missing CORS wildcard production warning.

### maintenance — POOL

**File:** `modules/maintenance/src/main.rs`

- **Line ~35** — `PgPoolOptions::new()` instead of `resolve_pool`.

### quality-inspection — BUS TYPE + MIGRATIONS

**File:** `modules/quality-inspection/src/main.rs`

- Event bus type read as `String` and matched with `.to_lowercase().as_str()` — not a typed enum like the canonical approach.
- No `sqlx::migrate!` call in main.rs.
- Has dual DB pools (`pool` + `wc_pool` for workcenter data).
- Missing CORS wildcard production warning.

### notifications — ROUTE PREFIX + STATE

**File:** `modules/notifications/src/main.rs`

- **Line 159** — extra `/ready` route registered **without** the `/api/` prefix. The canonical route is `/api/ready`. Both exist, but the unprefixed form is inconsistent.
- Uses raw `db.clone()` as router state instead of `Arc<AppState>`.

### subscriptions — APPSTATE + PERMISSIONS + CONFIG

**File:** `modules/subscriptions/src/main.rs`

- `AppState` struct defined in `main.rs` (lines ~17-25) instead of `lib.rs`.
- Host/port read directly from `std::env::var("HOST")` and `std::env::var("PORT")` instead of `config.host` / `config.port`.
- All routes gated under single `SUBSCRIPTIONS_MUTATE` permission — no separate read permission.

### payments — STATE + INLINE METRICS

**File:** `modules/payments/src/main.rs`

- `metrics_handler` function defined inline in `main.rs` (lines ~25-35) rather than imported from the metrics module.
- `AppState` has no `metrics` field (holds Tilled API credentials instead).
- Read and mutation routes not separated into distinct sub-routers.

### pdf-editor — BODY LIMIT + CORS

**File:** `modules/pdf-editor/src/main.rs`

- `build_cors_layer` imported from the lib's `cors` module rather than defined in `main.rs`.
- Custom `52_428_800` bytes (50 MiB) body limit applied to the PDF upload route — intentional and documented; the platform default 2 MiB limit remains on other routes.
- Health routes placed in a separate `health_routes` sub-router.
- Uses raw `db.clone()` as router state.

### workforce-competence — HEALTH + ADDRESS

**File:** `modules/workforce-competence/src/main.rs`

- Missing `/healthz` liveness route.
- Has extra `/api/schema-version` route.
- `SocketAddr::from(([0, 0, 0, 0], config.port))` hardcodes `0.0.0.0`, ignoring `config.host`.
- Missing CORS wildcard production warning.

### ar, ap — CORS ALLOW_CREDENTIALS + ADDRESS

**Files:** `modules/ar/src/main.rs`, `modules/ap/src/main.rs`

- Both add `.allow_credentials(false)` to the CORS layer — redundant (the default) but not harmful.
- Bind address constructed as `format!("{}:{}", config.host, config.port).parse()` instead of `SocketAddr::from(([0, 0, 0, 0], config.port))`.

### bom — CORS WARNING

**File:** `modules/bom/src/main.rs`

- `build_cors_layer` missing the production wildcard warning (`tracing::warn!(...)`).

### consolidation — MIGRATIONS

**File:** `modules/consolidation/src/main.rs`

- No `sqlx::migrate!` call in main.rs. Schema management mechanism unclear.

---

## Summary of Security-Critical Deviations

These deviations create actual security exposure and should be resolved first:

| Service | Issue | Severity |
|---|---|---|
| **customer-portal** | Entire security middleware stack absent (no rate limit, no timeout, no JWT, no CORS) | Critical |
| **fixed-assets** | Read routes unprotected — no `RequirePermissionsLayer` on reads | High |
| **treasury** | Read routes unprotected — no `RequirePermissionsLayer` on reads | High |
| **production** | No permission gating on any route | High |
| **workflow** | No separate read permission — read access requires mutate permission | Medium |
| **subscriptions** | No separate read permission — read access requires mutate permission | Medium |
| **shipping-receiving** | `/api/health` route missing — Docker health check broken | Medium |
