# What a Vertical Must Do to Plug Into the Platform

This is the complete list. Everything a vertical developer writes, configures, or needs to know — organized by what the SDK handles vs what you're on your own for.

---

## What the SDK Handles (Zero Code From You)

When you call `ModuleBuilder::from_manifest("module.toml").run().await`, the SDK gives you:

- Database connection pool (configured from manifest [database])
- Migration execution on startup (if auto_migrate = true)
- NATS event bus connection (if [bus] type = "nats")
- JWT verification with JWKS refresh and key rotation
- Rate limiting middleware (token bucket, tenant-aware, 429 on violation)
- CORS (manifest-driven with regex support, env var fallback)
- Request body size limit (manifest [server] body_limit)
- Request timeout (manifest [server] request_timeout)
- Health endpoints: /healthz, /api/health, /api/ready (with optional NATS probe)
- Metrics endpoint: /metrics (Prometheus)
- Version endpoint: /api/version
- Graceful shutdown
- Outbox publisher (polls outbox table, publishes to NATS)
- Consumer subscription with retry and exponential backoff
- Tracing context propagation (correlation_id through HTTP and NATS)

You don't write middleware for any of this. You don't wire health checks. You don't build a NATS publisher. It's config.

---

## What You Write

### 1. module.toml (~25 lines)

```toml
[module]
name = "your-module"
version = "0.1.0"
description = "What this module does"

[server]
host = "0.0.0.0"
port = 8150
# body_limit = "2mb"      # optional, default 2mb
# request_timeout = "30s"  # optional, default 30s

[database]
migrations = "./db/migrations"
auto_migrate = true
# pool_min = 2             # optional
# pool_max = 10            # optional

[bus]
type = "nats"

[events.publish]
outbox_table = "events_outbox"

[sdk]
min_version = "0.20.0"
```

That's the operational contract. The SDK reads it and configures everything.

### 2. main.rs (~30-90 lines)

Simplest possible module (no events, just HTTP):

```rust
use platform_sdk::ModuleBuilder;

mod http;

#[tokio::main]
async fn main() {
    ModuleBuilder::from_manifest("module.toml")
        .migrator(&sqlx::migrate!("./db/migrations"))
        .routes(http::routes)
        .run()
        .await;
}
```

Module with event consumers:

```rust
use platform_sdk::{ModuleBuilder, ConsumerDef};

mod http;
mod consumers;

#[tokio::main]
async fn main() {
    ModuleBuilder::from_manifest("module.toml")
        .migrator(&sqlx::migrate!("./db/migrations"))
        .consumer("payments.events.payment.succeeded", consumers::on_payment_succeeded)
        .routes(http::routes)
        .run()
        .await;
}
```

That's it for main.rs. No lifecycle code. No middleware setup. No health endpoints.

### 3. HTTP Handlers

Every handler follows this pattern:

```rust
use axum::{extract::State, Extension, Json};
use platform_http_contracts::{ApiError, PaginatedResponse};
use platform_security::claims::VerifiedClaims;

pub async fn list_orders(
    State(ctx): State<ModuleContext>,
    Extension(claims): Extension<VerifiedClaims>,
) -> Result<Json<PaginatedResponse<Order>>, ApiError> {
    let tenant_id = claims.tenant_id;
    let pool = ctx.pool();

    let orders = sqlx::query_as!(
        Order,
        "SELECT * FROM orders WHERE tenant_id = $1 ORDER BY created_at DESC LIMIT 50",
        tenant_id
    )
    .fetch_all(pool)
    .await
    .map_err(ApiError::from)?;

    let total = orders.len() as i64;
    Ok(Json(PaginatedResponse::new(orders, 1, total, total)))
}
```

Key points:
- `VerifiedClaims` comes from the JWT middleware the SDK already set up
- `tenant_id` MUST come from claims, NEVER from the request body
- Every SQL query MUST include `WHERE tenant_id = $1`
- Return `ApiError` for errors, `PaginatedResponse<T>` for lists
- The SDK's `TenantId` extractor is also available as a shortcut

