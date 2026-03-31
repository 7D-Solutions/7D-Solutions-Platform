//! Runtime context shared across all handlers in a module.
//!
//! [`ModuleContext`] provides access to the database pool, configuration,
//! and (eventually) the event bus. Handlers receive it via Axum state.

use std::sync::Arc;

use sqlx::PgPool;

use crate::manifest::Manifest;

/// Shared runtime context for a platform module.
///
/// Cloning is cheap (all fields are `Arc`-wrapped or `Clone`-cheap).
#[derive(Debug, Clone)]
pub struct ModuleContext {
    pool: PgPool,
    manifest: Arc<Manifest>,
}

impl ModuleContext {
    /// Create a new context from a database pool and parsed manifest.
    pub(crate) fn new(pool: PgPool, manifest: Manifest) -> Self {
        Self {
            pool,
            manifest: Arc::new(manifest),
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

    /// Attempt to access the event bus.
    ///
    /// Returns `Err` in this SDK version — event bus support is added in
    /// Slice 3. Callers that need the bus today should wire it manually.
    pub fn bus(&self) -> Result<(), BusNotAvailable> {
        Err(BusNotAvailable)
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

/// Error returned when a module tries to access the event bus before Slice 3.
#[derive(Debug, thiserror::Error)]
#[error("event bus is not available in this SDK version — use Slice 3+")]
pub struct BusNotAvailable;
