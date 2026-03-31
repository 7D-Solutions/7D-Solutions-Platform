//! Runtime context shared across all handlers in a module.
//!
//! [`ModuleContext`] provides access to the database pool, configuration,
//! the event bus, and module-specific custom state injected via the builder.

use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::sync::Arc;

use event_bus::EventBus;
use sqlx::PgPool;

use crate::manifest::Manifest;

/// Type-erased storage for module-specific state.
type Extensions = Arc<HashMap<TypeId, Box<dyn Any + Send + Sync>>>;

/// Shared runtime context for a platform module.
///
/// Cloning is cheap (all fields are `Arc`-wrapped or `Clone`-cheap).
#[derive(Clone)]
pub struct ModuleContext {
    pool: PgPool,
    manifest: Arc<Manifest>,
    bus: Option<Arc<dyn EventBus>>,
    extensions: Extensions,
}

impl std::fmt::Debug for ModuleContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ModuleContext")
            .field("pool", &"<PgPool>")
            .field("manifest", &self.manifest)
            .field("bus", &self.bus.as_ref().map(|_| "<EventBus>"))
            .field("extensions", &format!("{} entries", self.extensions.len()))
            .finish()
    }
}

impl ModuleContext {
    /// Create a new context from a database pool, parsed manifest, and optional bus.
    pub fn new(pool: PgPool, manifest: Manifest, bus: Option<Arc<dyn EventBus>>) -> Self {
        Self {
            pool,
            manifest: Arc::new(manifest),
            bus,
            extensions: Arc::new(HashMap::new()),
        }
    }

    /// Create a new context with pre-built extensions map.
    pub(crate) fn with_extensions(
        pool: PgPool,
        manifest: Manifest,
        bus: Option<Arc<dyn EventBus>>,
        extensions: HashMap<TypeId, Box<dyn Any + Send + Sync>>,
    ) -> Self {
        Self {
            pool,
            manifest: Arc::new(manifest),
            bus,
            extensions: Arc::new(extensions),
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

    /// Retrieve module-specific state injected via [`ModuleBuilder::state`]
    /// or [`ModuleBuilder::on_startup`].
    ///
    /// # Panics
    ///
    /// Panics if no state of type `T` was registered. Use
    /// [`try_state`](ModuleContext::try_state) for a non-panicking variant.
    pub fn state<T: Send + Sync + 'static>(&self) -> &T {
        self.try_state::<T>().unwrap_or_else(|| {
            panic!(
                "ModuleContext::state::<{}> — not registered; \
                 add .state(val) or .on_startup(…) to the builder",
                std::any::type_name::<T>()
            )
        })
    }

    /// Retrieve module-specific state, returning `None` if not registered.
    pub fn try_state<T: Send + Sync + 'static>(&self) -> Option<&T> {
        self.extensions
            .get(&TypeId::of::<T>())
            .and_then(|boxed| boxed.downcast_ref::<T>())
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