### 4. Route Wiring

```rust
use axum::Router;
use platform_sdk::ModuleContext;
use platform_security::permissions;

pub fn routes(ctx: &ModuleContext) -> Router<ModuleContext> {
    Router::new()
        // Mutation endpoints — protected by permission layer
        .route("/api/yourmod/orders", post(create_order))
        .route("/api/yourmod/orders/{id}", put(update_order))
        .route("/api/yourmod/orders/{id}", delete(delete_order))
        .route_layer(RequirePermissionsLayer::new(&["yourmod.mutate"]))
        // Read endpoints — below the mutation layer, still require auth
        .route("/api/yourmod/orders", get(list_orders))
        .route("/api/yourmod/orders/{id}", get(get_order))
}
```

**CRITICAL:** `.route_layer()` only protects routes defined ABOVE it. If you put a mutation route below the layer, it's unprotected. This has caused real security bugs.

### 5. Event Publishing (Outbox Pattern)

To publish an event, insert into your outbox table inside the same transaction as your business write:

```rust
use platform_event_bus::envelope::{EventEnvelope, validate_and_serialize_envelope};

async fn create_order(/* ... */) -> Result<Json<Order>, ApiError> {
    let mut tx = pool.begin().await?;

    // Business write
    let order = sqlx::query_as!(Order, "INSERT INTO orders ...")
        .fetch_one(&mut *tx)
        .await?;

    // Event publish — same transaction
    let envelope = EventEnvelope::new(
        "your-module",
        "order.created",
        claims.tenant_id,
        serde_json::to_value(&order)?,
    );
    let payload = validate_and_serialize_envelope(&envelope)?;

    sqlx::query!(
        "INSERT INTO events_outbox (event_type, payload, tenant_id)
         VALUES ($1, $2, $3)",
        "yourmod.events.order.created",
        payload,
        claims.tenant_id,
    )
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;
    Ok(Json(order))
}
```

The SDK's outbox publisher picks this up and publishes to NATS automatically. You don't write a publisher.

**Subject naming convention:** `event_type` is used directly as the NATS subject. Store the full subject path in `event_type` (e.g., `yourmod.events.order.created`). The SDK publisher does NOT add any prefix — it publishes to whatever value is in the `event_type` column.

### 6. Event Consuming

The consumer handler receives an EventEnvelope:

```rust
use platform_sdk::ModuleContext;
use platform_event_bus::envelope::EventEnvelope;
use platform_sdk::consumer::ConsumerError;

pub async fn on_payment_succeeded(
    ctx: ModuleContext,
    envelope: EventEnvelope<serde_json::Value>,
) -> Result<(), ConsumerError> {
    let pool = ctx.pool();
    let tenant_id = envelope.tenant_id;

    // Idempotency check — skip if already processed
    let already = sqlx::query_scalar!(
        "SELECT EXISTS(SELECT 1 FROM processed_events WHERE event_id = $1)",
        envelope.event_id
    )
    .fetch_one(pool)
    .await
    .map_err(|e| ConsumerError::Processing(e.to_string()))?;

    if already.unwrap_or(false) {
        return Ok(());
    }

    // Your business logic here
    // ...

    // Mark as processed
    sqlx::query!(
        "INSERT INTO processed_events (event_id) VALUES ($1) ON CONFLICT DO NOTHING",
        envelope.event_id
    )
    .execute(pool)
    .await
    .map_err(|e| ConsumerError::Processing(e.to_string()))?;

    Ok(())
}
```

The SDK handles subscription, retry with exponential backoff (100ms → 30s, 3 attempts), and ack/nack.

### 7. Calling Other Modules

Use generated typed clients:

```rust
use platform_client_party::PartiesClient;
use platform_sdk::http_client::PlatformClient;

// Construct once at startup or per-request
let platform_client = PlatformClient::new(
    std::env::var("PARTY_BASE_URL").unwrap()
).with_bearer_token(service_token);

let party_client = PartiesClient::new(platform_client);

// Call with per-request claims
let party = party_client.get_party(party_id, &claims).await?;
```

