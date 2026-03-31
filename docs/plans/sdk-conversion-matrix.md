# SDK Conversion Matrix

> Verified module-by-module conversion plan for migrating 22 remaining modules
> to `platform-sdk` v1.0 `ModuleBuilder`.
>
> **Date:** 2026-03-31
> **Bead:** bd-dwdnz
> **Method:** Every claim verified by reading current `src/main.rs` for each module.

---

## How to Read This Matrix

Each module is assigned to a **conversion group** (A through E) based on the
complexity of its `main.rs` relative to the SDK's `ModuleBuilder` pattern.

The SDK replaces all per-module boilerplate: dotenv, tracing, database pool,
migrations, event bus init, outbox publisher, JWT verification, rate limiting,
CORS, health endpoints (`/healthz`, `/api/health`, `/api/ready`, `/api/version`,
`/metrics`), graceful shutdown, and TCP binding. What remains module-specific
goes into the `.routes(|ctx| ...)` closure.

**Already converted (reference implementations):**
- **Party** — Group A pattern (migrator + routes)
- **Production** — Group B pattern (migrator + routes, bus in module.toml)
- **AR** — Group C pattern (migrator + consumer + routes)

---

## Conversion Groups

| Group | Description | SDK Shape | Count |
|-------|-------------|-----------|-------|
| A | HTTP-only, no event bus | `from_manifest().migrator().routes().run()` | 6 |
| B | Publisher-only (bus + outbox, no consumers) | + `[bus]` and `[events.publish]` in module.toml | 5 |
| C | Standard consumers (1-2 consumers) | + `.consumer(subject, handler)` calls | 4 |
| D | Heavy consumers or complex bus patterns | + multiple `.consumer()` calls, careful adaptation | 3 |
| E | Non-standard patterns requiring extra work | bus supervisor, dual DB, background workers | 4 |

---

## Group A — HTTP-Only (6 modules)

No event bus in `main.rs`. Conversion is mechanical: create `module.toml`, replace
entire `main()` with `ModuleBuilder::from_manifest().migrator().routes().run()`.
Custom `AppState` is built inside the `.routes(|ctx| ...)` closure using `ctx.pool()`.

### 1. reporting

| Attribute | Current State |
|-----------|--------------|
| Version | 2.1.0 |
| Port | 8097 |
| Bus init | No |
| Publisher | No |
| Consumers | None |
| Migrations | Yes (`sqlx::migrate!`) |
| OpenAPI | Yes (utoipa, in main.rs) |
| Custom state | `pool + metrics` (standard) |
| Special middleware | None |
| main.rs LOC | 266 |

**Conversion notes:** Clean drop-in. No bus config needed. Pool uses
`db::resolve_pool()` (app-ID scoped resolver) — this works inside the routes
closure since `ctx.pool()` provides the pool. OpenAPI struct stays in main.rs.

**Verification:**
```bash
./scripts/cargo-slot.sh build -p reporting
./scripts/cargo-slot.sh test -p reporting
curl http://localhost:8097/api/health
curl http://localhost:8097/api/openapi.json | jq .info.title
```

---

### 2. timekeeping

| Attribute | Current State |
|-----------|--------------|
| Version | 2.1.4 |
| Port | 8102 |
| Bus init | No (config has `bus_type` field but not used in main.rs) |
| Publisher | No |
| Consumers | None (events/mod.rs exists but is never spawned) |
| Migrations | Yes |
| OpenAPI | Yes (utoipa, via `http::ApiDoc`) |
| Custom state | `pool + metrics` (standard) |
| Special middleware | None |
| main.rs LOC | 152 |

**Conversion notes:** Dead `bus_type` config field and dead `events/mod.rs` —
evaluate: remove dead code or wire consumer in if it should run. For SDK
conversion, ignore the dead event code. Module.toml does NOT need `[bus]`.

**Verification:**
```bash
./scripts/cargo-slot.sh build -p timekeeping
./scripts/cargo-slot.sh test -p timekeeping
curl http://localhost:8102/api/health
curl http://localhost:8102/api/openapi.json | jq .info.title
```

---

### 3. ttp

