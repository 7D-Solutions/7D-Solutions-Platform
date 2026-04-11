//! Builder for constructing and running a platform module.
//!
//! The builder provides exactly three methods:
//! - [`from_manifest`](ModuleBuilder::from_manifest) — load configuration
//! - [`routes`](ModuleBuilder::routes) — register Axum routes
//! - [`run`](ModuleBuilder::run) — start the HTTP server
//!
//! # Example
//!
//! ```rust,no_run
//! use platform_sdk::ModuleBuilder;
//!
//! #[tokio::main]
//! async fn main() {
//!     ModuleBuilder::from_manifest("module.toml")
//!         .routes(|ctx| {
//!             axum::Router::new()
//!                 // ... your routes here ...
//!         })
//!         .run()
//!         .await
//!         .expect("module failed to start");
//! }
//! ```

use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;

use axum::Router;
use event_bus::EventEnvelope;

use security::ratelimit::RateLimitConfig;

use crate::authz_gate::AuthzGateConfig;
use crate::consumer::{BoxedHandler, ConsumerDef, ConsumerError, ProvisioningHandler, TenantProvisionedEvent};
use crate::context::{ModuleContext, TenantPoolResolver};
use crate::event_registry::{EventRegistry, RouteOutcome};
use crate::manifest::{Manifest, ManifestError};
use crate::platform_services::PlatformServices;
use crate::startup::{self, StartupError};

/// Type-erased async startup callback.
type StartupFn = Box<
    dyn FnOnce(
            sqlx::PgPool,
        ) -> Pin<
            Box<
                dyn Future<
                        Output = Result<
                            (TypeId, Box<dyn Any + Send + Sync>),
                            StartupError,
                        >,
                    > + Send,
            >,
        > + Send,
>;

/// Type-erased async route factory.
type RoutesFn = Box<
    dyn FnOnce(
            ModuleContext,
        ) -> Pin<Box<dyn Future<Output = Router> + Send>>
        + Send,
>;

/// Builder for a platform module HTTP runtime.
pub struct ModuleBuilder {
    manifest: Result<Manifest, ManifestError>,
    routes_fn: Option<RoutesFn>,
    migrator: Option<&'static sqlx::migrate::Migrator>,
    consumers: Vec<ConsumerDef>,
    extensions: HashMap<TypeId, Box<dyn Any + Send + Sync>>,
    startup_fns: Vec<StartupFn>,
    provisioning_handler: Option<ProvisioningHandler>,
    pool_resolver: Option<Arc<dyn TenantPoolResolver>>,
    skip_outbox_publisher: bool,
    skip_cors: bool,
    skip_rate_limit: bool,
    skip_auth: bool,
    skip_tracing: bool,
    use_blob_storage: bool,
    csrf_protection: bool,
    authz_gate: Option<Arc<AuthzGateConfig>>,
    /// Named rate limit tiers registered via `.rate_limit_tier()`.
    rate_limit_tiers: Vec<(String, RateLimitConfig, Vec<String>)>,
}

impl ModuleBuilder {
    /// Load a module manifest from the given path.
    ///
    /// The path is resolved at runtime: if `MODULE_MANIFEST_PATH` is
    /// set, it overrides the argument entirely. Otherwise the path is
    /// used as-is — relative paths resolve against the process working
    /// directory.
    ///
    /// If the file cannot be read or parsed, the error is deferred
    /// until [`run`](ModuleBuilder::run) is called — this lets the
    /// builder chain stay ergonomic.
    pub fn from_manifest(path: impl AsRef<Path>) -> Self {
        let resolved = resolve_manifest_path(path.as_ref());
        Self {
            manifest: Manifest::from_file(&resolved),
            routes_fn: None,
            migrator: None,
            consumers: Vec::new(),
            extensions: HashMap::new(),
            startup_fns: Vec::new(),
            provisioning_handler: None,
            pool_resolver: None,
            skip_outbox_publisher: false,
            skip_cors: false,
            skip_rate_limit: false,
            skip_auth: false,
            skip_tracing: false,
            use_blob_storage: false,
            csrf_protection: false,
            authz_gate: None,
            rate_limit_tiers: Vec::new(),
        }
    }