The generated client handles: tenant headers (x-tenant-id, x-correlation-id, x-app-id), retry on 429/503, and correct response deserialization.

### 8. Database Migrations

SQL files in `db/migrations/`:

```sql
-- 20260401000001_create_orders.sql
CREATE TABLE orders (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id UUID NOT NULL,          -- MANDATORY on every table
    order_name TEXT NOT NULL,
    total NUMERIC(12,2) NOT NULL DEFAULT 0,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX idx_orders_tenant ON orders (tenant_id);
```

And the outbox table if you publish events:

```sql
-- 20260401000002_create_outbox.sql
CREATE TABLE events_outbox (
    id BIGSERIAL PRIMARY KEY,
    event_type TEXT NOT NULL,
    payload JSONB NOT NULL,
    tenant_id UUID NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    published_at TIMESTAMPTZ
);

CREATE INDEX idx_outbox_unpublished ON events_outbox (created_at)
    WHERE published_at IS NULL;
```

And the processed events table if you consume events:

```sql
-- 20260401000003_create_processed_events.sql
CREATE TABLE processed_events (
    event_id UUID PRIMARY KEY,
    processed_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
```

### 9. OpenAPI Annotations

Every handler needs a utoipa annotation:

```rust
#[utoipa::path(
    get,
    path = "/api/yourmod/orders",
    responses(
        (status = 200, body = PaginatedResponse<Order>),
        (status = 401, body = ApiError),
    ),
    security(("bearer" = []))
)]
pub async fn list_orders(/* ... */) -> Result<Json<PaginatedResponse<Order>>, ApiError> {
    // ...
}
```

And an openapi_dump binary:

```rust
// src/bin/openapi_dump.rs
use utoipa::OpenApi;

#[derive(OpenApi)]
#[openapi(
    info(title = "Your Module", version = "0.1.0"),
    paths(
        crate::http::list_orders,
        crate::http::get_order,
        crate::http::create_order,
    ),
    components(schemas(Order, CreateOrderRequest)),
    security(("bearer" = []))
)]
struct ApiDoc;

fn main() {
    println!("{}", serde_json::to_string_pretty(&ApiDoc::openapi()).unwrap());
}
```

### 10. Cargo.toml Dependencies

```toml
[package]
name = "your-module-rs"
version = "0.1.0"
edition = "2021"

[dependencies]
platform-sdk = { path = "../../platform/platform-sdk" }
platform-http-contracts = { path = "../../platform/http-contracts" }
platform-security = { path = "../../platform/security" }
platform-event-bus = { path = "../../platform/event-bus" }
# Generated clients for modules you call:
platform-client-party = { path = "../../clients/party" }

axum = "0.8"
sqlx = { version = "0.8", features = ["runtime-tokio", "postgres", "uuid", "chrono"] }
tokio = { version = "1", features = ["full"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
uuid = { version = "1", features = ["v4", "serde"] }
chrono = { version = "0.4", features = ["serde"] }
utoipa = { version = "5", features = ["axum_extras", "uuid", "chrono"] }
tracing = "0.1"
```

---

## What You Must Configure (Not Code, But Must Set)

### Environment Variables

```bash
DATABASE_URL=postgres://user:pass@localhost:5432/yourmod
NATS_URL=nats://localhost:4222
JWT_PUBLIC_KEY=<PEM or base64 public key>
# Or: JWKS_URL=http://identity-auth:8080/.well-known/jwks.json
```

### Docker Compose (if deploying with the platform)

Add to `docker-compose.cross.yml`:
```yaml
your-module:
  image: 7d-runtime
  command: ["/usr/local/bin/your-module-rs"]
  volumes:
    - ./target/aarch64-unknown-linux-musl/debug/your-module-rs:/usr/local/bin/your-module-rs:ro
    - ./modules/your-module/module.toml:/app/module.toml:ro
    - ./modules/your-module/db/migrations:/app/db/migrations:ro
```