| Attribute | Current State |
|-----------|--------------|
| Version | 2.1.11 |
| Port | 8100 |
| Bus init | No (config has `bus_type` but no bus created in main.rs) |
| Publisher | No |
| Consumers | None (events are enveloped but never published) |
| Migrations | Yes |
| OpenAPI | No (not present in main.rs — must be added) |
| Custom state | `pool + metrics` (standard) |
| Special middleware | None |
| main.rs LOC | 144 |

**Conversion notes:** Dead `bus_type` config and dead event code — same decision
as timekeeping. No OpenAPI yet: either add during SDK conversion or as a
separate bead. External deps: AR client and TenantRegistry client use env var
lookups at request time — no startup wiring needed. Module.toml does NOT need `[bus]`.

**Verification:**
```bash
./scripts/cargo-slot.sh build -p ttp-rs
./scripts/cargo-slot.sh test -p ttp-rs
curl http://localhost:8100/api/health
```

---

### 4. workforce-competence

| Attribute | Current State |
|-----------|--------------|
| Version | 2.1.4 |
| Port | 8110 |
| Bus init | No |
| Publisher | No |
| Consumers | None |
| Migrations | Yes |
| OpenAPI | No (not present — must be added) |
| Custom state | `pool + metrics` (standard) |
| Special middleware | None |
| main.rs LOC | 189 |

**Conversion notes:** Clean drop-in. No bus config needed. No OpenAPI yet.
Route setup currently done inline in main.rs — move into routes closure.

**Verification:**
```bash
./scripts/cargo-slot.sh build -p workforce-competence-rs
./scripts/cargo-slot.sh test -p workforce-competence-rs
curl http://localhost:8110/api/health
```

---

### 5. bom

| Attribute | Current State |
|-----------|--------------|
| Version | 2.2.1 |
| Port | 8120 |
| Bus init | No |
| Publisher | No |
| Consumers | None |
| Migrations | Yes |
| OpenAPI | Yes (utoipa, in main.rs) |
| Custom state | `pool + metrics + numbering` (NumberingClient) |
| Special middleware | None |
| main.rs LOC | 334 |

**Conversion notes:** Custom state includes `NumberingClient::http(config.numbering_url)`.
Build this inside the routes closure — read `NUMBERING_URL` from env directly
or pass via module.toml custom config. The SDK doesn't pass config to routes,
so read the env var in the closure: `std::env::var("NUMBERING_URL")`.

**Verification:**
```bash
./scripts/cargo-slot.sh build -p bom-rs
./scripts/cargo-slot.sh test -p bom-rs
curl http://localhost:8120/api/health
curl http://localhost:8120/api/openapi.json | jq .info.title
```

---

### 6. consolidation

| Attribute | Current State |
|-----------|--------------|
| Version | 2.2.3 |
| Port | 8105 |
| Bus init | No |
| Publisher | No |
| Consumers | None |
| Migrations | Yes (inline `sqlx::migrate!`) |
| OpenAPI | No (not present — must be added) |
| Custom state | `pool + metrics + gl_base_url` |
| Special middleware | `optional_claims_mw` (permissive JWT — SDK default is permissive) |
| main.rs LOC | 149 |

**Conversion notes:** Custom state includes `gl_base_url` — read from env
inside routes closure. Uses `optional_claims_mw` which aligns with the SDK's
permissive JWT behavior. No bus config needed.

**Verification:**
```bash
./scripts/cargo-slot.sh build -p consolidation
./scripts/cargo-slot.sh test -p consolidation
curl http://localhost:8105/api/health
```

---

## Group B — Publisher-Only (5 modules)

These modules initialize an event bus and spawn an outbox publisher task but have
no event consumers. The SDK handles this via `[bus]` and `[events.publish]` in
`module.toml`. No `.consumer()` calls needed.

### 7. numbering

| Attribute | Current State |
|-----------|--------------|
| Version | 2.1.2 |
| Port | 8096 |
| Bus init | Yes (Nats/InMemory enum) |
| Publisher | Yes (outbox::run_publisher_task) |
| Consumers | None |
| Migrations | Yes |
| OpenAPI | Yes (utoipa, via `http::ApiDoc`) |
| Custom state | `pool + metrics` (standard) |
| Special middleware | `RequirePermissionsLayer(NUMBERING_ALLOCATE)` — inside routes |
| main.rs LOC | 198 |