    /// Register module-specific Axum routes (synchronous closure).
    ///
    /// The closure receives a [`ModuleContext`] that provides access
    /// to the database pool, configuration, and custom state.
    pub fn routes<F>(mut self, f: F) -> Self
    where
        F: FnOnce(ModuleContext) -> Router + Send + 'static,
    {
        self.routes_fn = Some(Box::new(move |ctx| Box::pin(async move { f(ctx) })));
        self
    }

    /// Register module-specific Axum routes (async closure).
    ///
    /// Use this when the route factory needs to perform async work —
    /// e.g. loading a JWT verifier or warming a cache before building
    /// the router.
    pub fn routes_async<F, Fut>(mut self, f: F) -> Self
    where
        F: FnOnce(ModuleContext) -> Fut + Send + 'static,
        Fut: Future<Output = Router> + Send + 'static,
    {
        self.routes_fn = Some(Box::new(move |ctx| Box::pin(f(ctx))));
        self
    }

    /// Provide a compile-time migrator (from `sqlx::migrate!`).
    ///
    /// Migrations run automatically when `database.auto_migrate = true`
    /// in the manifest. Without this, auto_migrate is a no-op with a
    /// warning log.
    pub fn migrator(mut self, m: &'static sqlx::migrate::Migrator) -> Self {
        self.migrator = Some(m);
        self
    }

    /// Register an event consumer for a NATS subject.
    ///
    /// The handler is called for each message on `subject`, wrapped in
    /// retry middleware (3 attempts, exponential backoff 100 ms → 30 s).
    /// Consumers are wired after the event bus is created and drained
    /// on shutdown before the database pool closes.
    pub fn consumer<F, Fut>(mut self, subject: impl Into<String>, handler: F) -> Self
    where
        F: Fn(ModuleContext, EventEnvelope<serde_json::Value>) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<(), ConsumerError>> + Send + 'static,
    {
        let handler: BoxedHandler = Arc::new(move |ctx, env| Box::pin(handler(ctx, env)));
        self.consumers.push(ConsumerDef {
            subject: subject.into(),
            handler,
        });
        self
    }

    /// Register a tenant-aware consumer that auto-resolves the tenant pool.
    ///
    /// Eliminates the boilerplate of parsing `tenant_id` from the envelope,
    /// resolving the pool via `ctx.pool_for()`, and deserializing the payload.
    /// The handler receives `(PgPool, Uuid, T)` — the tenant database pool,
    /// tenant ID, and typed payload.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// use platform_contracts::event_naming::SUBJECT_AR_INVOICE_OPENED;
    ///
    /// .tenant_consumer(SUBJECT_AR_INVOICE_OPENED, |pool, tenant_id, payload: InvoiceLifecyclePayload| async move {
    ///     // pool is already resolved for this tenant
    ///     // payload is deserialized from the envelope
    ///     Ok(())
    /// })
    /// ```
    pub fn tenant_consumer<T, F, Fut>(mut self, subject: impl Into<String>, handler: F) -> Self
    where
        T: serde::de::DeserializeOwned + Send + 'static,
        F: Fn(sqlx::PgPool, uuid::Uuid, T) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<(), ConsumerError>> + Send + 'static,
    {
        let handler = Arc::new(handler);
        self.consumers.push(ConsumerDef {
            subject: subject.into(),
            handler: Arc::new(move |ctx, env| {
                let handler = handler.clone();
                Box::pin(async move {
                    let tenant_id: uuid::Uuid = env.tenant_id.parse().map_err(|e| {
                        ConsumerError::Processing(format!(
                            "invalid tenant_id '{}': {e}",
                            env.tenant_id
                        ))
                    })?;

                    let pool = ctx.pool_for(tenant_id).await.map_err(|e| {
                        ConsumerError::Processing(format!(
                            "pool_for({tenant_id}) failed: {e}"
                        ))
                    })?;

                    let payload: T = serde_json::from_value(env.payload).map_err(|e| {
                        ConsumerError::Processing(format!(
                            "payload deserialization failed: {e}"
                        ))
                    })?;

                    handler(pool, tenant_id, payload).await
                })
            }),
        });
        self
    }

