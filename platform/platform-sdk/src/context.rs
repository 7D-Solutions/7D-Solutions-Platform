//! Runtime context shared across all handlers in a module.
//!
//! [`ModuleContext`] provides access to the database pool, configuration,
//! the event bus, and module-specific custom state injected via the builder.

use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use event_bus::EventBus;
use sqlx::PgPool;
use uuid::Uuid;

use crate::manifest::{Manifest, ServiceCriticality};
use crate::platform_services::{PlatformService, PlatformServices};
use crate::tenant_quota::{TenantQuota, TenantQuotaError};

/// Error returned when a tenant pool cannot be resolved.
#[derive(Debug, thiserror::Error)]
pub enum TenantPoolError {
    #[error("unknown tenant: {0}")]
    UnknownTenant(Uuid),

    #[error("tenant {tenant_id} exceeded connection budget of {max_connections}")]
    QuotaExceeded {
        tenant_id: Uuid,
        max_connections: usize,
    },

    #[error("pool error: {0}")]
    Pool(String),
}

/// Resolves tenant-specific database pools for database-per-tenant architectures.
///
/// Modules that use a separate PostgreSQL database per tenant implement this
/// trait and register it via [`ModuleBuilder::tenant_pool_resolver`]. The SDK
/// uses it for `ctx.pool_for(tenant_id)` and the multi-tenant outbox publisher.
///
/// Modules using the default single-database pattern do not need this — the
/// SDK's `pool_for()` falls back to the default pool when no resolver is
/// registered.
#[async_trait]
pub trait TenantPoolResolver: Send + Sync {
    /// Get the database pool for a specific tenant.
    async fn pool_for(&self, tenant_id: Uuid) -> Result<PgPool, TenantPoolError>;

    /// List all known tenant pools.
    ///
    /// Used by the multi-tenant outbox publisher to iterate every tenant
    /// database and publish pending events.
    async fn all_pools(&self) -> Result<Vec<(Uuid, PgPool)>, TenantPoolError>;
}

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
    pool_resolver: Option<Arc<dyn TenantPoolResolver>>,
    tenant_quota: Arc<TenantQuota>,
    extensions: Extensions,
}

impl std::fmt::Debug for ModuleContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ModuleContext")
            .field("pool", &"<PgPool>")
            .field("manifest", &self.manifest)
            .field("bus", &self.bus.as_ref().map(|_| "<EventBus>"))
            .field(
                "nats_client",
                &self.nats_client.as_ref().map(|_| "<NatsClient>"),
            )
            .field(
                "pool_resolver",
                &self.pool_resolver.as_ref().map(|_| "<TenantPoolResolver>"),
            )
            .field("tenant_quota", &self.tenant_quota.default_max_connections())
            .field("extensions", &format!("{} entries", self.extensions.len()))
            .finish()
    }
}

impl ModuleContext {
    /// Create a new context from a database pool, parsed manifest, and optional bus.
    pub fn new(pool: PgPool, manifest: Manifest, bus: Option<Arc<dyn EventBus>>) -> Self {
        let quota = Arc::new(TenantQuota::from_database_section(
            manifest.database.as_ref(),
        ));
        Self {
            pool,
            manifest: Arc::new(manifest),
            bus,
            nats_client: None,
            pool_resolver: None,
            tenant_quota: quota,
            extensions: Arc::new(HashMap::new()),
        }
    }

    /// Create a new context with pre-built extensions map and optional raw NATS client.
    pub(crate) fn with_extensions(
        pool: PgPool,
        manifest: Manifest,
        bus: Option<Arc<dyn EventBus>>,
        nats_client: Option<async_nats::Client>,
        pool_resolver: Option<Arc<dyn TenantPoolResolver>>,
        extensions: HashMap<TypeId, Box<dyn Any + Send + Sync>>,
    ) -> Self {
        let quota = Arc::new(TenantQuota::from_database_section(
            manifest.database.as_ref(),
        ));
        Self {
            pool,
            manifest: Arc::new(manifest),
            bus,
            nats_client,
            pool_resolver,
            tenant_quota: quota,
            extensions: Arc::new(extensions),
        }
    }

    /// Database connection pool (the default/management pool).
    pub fn pool(&self) -> &PgPool {
        &self.pool
    }

