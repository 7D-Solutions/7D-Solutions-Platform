# TrashTech SDK Conversion Plan (bd-k4nvv)

Platform-side SDK changes are committed. This document describes the remaining
TrashTech-side work to complete the migration.

## Prerequisites

- platform-sdk v0.1.0 with `.state()`, `.on_startup()`, `.skip_default_middleware()` (done)
- Event-bus EventEnvelope format compatibility verified

## Files to Create / Modify

### 1. `tt-server/module.toml` (new)

```toml
[module]
name = "trashtech"
version = "0.1.0"
description = "TrashTech vertical -- waste management SaaS"

[server]
host = "0.0.0.0"
port = 3001

[bus]
type = "nats"

[sdk]
min_version = "0.1.0"
```

Note: No `[database]` section -- TrashTech manages its own migrations via
`tt_core::db::run_migrations` and per-tenant migration sweeps.

### 2. `tt-events/src/analytics_projector.rs` (modify)

Make three handler functions pub so they can be called from SDK consumer
closures:

```diff
- async fn handle_stop_started(pool: &PgPool, tenant_id: Uuid, p: &StopStartedPayload) {
+ pub async fn handle_stop_started(pool: &PgPool, tenant_id: Uuid, p: &StopStartedPayload) {

- async fn handle_stop_completed(pool: &PgPool, tenant_id: Uuid, p: &StopCompletedPayload) {
+ pub async fn handle_stop_completed(pool: &PgPool, tenant_id: Uuid, p: &StopCompletedPayload) {

- async fn handle_stop_skipped(pool: &PgPool, tenant_id: Uuid, p: &StopSkippedPayload) {
+ pub async fn handle_stop_skipped(pool: &PgPool, tenant_id: Uuid, p: &StopSkippedPayload) {
```

### 3. `tt-server/Cargo.toml` (modify)

Add platform-sdk and serde_json deps. Remove tracing-subscriber (SDK handles it):

```toml
[dependencies]
# ... existing deps ...
platform-sdk = { path = "/Users/james/Projects/7D-Solutions Platform/platform/platform-sdk" }
serde_json = "1"
serde = { version = "1", features = ["derive"] }
sqlx = { version = "0.8", features = ["runtime-tokio", "postgres", "uuid", "chrono"] }
```

Remove: `tracing-subscriber` (SDK initializes tracing in phase_a).

### 4. `tt-server/src/main.rs` (rewrite)

## Architecture Notes

### Consumer Wiring

6 of 7 consumers use EventEnvelope format and wire via `.consumer()`:

| # | Subject | Handler |
|---|---------|---------|
| 1 | `trashtech.events.stop.skipped` | Projector: upsert skipped_stop_queue + notification |
| 2 | `trashtech.events.stop.started` | Analytics: handle_stop_started |
| 3 | `trashtech.events.stop.completed` | Analytics: handle_stop_completed |
| 4 | `trashtech.events.stop.skipped` | Analytics: handle_stop_skipped |
| 5 | `payments.events.payment.succeeded` | apply_payment_succeeded |
| 6 | `auth.events.password_reset_requested` | handle_reset |

Consumer 7 (`tenant.provisioned`) uses **bare payload format** (not
EventEnvelope), published via the platform outbox publisher which serializes
raw payload JSON. The SDK consumer expects EventEnvelope format and would
skip these messages. Wire this consumer manually via `tokio::spawn` with the
raw NATS client.

### Skip Default Middleware

TrashTech has its own auth stack (`tt_api::auth` with `jsonwebtoken`), custom
CORS (trashtech.app domain matching), custom health endpoints
(`/api/health/live`, `/api/health/ready`), and Prometheus metrics. The SDK's
built-in middleware (security crate JWT, generic CORS, SDK health/metrics
routes) would conflict. Use `.skip_default_middleware()`.

### Custom State

`TrashTechState` struct injected via `.on_startup()`:

```rust
struct TrashTechState {
    resolver: tt_core::tenant::TenantResolver,
    provisioning: tt_core::provisioning::ProvisioningService,
    feature_flags: tt_core::config::FeatureFlags,
    phase3_config: tt_core::config::Phase3Config,
    nats_client: Option<async_nats::Client>,
    platform_client: tt_integrations::PlatformClient,
    media_url_policy: tt_core::config::MediaUrlPolicy,
    geocoding_client: tt_integrations::GeocodingClient,
    media_storage: Option<tt_integrations::MediaStorageClient>,
    payments_client: Option<tt_integrations::PaymentsClient>,
    sendgrid: Option<tt_api::admin_notifications::EmailTickConfig>,
    body_size_limits: tt_core::config::BodySizeLimits,
}
```