**Conversion notes:** Clean Group B. Permission layer is applied in route
definitions, not as a global middleware — survives SDK conversion. Route layout
is inline (not delegated to `http::router()`) — move into routes closure.

**module.toml needs:** `[bus] type = "nats"`, `[events.publish] outbox_table = "events_outbox"`

**Verification:**
```bash
./scripts/cargo-slot.sh build -p numbering
./scripts/cargo-slot.sh test -p numbering
curl http://localhost:8096/api/health
curl http://localhost:8096/api/openapi.json | jq .info.title
```

---

### 8. pdf-editor

| Attribute | Current State |
|-----------|--------------|
| Version | 2.1.3 |
| Port | 8121 |
| Bus init | Yes (Nats/InMemory) |
| Publisher | Yes (event_bus::start_outbox_publisher) |
| Consumers | None |
| Migrations | Yes |
| OpenAPI | Yes (utoipa, via `handlers::ApiDoc`) |
| Custom state | `pool` (uses db directly, no separate AppState) |
| Special middleware | 50 MB body limit on `/api/pdf/render-annotations` route |
| main.rs LOC | 249 |

**Conversion notes:** The 50 MB body limit for PDF upload routes is applied as
a per-route layer inside the router — this survives SDK conversion. The SDK sets
the default 2 MiB limit globally; the nested router overrides it for the PDF
endpoint. Build route tree inside routes closure.

**module.toml needs:** `[bus] type = "nats"`, `[events.publish] outbox_table = "events_outbox"`

**Verification:**
```bash
./scripts/cargo-slot.sh build -p pdf-editor-rs
./scripts/cargo-slot.sh test -p pdf-editor-rs
curl http://localhost:8121/api/health
curl http://localhost:8121/api/openapi.json | jq .info.title
```

---

### 9. workflow

| Attribute | Current State |
|-----------|--------------|
| Version | 2.1.2 |
| Port | 8107 |
| Bus init | Yes (Nats/InMemory) |
| Publisher | Yes (outbox::run_publisher_task) |
| Consumers | None |
| Migrations | Yes |
| OpenAPI | Yes (via `http::openapi_json`) |
| Custom state | `pool + metrics` (standard) |
| Special middleware | None |
| main.rs LOC | 217 |

**Conversion notes:** Clean Group B. Durable execution engine, state machines,
and escalation logic live in domain layer — unaffected by SDK conversion.

**module.toml needs:** `[bus] type = "nats"`, `[events.publish] outbox_table = "events_outbox"`

**Verification:**
```bash
./scripts/cargo-slot.sh build -p workflow
./scripts/cargo-slot.sh test -p workflow
curl http://localhost:8107/api/health
curl http://localhost:8107/api/openapi.json | jq .info.title
```

---

### 10. ap

| Attribute | Current State |
|-----------|--------------|
| Version | 2.1.3 |
| Port | 8093 |
| Bus init | Yes (Nats/InMemory) |
| Publisher | Yes (outbox::run_publisher_task) |
| Consumers | None wired (consumer code may exist in crate but not started in main.rs) |
| Migrations | Yes |
| OpenAPI | Yes (utoipa, in main.rs) |
| Custom state | `pool + metrics` (standard) |
| Special middleware | None |
| main.rs LOC | 305 |

**Conversion notes:** Consumer code for `inventory_item_received` may exist in
the crate but is NOT started in main.rs. Decision for batch conversion: either
wire it via `.consumer()` or leave it unwired and document. Large route surface
(33 handlers) but all inline — move into routes closure.

**module.toml needs:** `[bus] type = "nats"`, `[events.publish] outbox_table = "events_outbox"`

**Verification:**
```bash
./scripts/cargo-slot.sh build -p ap
./scripts/cargo-slot.sh test -p ap
curl http://localhost:8093/api/health
curl http://localhost:8093/api/openapi.json | jq .info.title
```

---

### 11. treasury

| Attribute | Current State |
|-----------|--------------|
| Version | 2.1.3 |
| Port | 8094 |
| Bus init | Yes (Nats/InMemory) |
| Publisher | Yes (outbox::run_publisher_task) |
| Consumers | None wired (wave2 plan mentions 2 reconciliation consumers but not started in main.rs) |
| Migrations | Yes |
| OpenAPI | Yes (via `http::openapi_json`) |
| Custom state | `pool + metrics` (standard) |
| Special middleware | `metrics::latency_layer` on read routes (custom axum middleware) |
| main.rs LOC | 268 |