    /// Get the database pool for a specific tenant.
    ///
    /// If a [`TenantPoolResolver`] is registered (database-per-tenant),
    /// delegates to the resolver. Otherwise returns the default pool
    /// (single-database with tenant_id column).
    ///
    /// This is the preferred way to obtain a pool in handlers that need
    /// to work across both single-DB and multi-DB architectures.
    pub async fn pool_for(&self, tenant_id: Uuid) -> Result<PgPool, TenantPoolError> {
        match &self.pool_resolver {
            Some(resolver) => resolver.pool_for(tenant_id).await,
            None => Ok(self.pool.clone()),
        }
    }

    /// Reserve a tenant budget slot and acquire a database connection.
    pub async fn pool_for_tenant(
        &self,
        tenant_id: Uuid,
    ) -> Result<crate::tenant::TenantPoolGuard, TenantPoolError> {
        let permit = self
            .tenant_quota
            .try_acquire(tenant_id)
            .map_err(|err| match err {
                TenantQuotaError::BudgetExceeded {
                    tenant_id,
                    max_connections,
                } => TenantPoolError::QuotaExceeded {
                    tenant_id,
                    max_connections,
                },
            })?;

        let pool = self.pool_for(tenant_id).await?;

        let conn = pool
            .acquire()
            .await
            .map_err(|e| TenantPoolError::Pool(e.to_string()))?;

        Ok(crate::tenant::TenantPoolGuard::new(tenant_id, conn, permit))
    }