    /// Register a callback invoked when a new tenant completes provisioning.
    ///
    /// The SDK subscribes to `tenant.provisioned` on the event bus and
    /// calls the handler with the module context and tenant ID. Use this
    /// for module-specific setup — seed data, default configuration, etc.
    /// Infrastructure (database creation, migrations) is handled by the
    /// provisioning orchestrator before this hook fires.
    ///
    /// Requires an event bus (`bus.type` in module.toml).
    pub fn on_tenant_provisioned<F, Fut>(mut self, handler: F) -> Self
    where
        F: Fn(ModuleContext, TenantProvisionedEvent) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<(), ConsumerError>> + Send + 'static,
    {
        self.provisioning_handler =
            Some(Arc::new(move |ctx, event| Box::pin(handler(ctx, event))));
        self
    }

    /// Subscribe an [`EventRegistry`] to a NATS subject.
    ///
    /// The registry dispatches each incoming event to the handler registered
    /// for its `(event_type, schema_version)` pair. Unrecognized pairs are
    /// logged and skipped without returning an error.
    ///
    /// This is an opt-in alternative to [`consumer`](ModuleBuilder::consumer)
    /// and [`tenant_consumer`](ModuleBuilder::tenant_consumer). Those methods
    /// remain fully supported and their behavior is unchanged.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// use platform_sdk::event_registry::EventRegistry;
    ///
    /// let registry = EventRegistry::new()
    ///     .on::<InvoiceOpened>("invoice.opened", "1.0.0", |ctx, env| async move {
    ///         Ok(())
    ///     });
    ///
    /// ModuleBuilder::from_manifest("module.toml")
    ///     .event_registry("ar.events", registry)
    ///     .run()
    ///     .await?;
    /// ```
    pub fn event_registry(self, subject: impl Into<String>, registry: EventRegistry) -> Self {
        let registry = Arc::new(registry);
        self.consumer(subject, move |ctx, env| {
            let registry = Arc::clone(&registry);
            async move {
                match registry.dispatch(ctx, env).await {
                    RouteOutcome::Retried => Err(ConsumerError::Processing(
                        "event_registry: handler requested retry".into(),
                    )),
                    _ => Ok(()),
                }
            }
        })
    }

