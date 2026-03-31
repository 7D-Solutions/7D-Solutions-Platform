# platform-sdk

Module startup SDK for the 7D platform. Eliminates per-module boilerplate by
providing a single startup sequence that orchestrates the database pool, event
bus, JWT verification, rate limiting, CORS, health endpoints, and graceful
shutdown.

## v1.0 API Surface (Frozen)

After this version only **additive** changes are permitted. No existing method,
manifest key, or public type may be removed or have its signature changed.

Proven by three module conversions: **Party**, **Production**, **AR**.

### Builder

```rust
use platform_sdk::ModuleBuilder;

static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./db/migrations");

#[tokio::main]
async fn main() {
    ModuleBuilder::from_manifest("module.toml")
        .migrator(&MIGRATOR)
        .consumer("payments.events.payment.succeeded", on_payment_succeeded)
        .routes(|ctx| {
            axum::Router::new()
                // ... module routes ...
        })
        .run()
        .await
        .expect("module failed");
}
```

| Method | Purpose | Used by |
|--------|---------|---------|
| `from_manifest(path)` | Load module.toml configuration | Party, Production, AR |
| `migrator(&Migrator)` | Provide sqlx compile-time migrator | Party, Production, AR |
| `consumer(subject, handler)` | Register NATS event consumer with retry | AR |
| `routes(\|ctx\| Router)` | Register module-specific HTTP routes | Party, Production, AR |
| `run()` | Start server, block until shutdown | Party, Production, AR |

### ModuleContext

Passed to `routes` and `consumer` handlers.

| Method | Returns |
|--------|---------|
| `ctx.pool()` | `&PgPool` |
| `ctx.config()` | `&Manifest` |
| `ctx.bus()` | `Result<&dyn EventBus, BusNotAvailable>` |
| `ctx.require_permission(claims, perm)` | `Result<(), SecurityError>` |

### module.toml Manifest

```toml
[module]
name = "ar"                          # required
version = "2.3.0"                    # optional
description = "Invoicing and collections"  # optional

[server]
host = "0.0.0.0"                     # default: 0.0.0.0
port = 8086                          # default: 8080

[database]
migrations = "./db/migrations"       # path to sqlx migrations
auto_migrate = true                  # run migrations on startup

[bus]
type = "nats"                        # "nats" | "inmemory" | "none"

[events.publish]
outbox_table = "events_outbox"       # outbox table name for publisher

[sdk]
min_version = "0.1.0"               # minimum SDK version required
```

All keys above are proven by at least one conversion. Unknown keys produce a
warning but do not error.

### Consumer Retry Policy

Consumers use the event-bus default retry configuration:

- **3 attempts** maximum
- **Exponential backoff**: 100ms initial, 30s cap
- After exhausting retries: error is logged, message is skipped

The retry policy is not per-consumer configurable in v1.0. If a future
conversion proves the need, `consumer_with_retry(subject, handler, config)`
can be added in v1.1.

### Public Re-exports

```rust
pub use builder::ModuleBuilder;
pub use consumer::ConsumerError;
pub use context::{BusNotAvailable, ModuleContext};
pub use manifest::Manifest;
pub use startup::StartupError;

// Convenience re-exports from platform sub-crates
pub use event_bus::{EventBus, EventEnvelope};
pub use security::claims::VerifiedClaims;
pub use sqlx::PgPool;
```

### What Was Deliberately Excluded

These were evaluated against all three conversions and found unnecessary:

| Method | Reason excluded |
|--------|----------------|
| `.state<T>(val)` | No conversion needed custom state injection |
| `.layer(middleware)` | No conversion needed custom middleware |
| `.on_startup(hook)` | No conversion needed custom init hooks |
| `.open_api(spec)` | No conversion mounted OpenAPI from the builder |

Any of these can be added in v1.1+ when proven by a real module conversion.

### Startup Sequence

The SDK runs a two-phase startup:

**Phase A** (infrastructure): dotenv, tracing, database pool, migrations check,
event bus, outbox publisher, JWT verifier, rate limiter.

**Phase B** (HTTP): middleware stack, health routes (`/healthz`, `/api/health`,
`/api/ready`, `/api/version`), metrics (`/metrics`), module routes, TCP bind,
graceful shutdown with consumer drain.

### Grok Adversarial Review

Reviewed 2026-03-31. Verdict: "freeze-ready, admirably minimal and proven."
Grok recommended adding `.state()` and `.open_api()` pre-freeze. Rejected per
bead invariant: no method without proven conversion need. Deferred to v1.1.