**Conversion notes:** Custom `latency_layer` middleware is applied to treasury
read routes only — this is a per-route-group layer, survives SDK conversion.
Apply it inside the routes closure. `rust_decimal::Decimal` for all financial
math — no change needed, lives in domain layer.

**module.toml needs:** `[bus] type = "nats"`, `[events.publish] outbox_table = "events_outbox"`

**Verification:**
```bash
./scripts/cargo-slot.sh build -p treasury
./scripts/cargo-slot.sh test -p treasury
curl http://localhost:8094/api/health
curl http://localhost:8094/api/openapi.json | jq .info.title
```

---

## Group C — Standard Consumers (4 modules)

These modules have 1-2 event consumers that need adapting to the SDK consumer
handler signature: `async fn(ModuleContext, EventEnvelope<Value>) -> Result<(), ConsumerError>`.

### 12. fixed-assets

| Attribute | Current State |
|-----------|--------------|
| Version | 2.1.3 |
| Port | 8104 |
| Bus init | Yes (Nats/InMemory) |
| Publisher | Yes (outbox::run_publisher_task) |
| Consumers | 1: `ap_bill_approved` (asset capitalization from AP) |
| Migrations | Yes |
| OpenAPI | Yes (via `http::ApiDoc`) |
| Custom state | `pool + metrics` (standard) |
| Special middleware | None |
| main.rs LOC | 275 |

**Conversion notes:** Single consumer, standard pattern. Adapt
`start_ap_bill_approved_consumer` to SDK signature. Follow AR consumer as
reference implementation.

**module.toml needs:** `[bus] type = "nats"`, `[events.publish] outbox_table = "events_outbox"`

**Verification:**
```bash
./scripts/cargo-slot.sh build -p fixed-assets
./scripts/cargo-slot.sh test -p fixed-assets
curl http://localhost:8104/api/health
curl http://localhost:8104/api/openapi.json | jq .info.title
```

---

### 13. shipping-receiving

| Attribute | Current State |
|-----------|--------------|
| Version | 2.2.3 |
| Port | 8103 |
| Bus init | Yes (Nats/InMemory) |
| Publisher | Yes (outbox::run_publisher_task) |
| Consumers | 2: `po_approved`, `so_released` |
| Migrations | Yes |
| OpenAPI | Yes (utoipa, in main.rs) |
| Custom state | `pool + metrics + inventory` (InventoryIntegration) |
| Special middleware | None |
| main.rs LOC | 318 |

**Conversion notes:** Two consumers, each a separate `.consumer()` call.
Custom state has `InventoryIntegration` (HTTP or deterministic mode based on
`INVENTORY_URL` env var) — build inside routes closure. Read
`INVENTORY_URL` from env in the closure.

**module.toml needs:** `[bus] type = "nats"`, `[events.publish] outbox_table = "events_outbox"`

**Verification:**
```bash
./scripts/cargo-slot.sh build -p shipping-receiving-rs
./scripts/cargo-slot.sh test -p shipping-receiving-rs
curl http://localhost:8103/api/health
curl http://localhost:8103/api/openapi.json | jq .info.title
```

---

### 14. subscriptions

| Attribute | Current State |
|-----------|--------------|
| Version | 2.2.5 |
| Port | 8087 |
| Bus init | Yes (Nats/InMemory) |
| Publisher | Yes (publisher::run_publisher) |
| Consumers | 1: `ar.invoice_suspended` (hand-rolled, NOT using SDK consumer pattern) |
| Migrations | Yes |
| OpenAPI | Yes (via `http::ApiDoc`) |
| Custom state | `pool + metrics` (standard) |
| Special middleware | None |
| main.rs LOC | 337 |

**Conversion notes:** The `ar.invoice_suspended` consumer is hand-rolled
inline in main.rs (~85 lines of manual subscribe/parse/dispatch/DLQ logic).
Must be rewritten as an SDK consumer handler. The business logic is in
`consumer::handle_invoice_suspended()` — the SDK adapter wraps this.
Includes inline DLQ insertion — the SDK does NOT send to DLQ after retry
exhaustion (known gap from AR conversion).