    /// Inject module-specific state accessible via [`ModuleContext::state`].
    ///
    /// Any `Send + Sync + 'static` value can be stored. Retrieve it later
    /// in route handlers or consumer closures with `ctx.state::<T>()`.
    ///
    /// Multiple types can be registered — each is keyed by its concrete
    /// type. Registering the same type twice overwrites the earlier value.
    pub fn state<T: Send + Sync + 'static>(mut self, val: T) -> Self {
        self.extensions
            .insert(TypeId::of::<T>(), Box::new(val));
        self
    }

    /// Register an async startup callback that runs after infrastructure
    /// (DB pool, event bus) is ready but before consumers and routes.
    ///
    /// The callback receives the database pool and returns a value that
    /// is stored as module state (accessible via `ctx.state::<T>()`).
    /// Use this for initialisation that depends on the pool — e.g.
    /// loading a tenant resolver, warming caches, or running seed data.
    ///
    /// Multiple `on_startup` callbacks can be registered; each stores
    /// its return value under its own type key.
    pub fn on_startup<T, F, Fut>(mut self, f: F) -> Self
    where
        T: Send + Sync + 'static,
        F: FnOnce(sqlx::PgPool) -> Fut + Send + 'static,
        Fut: Future<Output = Result<T, StartupError>> + Send + 'static,
    {
        let startup: StartupFn = Box::new(move |pool| {
            Box::pin(async move {
                let val = f(pool).await?;
                Ok((TypeId::of::<T>(), Box::new(val) as Box<dyn Any + Send + Sync>))
            })
        });
        self.startup_fns.push(startup);
        self
    }

    /// Skip all SDK middleware layers in one call.
    ///
    /// Equivalent to calling `.skip_cors().skip_rate_limit().skip_auth().skip_tracing()`.
    ///
    /// **Prefer the granular methods** — they let you skip only what you need
    /// while keeping the rest of the SDK stack. This method exists for
    /// backwards compatibility and full-bypass scenarios where the module
    /// supplies its own complete middleware stack.
    ///
    /// The SDK still handles infrastructure: DB pool, event bus, consumers,
    /// graceful shutdown, consumer draining, and observability endpoints
    /// (health, ready, metrics, version).
    #[deprecated(
        since = "0.1.0",
        note = "Use the granular methods instead: .skip_cors(), .skip_rate_limit(), \
                .skip_auth(), .skip_tracing()"
    )]
    pub fn skip_default_middleware(mut self) -> Self {
        self.skip_cors = true;
        self.skip_rate_limit = true;
        self.skip_auth = true;
        self.skip_tracing = true;
        self
    }

    /// Skip the SDK's built-in outbox publisher.
    ///
    /// Use this when the module manages its own outbox publishing — for
    /// example a multi-tenant vertical that publishes from per-tenant
    /// databases rather than the management database. The manifest can
    /// still declare `[events.publish].outbox_table` for documentation
    /// and validation without the SDK spawning a publisher for it.
    ///
    /// **Note:** If you use [`tenant_pool_resolver`](ModuleBuilder::tenant_pool_resolver),
    /// you do not need to call this — the SDK automatically uses the
    /// multi-tenant outbox publisher instead.
    pub fn skip_outbox_publisher(mut self) -> Self {
        self.skip_outbox_publisher = true;
        self
    }

    /// Disable the SDK's CORS middleware layer.
    ///
    /// Use this when the module provides its own CORS configuration —
    /// for example a vertical that runs behind a reverse proxy that
    /// already adds CORS headers. Prevents double-header issues that
    /// break browsers.
    ///
    /// All other SDK middleware (JWT, rate limiting, timeout, health
    /// endpoints) continue to operate normally.
    pub fn skip_cors(mut self) -> Self {
        self.skip_cors = true;
        self
    }

    /// Disable the SDK's rate-limiting middleware layer.
    ///
    /// Use this when the module manages its own request throttling or
    /// when rate limiting is handled upstream (e.g. API gateway, ingress).
    ///
    /// All other SDK middleware (CORS, JWT, timeout, health endpoints)
    /// continue to operate normally.
    pub fn skip_rate_limit(mut self) -> Self {
        self.skip_rate_limit = true;
        self
    }

    /// Register a named rate limit tier with per-route assignment.
    ///
    /// Routes are matched by prefix (longest-first). Paths not matching any
    /// configured tier fall through to the default `"api"` tier.
    ///
    /// Multiple calls accumulate tiers. Tiers registered here are merged with
    /// any tiers declared in `[rate_limit.tiers]` in the manifest (builder
    /// entries win on name collision).
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// use security::ratelimit::RateLimitConfig;
    /// use std::time::Duration;
    ///
    /// ModuleBuilder::from_manifest("module.toml")
    ///     .rate_limit_tier(
    ///         "login",
    ///         RateLimitConfig::new(10, Duration::from_secs(60)),
    ///         ["/api/auth/", "/api/login"],
    ///     )
    ///     .rate_limit_tier(
    ///         "api",
    ///         RateLimitConfig::new(1000, Duration::from_secs(60)),
    ///         ["/api/"],
    ///     )
    ///     .routes(|ctx| { /* ... */ })
    ///     .run()
    ///     .await?;
    /// ```
    pub fn rate_limit_tier(
        mut self,
        name: impl Into<String>,
        config: RateLimitConfig,
        routes: impl IntoIterator<Item = impl Into<String>>,
    ) -> Self {
        self.rate_limit_tiers.push((
            name.into(),
            config,
            routes.into_iter().map(|r| r.into()).collect(),
        ));
        self
    }

    /// Disable the SDK's JWT authentication middleware layer.
    ///
    /// Use this when the module provides its own auth logic or serves
    /// only internal/machine traffic that does not carry bearer tokens.
    ///
    /// All other SDK middleware (CORS, rate limiting, tracing, timeout, health
    /// endpoints) continue to operate normally.
    pub fn skip_auth(mut self) -> Self {
        self.skip_auth = true;
        self
    }

    /// Disable the SDK's tracing context middleware layer.
    ///
    /// Use this when the module receives trace IDs from an upstream proxy
    /// that already injects them, or when the module provides its own
    /// tracing instrumentation.
    ///
    /// All other SDK middleware (CORS, JWT, rate limiting, timeout, health
    /// endpoints) continue to operate normally.
    pub fn skip_tracing(mut self) -> Self {
        self.skip_tracing = true;
        self
    }

    /// Enable optional CSRF protection using the double-submit cookie pattern.
    ///
    /// When `enable` is `true`, the SDK adds a CSRF middleware layer:
    /// - **GET / HEAD / OPTIONS**: sets a `__csrf` cookie with a fresh random token.
    ///   The cookie is `HttpOnly=false` (JavaScript must read it), `SameSite=Strict`,
    ///   and `Secure` when `CSRF_SECURE=true` or `APP_ENV=production`.
    /// - **POST / PUT / PATCH / DELETE**: requires the `X-CSRF-Token` header to match
    ///   the `__csrf` cookie value. Mismatch returns `403 Forbidden`.
    ///
    /// This is a stateless defence-in-depth layer. `SameSite=Strict` is the primary
    /// CSRF protection; the double-submit token adds a second line of defence for
    /// non-SameSite-capable clients.
    ///
    /// Use `enable = false` (or omit this call) for pure API servers that do not
    /// serve browser-facing HTML — most platform modules do not need CSRF protection.
    pub fn csrf_protection(mut self, enable: bool) -> Self {
        self.csrf_protection = enable;
        self
    }

    /// Enable route-level permission enforcement via [`AuthzGateConfig`].
    ///
    /// The config maps `(Method, path)` pairs to the permissions required for
    /// access. Routes not present in the map are **unprotected** — this is
    /// opt-in enforcement, not deny-by-default.
    ///
    /// The middleware runs after JWT authentication, so `VerifiedClaims` are
    /// available for inspection. Users with `admin:all` bypass all checks.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// use axum::http::Method;
    /// use platform_sdk::authz_gate::AuthzGateConfig;
    ///
    /// let config = AuthzGateConfig::new([
    ///     ((Method::GET, "/api/orders"), vec!["orders:read"]),
    ///     ((Method::POST, "/api/orders"), vec!["orders:write"]),
    /// ]);
    ///
    /// ModuleBuilder::from_manifest("module.toml")
    ///     .authz_gate(config)
    ///     .routes(|ctx| { /* ... */ })
    ///     .run()
    ///     .await?;
    /// ```
    pub fn authz_gate(mut self, config: AuthzGateConfig) -> Self {
        self.authz_gate = Some(Arc::new(config));
        self
    }

    /// Enable blob storage for this module.
    ///
    /// The SDK reads the bucket name from the `[blob]` section of `module.toml`
    /// (required when this is called). Credentials and endpoint are resolved
    /// from environment variables at startup:
    ///
    /// - `BLOB_REGION` — required
    /// - `BLOB_ACCESS_KEY_ID` — required
    /// - `BLOB_SECRET_ACCESS_KEY` — required
    /// - `BLOB_ENDPOINT` — optional (MinIO / Cloudflare R2)
    /// - `BLOB_PROVIDER` — optional (default: `"s3"`)
    /// - `BLOB_PRESIGN_TTL_SECONDS` — optional (default: 900)
    /// - `BLOB_MAX_UPLOAD_BYTES` — optional (default: 26 214 400)
    ///
    /// The initialised client is available via [`ModuleContext::blob_storage`].
    ///
    /// Startup fails fast if the `[blob]` section is absent or any required
    /// environment variable is missing.
    pub fn blob_storage(mut self) -> Self {
        self.use_blob_storage = true;
        self
    }

    /// Register a tenant pool resolver for database-per-tenant architectures.
    ///
    /// When registered, `ctx.pool_for(tenant_id)` resolves to the correct
    /// tenant database and the SDK spawns a multi-tenant outbox publisher
    /// that iterates all tenant pools.
    ///
    /// Single-database modules do not need this — `pool_for()` returns the
    /// default pool when no resolver is registered.
    pub fn tenant_pool_resolver<R: TenantPoolResolver + 'static>(mut self, resolver: R) -> Self {
        self.pool_resolver = Some(Arc::new(resolver));
        self
    }

    /// Run the module through the full startup sequence.
    ///
    /// This blocks until the server receives a shutdown signal (SIGTERM
    /// or Ctrl+C), then drains connections and returns.
    pub async fn run(mut self) -> Result<(), StartupError> {
        let manifest = self.manifest?;

        // Merge manifest tiers with builder tiers (builder wins on name collision).
        let mut merged_tiers: Vec<(String, RateLimitConfig, Vec<String>)> =
            if let Some(ref rl) = manifest.rate_limit {
                rl.tiers
                    .iter()
                    .filter(|(name, _)| {
                        !self.rate_limit_tiers.iter().any(|(n, _, _)| n == *name)
                    })
                    .map(|(name, ts)| {
                        (
                            name.clone(),
                            RateLimitConfig::new(
                                ts.requests_per_window,
                                std::time::Duration::from_secs(ts.window_seconds),
                            ),
                            ts.routes.clone(),
                        )
                    })
                    .collect()
            } else {
                Vec::new()
            };
        merged_tiers.extend(self.rate_limit_tiers.drain(..));

        // Phase A: infrastructure
        let phase_a = startup::phase_a(
            &manifest,
            self.skip_outbox_publisher,
            self.skip_auth,
            self.pool_resolver.clone(),
            merged_tiers,
        )
        .await?;

        // Build platform service clients from [platform.services] manifest section.
        let platform_services = PlatformServices::from_manifest(
            manifest.platform.as_ref(),
            &manifest.module.name,
        )?;
        if !platform_services.is_empty() {
            tracing::info!(
                module = %manifest.module.name,
                count = platform_services.len(),
                "platform service clients ready"
            );
        }

        // Initialise blob storage client if requested.
        if self.use_blob_storage {
            let blob_client = startup::init_blob_storage(&manifest).await?;
            tracing::info!(
                module = %manifest.module.name,
                bucket = %blob_client.config.bucket,
                "blob storage client ready"
            );
            self.extensions.insert(
                TypeId::of::<std::sync::Arc<blob_storage::BlobStorageClient>>(),
                Box::new(std::sync::Arc::new(blob_client)),
            );
        }

        // Run startup callbacks — each returns a typed value to store.
        let mut extensions = self.extensions;
        extensions.insert(
            TypeId::of::<PlatformServices>(),
            Box::new(platform_services),
        );
        for startup_fn in self.startup_fns {
            let (type_id, val) = startup_fn(phase_a.pool.clone()).await?;
            extensions.insert(type_id, val);
        }

        // Build module context for the route factory and consumers
        let ctx = ModuleContext::with_extensions(
            phase_a.pool.clone(),
            manifest.clone(),
            phase_a.bus.clone(),
            phase_a.nats_client.clone(),
            self.pool_resolver.clone(),
            extensions,
        );

        // Phase A step 8: wire consumers (after EventBus in step 6)
        let mut consumer_handles = if !self.consumers.is_empty() {
            let bus = phase_a.bus.as_ref().ok_or_else(|| {
                StartupError::Config("consumers registered but no event bus configured".into())
            })?;
            tracing::info!(
                module = %manifest.module.name,
                count = self.consumers.len(),
                "wiring consumers"
            );
            crate::consumer::wire_consumers(self.consumers, bus, &ctx).await?
        } else {
            crate::consumer::ConsumerHandles::empty()
        };

        // Wire provisioning hook (shares shutdown signal with consumers)
        if let Some(handler) = self.provisioning_handler {
            let bus = phase_a.bus.as_ref().ok_or_else(|| {
                StartupError::Config(
                    "on_tenant_provisioned requires an event bus (set bus.type in module.toml)"
                        .into(),
                )
            })?;
            let handle = crate::consumer::wire_provisioning_hook(
                handler,
                bus,
                &ctx,
                consumer_handles.shutdown_rx(),
            )
            .await?;
            consumer_handles.add_task(handle);
        }

        // Auto-wire tenant pool resolver to handle tenant.provisioned events.
        //
        // When a TenantPoolResolver is registered and a bus is available, the SDK
        // subscribes to tenant.provisioned and calls pool_for() to warm the resolver's
        // cache. Modules no longer need a manual on_tenant_provisioned callback for
        // this common pattern.
        //
        // A separate subscription is used so the auto-wire and any explicit
        // on_tenant_provisioned handler both receive the event independently.
        if let (Some(resolver), Some(bus)) = (self.pool_resolver.as_ref(), phase_a.bus.as_ref()) {
            let auto_handle = crate::provisioning_hook::wire_pool_resolver_auto_register(
                Arc::clone(resolver),
                bus,
                &ctx,
                consumer_handles.shutdown_rx(),
            )
            .await?;
            consumer_handles.add_task(auto_handle);
            tracing::info!(
                module = %manifest.module.name,
                "tenant pool resolver auto-registered to tenant.provisioned"
            );
        }

        // Clone context for phase_b before routes_fn consumes the original.
        let phase_b_ctx = ctx.clone();

        // Build routes (or empty router if none provided)
        let module_routes = match self.routes_fn {
            Some(f) => f(ctx).await,
            None => {
                tracing::warn!(
                    module = %manifest.module.name,
                    "no routes registered — module will only serve health endpoints"
                );
                Router::new()
            }
        };

        // Phase B: HTTP stack + serve
        let flags = startup::MiddlewareFlags {
            skip_cors: self.skip_cors,
            skip_rate_limit: self.skip_rate_limit,
            skip_auth: self.skip_auth,
            skip_tracing: self.skip_tracing,
            csrf_protection: self.csrf_protection,
            authz_gate: self.authz_gate,
        };
        startup::phase_b(&manifest, phase_a, module_routes, self.migrator, consumer_handles, phase_b_ctx, flags)
            .await
    }
}

/// Resolve the manifest file path at runtime.
///
/// If `MODULE_MANIFEST_PATH` is set, it overrides the caller's path entirely.
/// Otherwise the given path is returned as-is (relative to CWD).
fn resolve_manifest_path(path: &Path) -> PathBuf {
    if let Ok(override_path) = std::env::var("MODULE_MANIFEST_PATH") {
        return PathBuf::from(override_path);
    }
    path.to_path_buf()
}
