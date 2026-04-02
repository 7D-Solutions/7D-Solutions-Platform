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
use crate::platform_services::{PlatformService, PlatformServices};

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
    nats_client: Option<async_nats::Client>,
    extensions: Extensions,
}

impl std::fmt::Debug for ModuleContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ModuleContext")
            .field("pool", &"<PgPool>")
            .field("manifest", &self.manifest)
            .field("bus", &self.bus.as_ref().map(|_| "<EventBus>"))
            .field("nats_client", &self.nats_client.as_ref().map(|_| "<NatsClient>"))
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
            nats_client: None,
            extensions: Arc::new(HashMap::new()),
        }
    }

    /// Create a new context with pre-built extensions map and optional raw NATS client.
    pub(crate) fn with_extensions(
        pool: PgPool,
        manifest: Manifest,
        bus: Option<Arc<dyn EventBus>>,
        nats_client: Option<async_nats::Client>,
        extensions: HashMap<TypeId, Box<dyn Any + Send + Sync>>,
    ) -> Self {
        Self {
            pool,
            manifest: Arc::new(manifest),
            bus,
            nats_client,
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

    /// Access the raw NATS client for non-EventEnvelope subscriptions.
    ///
    /// Returns `Some` when the bus type is NATS. Returns `None` for
    /// InMemoryBus or when no bus is configured. Use this for subjects
    /// that use bare JSON payloads instead of the platform EventEnvelope
    /// format.
    pub fn raw_nats_client(&self) -> Option<&async_nats::Client> {
        self.nats_client.as_ref()
    }

    /// Get an owned `Arc<dyn EventBus>` for storing in module-specific state.
    ///
    /// Some modules need to store the bus in their `AppState` for handler
    /// access. Returns the SDK's bus Arc so modules don't create a second
    /// connection.
    pub fn bus_arc(&self) -> Result<Arc<dyn EventBus>, BusNotAvailable> {
        self.bus.clone().ok_or(BusNotAvailable)
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

    /// Construct a typed platform service client.
    ///
    /// The service must be declared in `[platform.services]` in `module.toml`.
    /// The typed client `T` must implement [`PlatformService`] — generated
    /// clients do this automatically.
    ///
    /// ```rust,ignore
    /// let party = ctx.platform_client::<PartiesClient>();
    /// let customer = party.get_party(&claims, id).await?;
    /// ```
    ///
    /// Each call constructs a new `T` from a cloned `PlatformClient`.
    /// This is cheap: `reqwest::Client` is internally Arc-wrapped.
    ///
    /// # Panics
    ///
    /// Panics if `PlatformServices` was not registered (module has no
    /// `[platform]` section) or if the requested service is not declared.
    pub fn platform_client<T: PlatformService>(&self) -> T {
        let services = self.try_state::<PlatformServices>().unwrap_or_else(|| {
            panic!(
                "platform_client::<{}> called but no [platform.services] section in manifest",
                std::any::type_name::<T>()
            )
        });
        let client = services.get(T::SERVICE_NAME).unwrap_or_else(|| {
            panic!(
                "platform service '{}' not declared in [platform.services] — \
                 add it to module.toml",
                T::SERVICE_NAME
            )
        });
        T::from_platform_client(client.clone())
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