**module.toml needs:** `[bus] type = "nats"`, `[events.publish] outbox_table = "events_outbox"`

**Verification:**
```bash
./scripts/cargo-slot.sh build -p subscriptions
./scripts/cargo-slot.sh test -p subscriptions
curl http://localhost:8087/api/health
curl http://localhost:8087/api/openapi.json | jq .info.title
```

---

### 15. maintenance

| Attribute | Current State |
|-----------|--------------|
| Version | 2.1.1 |
| Port | 8101 |
| Bus init | Yes (Nats/InMemory) |
| Publisher | Yes (outbox::run_publisher_task) |
| Consumers | 2: `production_workcenter_bridge`, `production_downtime_bridge` |
| Migrations | Yes |
| OpenAPI | Yes (utoipa, in main.rs) |
| Custom state | `pool + metrics` (standard) |
| Special middleware | None |
| Background tasks | Scheduler polling task (`run_scheduler_task`, interval from config) |
| main.rs LOC | 420 |

**Conversion notes:** Two consumers (adapt to SDK signature) plus a background
scheduler task. The SDK v1.0 has no `.on_startup()` hook. The scheduler must
be spawned inside the `.routes()` closure or from the routes return. Since
`.routes()` returns a `Router`, the scheduler `tokio::spawn` should happen
inside the closure before returning the router. The `scheduler_interval_secs`
config value needs to be read from env inside the closure.

**module.toml needs:** `[bus] type = "nats"`, `[events.publish] outbox_table = "events_outbox"`

**Verification:**
```bash
./scripts/cargo-slot.sh build -p maintenance-rs
./scripts/cargo-slot.sh test -p maintenance-rs
curl http://localhost:8101/api/health
curl http://localhost:8101/api/openapi.json | jq .info.title
```

---

## Group D — Heavy Consumers (3 modules)

Multiple consumers, complex bus patterns, or non-standard config that requires
careful adaptation beyond simple `.consumer()` wiring.

### 16. payments

| Attribute | Current State |
|-----------|--------------|
| Version | 2.1.5 |
| Port | 8088 |
| Bus init | Yes (Nats/InMemory) |
| Publisher | Yes (events::outbox::start_outbox_publisher) |
| Consumers | 1: `start_payment_collection_consumer` |
| Migrations | Yes |
| OpenAPI | Yes (utoipa, in main.rs) |
| Custom state | `pool + tilled_api_key + tilled_account_id + tilled_webhook_secret + tilled_webhook_secret_prev` |
| Special middleware | None |
| Custom metrics | Complex metrics handler combining prometheus_client + prometheus crate registries |
| main.rs LOC | 390 |

**Conversion notes:** The consumer is straightforward to convert. The complexity
is in the custom AppState (Tilled payment provider config — 4 fields from env)
and the custom `/metrics` handler that merges two metric registries (prometheus_client
for outbox depth + standard prometheus for projections and SLOs). The SDK provides
its own `/metrics` endpoint — this module needs to either merge its metrics into
the SDK's registry or override the `/metrics` route in the routes closure.
Webhook endpoint (`/api/payments/webhook/tilled`) must remain unauthenticated
(signature verification only) — route it outside the permission layer.

**module.toml needs:** `[bus] type = "nats"`, `[events.publish] outbox_table = "events_outbox"`

**Verification:**
```bash
./scripts/cargo-slot.sh build -p payments-rs
./scripts/cargo-slot.sh test -p payments-rs
curl http://localhost:8088/api/health
curl http://localhost:8088/api/openapi.json | jq .info.title
```

---

### 17. notifications

| Attribute | Current State |
|-----------|--------------|
| Version | 2.1.2 |
| Port | 8089 |
| Bus init | Yes (Nats/InMemory) |
| Publisher | Yes (event_bus::start_outbox_publisher) |
| Consumers | 3: `invoice_issued`, `payment_succeeded`, `payment_failed` |
| Migrations | Yes |
| OpenAPI | No (not present — must be added) |
| Custom state | `pool` (passed as db clone, no separate AppState struct in main.rs) |
| Special middleware | None |
| Background tasks | Dispatch loop (interval-based), orphaned claims recovery on startup |
| main.rs LOC | 351 |