The `.on_startup()` callback receives the SDK's PgPool (connected to
DATABASE_URL = management DB) and initializes all the above. A separate
NATS connection is made for the health state and the tenant.provisioned
consumer (the SDK manages its own NATS connection internally for
`.consumer()` wiring).

### Main.rs Skeleton

```rust
use std::sync::Arc;
use std::time::Instant;

use platform_sdk::{ConsumerError, ModuleBuilder, StartupError};
use serde_json;
use uuid::Uuid;

use tt_core::config::FeatureFlags;
use tt_events::stop_lifecycle::{StopCompletedPayload, StopSkippedPayload, StopStartedPayload};

struct TrashTechState { /* fields above */ }

#[tokio::main]
async fn main() {
    // Feature flags loaded before builder for conditional consumer registration.
    // NOTE: tracing is NOT yet initialized here -- flags come from env only.
    // Use std::env::var, not tracing macros.
    let raw_flags = FeatureFlagsRaw::from_env(); // plain env reads

    let mut builder = ModuleBuilder::from_manifest("module.toml")
        .on_startup(|pool| async move {
            warn_placeholder_secrets();
            tt_api::prom_metrics::init();

            // Management DB migrations
            tt_core::db::run_migrations(&pool).await
                .map_err(|e| StartupError::Migration(e.to_string()))?;

            // Tenant resolver
            let resolver = tt_core::tenant::TenantResolver::load_all(&pool).await
                .map_err(|e| StartupError::Config(e.to_string()))?;
            migrate_all_tenants(&resolver).await;

            // Feature flags (now with tracing available)
            let feature_flags = tt_core::config::FeatureFlags::from_env();
            let phase3_config = tt_core::config::Phase3Config::from_env();
            phase3_config.validate(&feature_flags);

            // Provisioning service
            let database_url = std::env::var("DATABASE_URL")
                .map_err(|_| StartupError::Config("DATABASE_URL required".into()))?;
            let base_url = tt_core::provisioning::base_url_from_database_url(&database_url);
            let provisioning = tt_core::provisioning::ProvisioningService::new(
                pool.clone(), resolver.clone(), base_url,
            );

            // NATS client (separate connection for health + provisioning consumer)
            let nats_url = std::env::var("NATS_URL")
                .unwrap_or_else(|_| "nats://localhost:4223".into());
            let nats_client = async_nats::connect(&nats_url).await.ok();

            // External service clients
            let platform_client = tt_integrations::PlatformClient::new(
                tt_integrations::PlatformConfig::from_env()
            );
            // ... geocoding, media, payments, sendgrid clients ...

            Ok(Arc::new(TrashTechState { resolver, provisioning, ... }))
        })
        .skip_default_middleware()
        // --- Stop skipped projector (EventEnvelope format) ---
        .consumer(
            tt_events::stop_skipped_projector::STOP_SKIPPED_SUBJECT,
            |ctx, env| async move {
                let state = ctx.state::<Arc<TrashTechState>>();
                let tenant_id = parse_tenant_id(&env.tenant_id)?;
                let pool = state.resolver.resolve(tenant_id).await
                    .map_err(|e| ConsumerError::Processing(e.to_string()))?;
                let payload: StopSkippedPayload = serde_json::from_value(env.payload.clone())
                    .map_err(|e| ConsumerError::Processing(e.to_string()))?;
                // ... upsert + notification logic ...
                Ok(())
            },
        );

    // Conditional analytics consumers
    if raw_flags.analytics {
        builder = builder
            .consumer("trashtech.events.stop.started", |ctx, env| async move {
                let state = ctx.state::<Arc<TrashTechState>>();
                let tenant_id = parse_tenant_id(&env.tenant_id)?;
                let pool = state.resolver.resolve(tenant_id).await
                    .map_err(|e| ConsumerError::Processing(e.to_string()))?;
                let payload: StopStartedPayload = serde_json::from_value(env.payload.clone())
                    .map_err(|e| ConsumerError::Processing(e.to_string()))?;
                tt_events::analytics_projector::handle_stop_started(&pool, tenant_id, &payload).await;
                Ok(())
            })
            .consumer("trashtech.events.stop.completed", |ctx, env| async move {
                // ... similar pattern ...
                Ok(())
            })
            .consumer("trashtech.events.stop.skipped", |ctx, env| async move {
                // ... similar pattern ...
                Ok(())
            });
    }

    if raw_flags.payments {
        builder = builder.consumer(
            tt_events::payment_consumer::PAYMENT_SUCCEEDED_SUBJECT,
            |ctx, env| async move {
                let state = ctx.state::<Arc<TrashTechState>>();
                let tenant_id = parse_tenant_id(&env.tenant_id)?;
                let pool = state.resolver.resolve(tenant_id).await
                    .map_err(|e| ConsumerError::Processing(e.to_string()))?;
                let payload: tt_events::payment_consumer::PaymentSucceededPayload =
                    serde_json::from_value(env.payload.clone())
                    .map_err(|e| ConsumerError::Processing(e.to_string()))?;
                let ar_invoice_id: i32 = payload.invoice_id.parse()
                    .map_err(|e: std::num::ParseIntError| ConsumerError::Processing(e.to_string()))?;
                tt_events::payment_consumer::apply_payment_succeeded(
                    &pool, tenant_id, env.event_id, ar_invoice_id, &payload.session_id,
                ).await.map_err(|e| ConsumerError::Processing(e.to_string()))?;
                Ok(())
            },
        );
    }

    if raw_flags.email {
        builder = builder.consumer(
            tt_events::password_reset_handler::PASSWORD_RESET_SUBJECT,
            |ctx, env| async move {
                let state = ctx.state::<Arc<TrashTechState>>();
                let tenant_id = parse_tenant_id(&env.tenant_id)?;
                let pool = state.resolver.resolve(tenant_id).await
                    .map_err(|e| ConsumerError::Processing(e.to_string()))?;
                let payload: tt_events::password_reset_handler::PasswordResetRequestedPayload =
                    serde_json::from_value(env.payload.clone())
                    .map_err(|e| ConsumerError::Processing(e.to_string()))?;
                // Build email config from state.phase3_config...
                tt_events::password_reset_handler::handle_reset(
                    &pool, tenant_id, &payload, &email_cfg,
                ).await.map_err(|e| ConsumerError::Processing(e.to_string()))?;
                Ok(())
            },
        );
    }

    builder
        .routes(|ctx| {
            let state = ctx.state::<Arc<TrashTechState>>();
            // Spawn tenant.provisioned consumer manually (bare payload format)
            if let Some(ref nats) = state.nats_client {
                let svc = state.provisioning.clone();
                let nats_for_signals = nats.clone();
                tokio::spawn(tt_events::consumer::run_tenant_provisioned_subscriber(
                    nats.clone(),
                    move |event| { /* ... existing 165-line handler ... */ },
                ));
            }

            let health_state = Arc::new(tt_api::health::HealthState {
                mgmt_pool: ctx.pool().clone(),
                nats: state.nats_client.clone(),
            });

            // Build the existing TrashTech router
            tokio::runtime::Handle::current().block_on(tt_api::router(
                state.resolver.clone(),
                state.platform_client.clone(),
                state.media_url_policy.clone(),
                state.geocoding_client.clone(),
                state.media_storage.clone(),
                state.payments_client.clone(),
                state.sendgrid.clone(),
                health_state,
                state.body_size_limits.clone(),
            ))
        })
        .run()
        .await
        .expect("tt-server failed");
}
```

### Verification

```bash
cargo test --workspace     # All existing tests must pass
# Verify: 7 consumers receive events (6 via SDK, 1 manual)
# Verify: healthz/ready/version return 200
# Verify: auth works end-to-end (JWT via TrashTech's own auth stack)
```

## Open Issues

1. **RESOLVED**: `tt_api::router()` is async -- use `.routes_async()`
   (added to SDK in this bead).
2. `tenant.provisioned` uses bare payload format, not EventEnvelope.
   Spawned manually instead of via `.consumer()`.
3. Feature flags need to be read before tracing init (from raw env vars)
   for conditional `.consumer()` registration.
