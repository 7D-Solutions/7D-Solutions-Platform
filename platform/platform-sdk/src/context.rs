//! Runtime context shared across all handlers in a module.
//!
//! [`ModuleContext`] provides access to the database pool, configuration,
//! and the event bus. Handlers receive it via Axum state.

use std::sync::Arc;

use event_bus::EventBus;
use sqlx::PgPool;

use crate::manifest::Manifest;

/// Shared runtime context for a platform module.
///
/// Cloning is cheap (all fields are `Arc`-wrapped or `Clone`-cheap).
#[derive(Debug, Clone)]
pub struct ModuleContext {
    pool: PgPool,
    manifest: Arc<Manifest>,
    bus: Option<Arc<dyn EventBus>>,
}

impl ModuleContext {
    /// Create a new context from a database pool, parsed manifest, and optional bus.
    pub fn new(pool: PgPool, manifest: Manifest, bus: Option<Arc<dyn EventBus>>) -> Self {
        Self {
            pool,
            manifest: Arc::new(manifest),
            bus,
        }
    }

    /// Database connection pool.
    pub fn pool(&self) -> &PgPool {
        &self.pool
    }

    /// The parsed module manifest.
    pub fn config(&self) -> &Manifest {
        &self.manifest
    }

    /// Access the event bus.
    ///
    /// Returns `Ok` when a bus was configured via `[bus]` in the manifest.
    /// Returns `Err(BusNotAvailable)` when bus type is `"none"` or the
    /// `[bus]` section is absent.
    pub fn bus(&self) -> Result<&dyn EventBus, BusNotAvailable> {
        self.bus.as_deref().ok_or(BusNotAvailable)
    }

    /// Check that the caller has the given permission.
    ///
    /// Delegates to `security::rbac::check_permissions`. This is a
    /// convenience wrapper so handlers don't import `security` directly.
    pub fn require_permission(
        &self,
        claims: &security::claims::VerifiedClaims,
        permission: &str,
    ) -> Result<(), security::SecurityError> {
        security::check_permissions(claims, &[permission])
    }
}

/// Error returned when a module tries to access the event bus without one configured.
#[derive(Debug, thiserror::Error)]
#[error("event bus is not available — configure [bus] in module.toml")]
pub struct BusNotAvailable;