**Conversion notes:** Three consumers need SDK adapter. Two background concerns:
(1) dispatch loop runs every N seconds (configurable via env var) — spawn inside
routes closure. (2) Orphaned claims recovery runs once at startup — also in routes
closure before returning router. Complex config: email/SMS sender types, retry
policies, HTTP endpoint config. These are all read from env and used to construct
senders — do this inside the routes closure. No OpenAPI — add during conversion
or in a separate bead.

**module.toml needs:** `[bus] type = "nats"`, `[events.publish] outbox_table = "events_outbox"`

**Verification:**
```bash
./scripts/cargo-slot.sh build -p notifications-rs
./scripts/cargo-slot.sh test -p notifications-rs
curl http://localhost:8089/api/health
```

---

### 18. gl

| Attribute | Current State |
|-----------|--------------|
| Version | 2.1.7 |
| Port | 8090 |
| Bus init | Yes (string-based bus_type: `"inmemory"` or `"nats"`, NOT enum) |
| Publisher | No explicit outbox publisher in main.rs |
| Consumers | **11 consumers**: gl_posting, gl_reversal, gl_writeoff, gl_inventory, ar_tax_committed, ar_tax_voided, fixed_assets_depreciation, gl_credit_note, ap_vendor_bill_approved, gl_fx_realized, timekeeping_labor_cost |
| Migrations | Yes |
| OpenAPI | Yes (utoipa, in main.rs) |
| Custom state | `pool + dlq_validation_enabled + metrics` |
| Special middleware | None |
| main.rs LOC | 420 |

**Conversion notes:** The largest consumer surface in the platform (11 consumers).
Each needs an SDK consumer adapter. The bus type is parsed from a raw string
(`config.bus_type.to_lowercase()`) with a `panic!()` on invalid values — the SDK's
module.toml `[bus] type = "nats"` replaces this. No outbox publisher is visible in
main.rs — GL may not publish events (it consumes them from other modules). Verify
whether `[events.publish]` is needed. Custom state field `dlq_validation_enabled`
is a bool from config — read from env inside routes closure.

**Gotcha:** GL has the most consumers. Converting all 11 is mechanical but large.
Consider splitting into a dedicated bead or having two agents pair on it.

**module.toml needs:** `[bus] type = "nats"` (no `[events.publish]` unless outbox publisher is confirmed)

**Verification:**
```bash
./scripts/cargo-slot.sh build -p gl-rs
./scripts/cargo-slot.sh test -p gl-rs
curl http://localhost:8090/api/health
curl http://localhost:8090/api/openapi.json | jq .info.title
```

---

## Group E — Non-Standard Patterns (4 modules)

These modules have patterns that diverge from the SDK's assumptions and need
extra adaptation or SDK extensions.

### 19. customer-portal

| Attribute | Current State |
|-----------|--------------|
| Version | 2.1.1 |
| Port | 8111 |
| Bus init | No |
| Publisher | No |
| Consumers | None |
| Migrations | Yes |
| OpenAPI | No (not present — must be added) |
| Custom state | `pool + metrics + portal_jwt + config` |
| Special middleware | None (uses its own auth, NOT platform JWT) |
| main.rs LOC | 89 |

**Conversion notes:** Customer-portal uses its own RS256 JWT system (`PortalJwt`
with private + public key pair) for customer-facing auth. It does NOT use the
platform `JwtVerifier`. The SDK auto-wires `JwtVerifier::from_env_with_overlap()`
which returns `None` if `JWT_PUBLIC_KEY` is not set — so it won't interfere. But
the SDK's `ClaimsLayer` middleware (permissive mode) will still run and find no
token, which is fine. The real concern: customer-portal passes `config` directly
into `AppState` (Argon2 params, refresh token config, doc-mgmt URL). All of this
must be read from env inside the routes closure and built into the custom AppState.
No OpenAPI — add during conversion or separate bead.

**module.toml needs:** No `[bus]` section.

**Verification:**
```bash
./scripts/cargo-slot.sh build -p customer-portal
./scripts/cargo-slot.sh test -p customer-portal
curl http://localhost:8111/api/health
```

---

### 20. quality-inspection