    /// Access the tenant pool resolver, if registered.
    pub fn tenant_pool_resolver(&self) -> Option<&dyn TenantPoolResolver> {
        self.pool_resolver.as_deref()
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

    /// Access the blob storage client initialised by [`ModuleBuilder::blob_storage`].
    ///
    /// # Panics
    ///
    /// Panics if `.blob_storage()` was not called on the builder. Use
    /// [`try_blob_storage`](ModuleContext::try_blob_storage) for a non-panicking variant.
    pub fn blob_storage(&self) -> &blob_storage::BlobStorageClient {
        self.try_blob_storage().unwrap_or_else(|| {
            panic!(
                "ModuleContext::blob_storage — not available; \
                 add .blob_storage() to the builder and [blob] to module.toml"
            )
        })
    }

    /// Access the blob storage client if initialised, or `None`.
    pub fn try_blob_storage(&self) -> Option<&blob_storage::BlobStorageClient> {
        self.try_state::<std::sync::Arc<blob_storage::BlobStorageClient>>()
            .map(|arc| arc.as_ref())
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

    /// Construct a typed platform service client for a **critical** dependency.
    ///
    /// Behaves identically to [`platform_client`](Self::platform_client) but
    /// also asserts at runtime that the service was declared with
    /// `criticality = "critical"` (the default).  Use this to make the
    /// contract explicit and to catch accidental miscategorisation early.
    ///
    /// # Panics
    ///
    /// Panics if the service is not declared, if it was declared with a
    /// non-critical criticality, or if `PlatformServices` was not registered.
    pub fn critical_client<T: PlatformService>(&self) -> T {
        let services = self.try_state::<PlatformServices>().unwrap_or_else(|| {
            panic!(
                "critical_client::<{}> called but no [platform.services] section in manifest",
                std::any::type_name::<T>()
            )
        });
        match services.get_criticality(T::SERVICE_NAME) {
            None => panic!(
                "platform service '{}' not declared in [platform.services] — \
                 add it to module.toml",
                T::SERVICE_NAME
            ),
            Some(c) if c != ServiceCriticality::Critical => panic!(
                "critical_client::<{}> called but service '{}' is declared as {:?} — \
                 use degraded_client for non-critical services",
                std::any::type_name::<T>(),
                T::SERVICE_NAME,
                c
            ),
            _ => {}
        }
        let client = services.get(T::SERVICE_NAME).unwrap_or_else(|| {
            panic!(
                "critical_client::<{}> — service '{}' has no resolved URL \
                 (this should have caused a startup failure)",
                std::any::type_name::<T>(),
                T::SERVICE_NAME
            )
        });
        T::from_platform_client(client.clone())
    }

    /// Construct a typed platform service client for a **degraded** or
    /// **best-effort** dependency.
    ///
    /// Returns `Ok(T)` when the service URL was resolved at startup.
    /// Returns `Err(DegradedMode::Unavailable)` when the service URL was
    /// absent at startup — startup was NOT failed because the criticality is
    /// non-critical.
    ///
    /// Callers should handle the `Err` variant by continuing without the
    /// service and signalling the absence via an `X-Degraded` response header.
    ///
    /// ```rust,ignore
    /// match ctx.degraded_client::<NotificationsClient>() {
    ///     Ok(notif) => {
    ///         if let Err(e) = notif.send_notification(payload).await {
    ///             tracing::warn!(error = %e, "notification failed — ignoring");
    ///         }
    ///     }
    ///     Err(DegradedMode::Unavailable { service }) => {
    ///         tracing::warn!(service, "notifications unavailable — skipping");
    ///         // add X-Degraded: <service> header to the response
    ///     }
    /// }
    /// ```
    ///
    /// # Panics
    ///
    /// Panics if `PlatformServices` was not registered (no `[platform.services]`
    /// section) or if the service was NOT declared as `degraded`/`best-effort`
    /// in `module.toml` (miscategorisation guard).
    pub fn degraded_client<T: PlatformService>(&self) -> Result<T, DegradedMode> {
        let services = self.try_state::<PlatformServices>().unwrap_or_else(|| {
            panic!(
                "degraded_client::<{}> called but no [platform.services] section in manifest",
                std::any::type_name::<T>()
            )
        });
        match services.get_criticality(T::SERVICE_NAME) {
            None => panic!(
                "platform service '{}' not declared in [platform.services] — \
                 add it to module.toml with criticality = \"degraded\" or \"best-effort\"",
                T::SERVICE_NAME
            ),
            Some(c) if !c.is_non_critical() => panic!(
                "degraded_client::<{}> called but service '{}' is declared as {:?} — \
                 use platform_client or critical_client for critical services",
                std::any::type_name::<T>(),
                T::SERVICE_NAME,
                c
            ),
            _ => {}
        }
        match services.get(T::SERVICE_NAME) {
            Some(client) => Ok(T::from_platform_client(client.clone())),
            None => Err(DegradedMode::Unavailable {
                service: T::SERVICE_NAME,
            }),
        }
    }

    /// Build service-level claims for module-to-module calls that don't
    /// originate from an HTTP request (e.g. event consumers, background tasks).
    ///
    /// Delegates to [`PlatformClient::service_claims`](crate::http_client::PlatformClient::service_claims).
    pub fn service_claims(&self, tenant_id: Uuid) -> security::claims::VerifiedClaims {
        crate::http_client::PlatformClient::service_claims(tenant_id)
    }

    /// Like [`service_claims`](Self::service_claims), but parses a string tenant ID.
    ///
    /// Delegates to [`PlatformClient::service_claims_from_str`](crate::http_client::PlatformClient::service_claims_from_str).
    pub fn service_claims_from_str(
        &self,
        tenant_id: &str,
    ) -> Result<security::claims::VerifiedClaims, uuid::Error> {
        crate::http_client::PlatformClient::service_claims_from_str(tenant_id)
    }

    /// Create a tracing span pre-populated with the required structured logging fields.
    ///
    /// The span records `module` (from the manifest), `tenant_id`, `request_id`, and
    /// `actor_id`.  Use this in event consumers and background tasks where
    /// `platform_trace_middleware` is not running.  In HTTP handlers the middleware
    /// creates the span automatically — no manual call is needed.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// use tracing::Instrument as _;
    ///
    /// let span = ctx.log_span(&tenant_id.to_string(), &request_id, &actor_id.to_string());
    /// async move {
    ///     tracing::info!(event = "order.placed", "order created");
    /// }.instrument(span).await;
    /// ```
    pub fn log_span(&self, tenant_id: &str, request_id: &str, actor_id: &str) -> tracing::Span {
        crate::logging::request_span(&self.manifest.module.name, tenant_id, request_id, actor_id)
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

/// Returned by [`ModuleContext::degraded_client`] when a non-critical service
/// dependency could not be satisfied — either because the URL was not configured
/// at startup or the service was explicitly disabled.
///
/// Callers should continue the request with reduced functionality and signal
/// the degraded state to the caller via the `X-Degraded` response header.
///
/// ```rust,ignore
/// match ctx.degraded_client::<NotificationsClient>() {
///     Ok(notif) => { let _ = notif.send(...).await; }
///     Err(DegradedMode::Unavailable { ref service }) => {
///         tracing::warn!(service, "notifications unavailable — skipping");
///         // response builder: add X-Degraded: <service> header
///     }
/// }
/// ```
#[derive(Debug, thiserror::Error)]
pub enum DegradedMode {
    /// The service was declared in `[platform.services]` but its URL was not
    /// resolvable at startup, or the service is disabled.
    #[error("platform service '{service}' is unavailable (non-critical — continuing degraded)")]
    Unavailable { service: &'static str },
}
