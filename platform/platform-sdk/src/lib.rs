//! Platform SDK — module startup and HTTP runtime.
//!
//! Eliminates per-module boilerplate by providing a single startup sequence
//! that orchestrates existing platform crates (security, event-bus, health).
//!
//! # Usage
//!
//! ```rust,ignore
//! use platform_sdk::ModuleBuilder;
//!
//! #[tokio::main]
//! async fn main() {
//!     ModuleBuilder::from_manifest("module.toml")
//!         .migrator(sqlx::migrate!("./db/migrations"))
//!         .routes(|ctx| {
//!             axum::Router::new()
//!                 // ... register handlers ...
//!         })
//!         .run()
//!         .await
//!         .expect("module failed");
//! }
//! ```

pub mod authz_gate;
pub mod builder;
pub mod client_core;
pub mod consumer;
pub mod context;
pub mod csrf;
pub mod dlq;
pub mod event_registry;
pub mod idempotency;
pub mod http_client;
pub mod manifest;
pub mod platform_services;
pub mod provisioning_hook;
pub mod publisher;
pub mod startup;
mod startup_helpers;
pub mod tenant;
pub mod tenant_resolver;

pub use builder::ModuleBuilder;
pub use event_registry::{EventRegistry, RouteOutcome};
pub use client_core::{ensure, parse_empty, parse_response, build_query_url, ClientError};
pub use consumer::{ConsumerError, TenantProvisionedEvent};
pub use context::{BusNotAvailable, ModuleContext, TenantPoolError, TenantPoolResolver};
pub use http_client::{PlatformClient, TimeoutConfig};
pub use manifest::Manifest;
pub use platform_services::PlatformService;
pub use startup::StartupError;
pub use publisher::{STANDARD_OUTBOX_DDL, ensure_outbox_table};
pub use dlq::{STANDARD_DLQ_DDL, ensure_dlq_table};
pub use idempotency::{STANDARD_DEDUPE_DDL, ensure_dedupe_table};
pub use tenant::{TenantId, TenantPool};
pub use tenant_resolver::DefaultTenantResolver;

// Re-export commonly needed types so modules don't have to depend on
// platform sub-crates directly for basic operations.
pub use async_nats::Client as NatsClient;
pub use event_bus::{EventBus, EventEnvelope};
pub use health::{TenantReadinessCheck, TenantReadinessRegistry};
pub use security::claims::VerifiedClaims;
pub use sqlx::PgPool;

/// Extract the tenant ID string from verified JWT claims in request extensions.
///
/// Returns `Err(ApiError::unauthorized)` if no claims are present.
pub fn extract_tenant(
    claims: &Option<axum::Extension<VerifiedClaims>>,
) -> Result<String, platform_http_contracts::ApiError> {
    match claims {
        Some(axum::Extension(c)) => Ok(c.tenant_id.to_string()),
        None => Err(platform_http_contracts::ApiError::unauthorized(
            "Missing or invalid authentication",
        )),
    }
}