| Attribute | Current State |
|-----------|--------------|
| Version | 2.0.2 |
| Port | 8106 |
| Bus init | Yes (Nats with graceful degradation to InMemory on failure) |
| Publisher | No outbox publisher |
| Consumers | 2: `receipt_event_bridge`, `production_event_bridge` |
| Migrations | Yes |
| OpenAPI | Yes (via `http::openapi_json`) |
| Custom state | `pool + wc_pool + metrics` (DUAL DATABASE POOL) |
| Special middleware | None |
| main.rs LOC | 263 |

**Conversion notes:** The SDK creates one database pool from `DATABASE_URL`. This
module needs a SECOND pool for `WORKFORCE_COMPETENCE_DATABASE_URL`. Create the
second pool inside the routes closure: `PgPoolOptions::new().connect(&env("WORKFORCE_COMPETENCE_DATABASE_URL"))`.
The bus graceful degradation (NATS failure falls back to InMemory) is similar to
the SDK's behavior — verify the SDK does the same. If not, this needs an SDK
enhancement or a pre-bus-init hook. No outbox publisher — module consumes only.
`ConfigValidator` should validate both `DATABASE_URL` and
`WORKFORCE_COMPETENCE_DATABASE_URL`.

**module.toml needs:** `[bus] type = "nats"` (no `[events.publish]`)

**Verification:**
```bash
./scripts/cargo-slot.sh build -p quality-inspection-rs
./scripts/cargo-slot.sh test -p quality-inspection-rs
curl http://localhost:8106/api/health
curl http://localhost:8106/api/openapi.json | jq .info.title
```

---

### 21. integrations

| Attribute | Current State |
|-----------|--------------|
| Version | 2.3.0 |
| Port | 8099 |
| Bus init | Yes (Nats/InMemory) |
| Publisher | Yes (outbox::run_publisher_task) |
| Consumers | None |
| Migrations | Yes |
| OpenAPI | Yes (utoipa, in main.rs) |
| Custom state | `pool + metrics + bus` (bus reference in AppState) |
| Special middleware | None |
| Background tasks | OAuth token refresh worker (30s, conditional on QBO_CLIENT_ID), CDC polling worker (15m, conditional) |
| main.rs LOC | 293 |

**Conversion notes:** Two conditional background workers that only start when
`QBO_CLIENT_ID` is set. These must be spawned inside the routes closure. The
`bus` field in AppState is used for publishing from HTTP handlers — the SDK
provides `ctx.bus()` which can be used instead (eliminating the need to store
bus in AppState). The `shutdown_rx` channel for the OAuth worker currently does
nothing useful (no graceful stop) — simplify in conversion.

**module.toml needs:** `[bus] type = "nats"`, `[events.publish] outbox_table = "events_outbox"`

**Verification:**
```bash
./scripts/cargo-slot.sh build -p integrations-rs
./scripts/cargo-slot.sh test -p integrations-rs
curl http://localhost:8099/api/health
curl http://localhost:8099/api/openapi.json | jq .info.title
```

---

### 22. inventory

| Attribute | Current State |
|-----------|--------------|
| Version | 2.4.6 |
| Port | 8092 |
| Bus init | Yes (starts InMemoryBus, then supervisor upgrades to NATS) |
| Publisher | Yes (via event_bus_supervisor) |
| Consumers | 2: `component_issue_consumer`, `fg_receipt_consumer` (spawned inside supervisor) |
| Migrations | Yes |
| OpenAPI | Yes (utoipa, in main.rs) |
| Custom state | `pool + metrics + event_bus + bus_health` (BusHealth tracking) |
| Special middleware | None |
| main.rs LOC | 724 |

**Conversion notes:** The most complex module. The `start_event_bus_supervisor`
pattern creates an InMemoryBus immediately, then spawns a supervisor task that
connects to NATS, replaces the bus reference, and starts consumers. This is
fundamentally different from the SDK's startup sequence (which creates the bus
during Phase A and consumers during Phase B).

**Options:**
1. Replace the supervisor with the SDK's standard bus init. This means the SDK
   connects to NATS at startup and fails if NATS is unavailable (unless the SDK
   has graceful degradation). Consumers are registered with `.consumer()`.
   BusHealth state is replaced by the SDK's built-in `/api/ready` health check.
2. Keep the supervisor and pass it as a background task inside routes closure.
   This preserves the hot-reconnect behavior but loses the SDK's consumer
   management.