Add to `docker-compose.services.yml`:
```yaml
your-module:
  environment:
    DATABASE_URL: postgres://...
    NATS_URL: nats://nats:4222
    JWT_PUBLIC_KEY: ${JWT_PUBLIC_KEY}
```

---

## What Will Break You If You Don't Know It

These are real bugs that have happened in the platform. Each one cost someone time.

### 1. tenant_id from JWT, never from request body
Multiple modules had P0 security vulnerabilities where tenant_id was accepted from the client request instead of JWT claims. An attacker could access any tenant's data. Always use `claims.tenant_id`.

### 2. Every SQL query needs WHERE tenant_id = $1
If you forget the tenant filter, you return all tenants' data. There's no database-level RLS to catch this. The SDK provides the `TenantId` extractor but can't enforce your SQL.

### 3. .route_layer() only protects routes ABOVE it
Axum applies route layers top-down. If you define a mutation endpoint below `.route_layer(RequirePermissionsLayer)`, it's unprotected. This has caused unprotected mutation endpoints in production.

### 4. Event subject naming: event_type IS the NATS subject
The SDK publisher publishes directly to the `event_type` value — no prefix is added. If you store `"order.created"` as event_type, that's the NATS subject consumers must subscribe to. Convention is `{module}.events.{event_name}`, e.g., `"yourmod.events.order.created"`. Store the full path in `event_type`.

### 5. Outbox insert must be in the same transaction as the business write
If you insert the business row in one transaction and the outbox row in another, a crash between them means the event never publishes. Always use `pool.begin()` → business insert → outbox insert → `tx.commit()`.

### 6. Consumer idempotency is your responsibility
The SDK delivers events at-least-once. If your consumer isn't idempotent (checking processed_events before acting), you'll process the same event multiple times. This causes duplicate invoices, double charges, etc.

### 7. cargo-slot.sh, not cargo
Running `cargo build` directly will hang if another agent is compiling. Use `./scripts/cargo-slot.sh build -p your-module-rs`.

### 8. Bead workflow is mandatory
All work must be tracked with a bead. Edits are blocked by pre-commit hooks until you have an active bead. Run `./scripts/br-start-work.sh "Your task"` before writing code.

---

## What's NOT Available Yet (Gaps a Vertical Will Hit)

1. **No identity-auth generated Rust client.** If you need `register_user()`, you write the HTTP call yourself.
2. **No event catalog.** To discover what events exist and their payloads, you read module source code.
3. **No SDK test harness.** You build your own test setup (database, mock bus, claims injection).
4. **Inconsistent response formats.** Only 1 of 25 platform modules (BOM) returns fully standardized responses. Others use custom formats — your generated client will work, but you'll encounter inconsistencies.
5. **3 platform modules have dead event publishers** (GL, Notifications, PDF-Editor). If you subscribe to their events, nothing arrives.

---

## The Minimum File Set for a New Module

```
modules/your-module/
├── module.toml                    # ~25 lines
├── Cargo.toml                     # ~30 lines
├── db/
│   └── migrations/
│       ├── 20260401000001_create_tables.sql
│       ├── 20260401000002_create_outbox.sql        # if publishing events
│       └── 20260401000003_create_processed.sql     # if consuming events
├── src/
│   ├── main.rs                    # ~30-90 lines
│   ├── http/
│   │   ├── mod.rs                 # routes function
│   │   └── orders.rs              # handlers
│   ├── domain/                    # business logic (optional structure)
│   │   └── models.rs
│   ├── consumers/                 # event handlers (if consuming)
│   │   └── mod.rs
│   └── bin/
│       └── openapi_dump.rs        # ~30 lines
└── REVISIONS.md                   # if version >= 1.0.0
```

Total new code for a simple module: ~200-400 lines of Rust, ~50 lines of SQL, ~50 lines of config. The SDK handles everything else.