**Recommendation:** Option 1 (standard SDK pattern). The supervisor pattern was
invented before the SDK existed. The SDK's bus init + consumer registration is
the proven replacement. If NATS degradation is needed, add it to the SDK (v1.1).

**module.toml needs:** `[bus] type = "nats"`, `[events.publish] outbox_table = "events_outbox"`

**Verification:**
```bash
./scripts/cargo-slot.sh build -p inventory-rs
./scripts/cargo-slot.sh test -p inventory-rs
curl http://localhost:8092/api/health
curl http://localhost:8092/api/openapi.json | jq .info.title
```

---

## Summary Table

| # | Module | Group | Version | Port | Bus | Publisher | Consumers | OpenAPI | Gotchas |
|---|--------|-------|---------|------|-----|-----------|-----------|---------|---------|
| 1 | reporting | A | 2.1.0 | 8097 | No | No | 0 | Yes | — |
| 2 | timekeeping | A | 2.1.4 | 8102 | No | No | 0 | Yes | Dead event code |
| 3 | ttp | A | 2.1.11 | 8100 | No | No | 0 | No | Dead event code, no OpenAPI |
| 4 | workforce-competence | A | 2.1.4 | 8110 | No | No | 0 | No | No OpenAPI |
| 5 | bom | A | 2.2.1 | 8120 | No | No | 0 | Yes | NumberingClient in state |
| 6 | consolidation | A | 2.2.3 | 8105 | No | No | 0 | No | gl_base_url in state, no OpenAPI |
| 7 | numbering | B | 2.1.2 | 8096 | Yes | Yes | 0 | Yes | — |
| 8 | pdf-editor | B | 2.1.3 | 8121 | Yes | Yes | 0 | Yes | 50 MB body limit on PDF route |
| 9 | workflow | B | 2.1.2 | 8107 | Yes | Yes | 0 | Yes | — |
| 10 | ap | B | 2.1.3 | 8093 | Yes | Yes | 0 | Yes | Unwired consumer code exists |
| 11 | treasury | B | 2.1.3 | 8094 | Yes | Yes | 0 | Yes | Custom latency_layer |
| 12 | fixed-assets | C | 2.1.3 | 8104 | Yes | Yes | 1 | Yes | — |
| 13 | shipping-receiving | C | 2.2.3 | 8103 | Yes | Yes | 2 | Yes | InventoryIntegration in state |
| 14 | subscriptions | C | 2.2.5 | 8087 | Yes | Yes | 1 | Yes | Hand-rolled consumer needs rewrite |
| 15 | maintenance | C | 2.1.1 | 8101 | Yes | Yes | 2 | Yes | Scheduler background task |
| 16 | payments | D | 2.1.5 | 8088 | Yes | Yes | 1 | Yes | Custom metrics, Tilled state |
| 17 | notifications | D | 2.1.2 | 8089 | Yes | Yes | 3 | No | Dispatch loop, no OpenAPI |
| 18 | gl | D | 2.1.7 | 8090 | Yes | No | 11 | Yes | String bus_type, 11 consumers |
| 19 | customer-portal | E | 2.1.1 | 8111 | No | No | 0 | No | Own JWT system, no OpenAPI |
| 20 | quality-inspection | E | 2.0.2 | 8106 | Yes | No | 2 | Yes | Dual DB pool |
| 21 | integrations | E | 2.3.0 | 8099 | Yes | Yes | 0 | Yes | Background workers (conditional) |
| 22 | inventory | E | 2.4.6 | 8092 | Yes | Yes | 2 | Yes | Bus supervisor pattern (724 LOC) |

---

## Conversion Order Recommendation

1. **Group A first** (6 modules) — mechanical, validates module.toml generation
2. **Group B next** (5 modules) — validates SDK bus init from module.toml
3. **Group C** (4 modules) — validates `.consumer()` adapter pattern
4. **Group D** (3 modules) — needs careful handling, one at a time
5. **Group E last** (4 modules) — may surface SDK v1.1 needs

All Group A modules can convert in parallel. All Group B can convert in parallel
after A validates the pattern. Group C-E should have at least one reference
conversion before parallelizing.

**Total: 22 modules, 22 conversion beads minimum** (plus child beads for
discovered issues).
