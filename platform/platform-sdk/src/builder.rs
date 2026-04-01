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

use crate::consumer::{BoxedHandler, ConsumerDef, ConsumerError};
use crate::context::ModuleContext;
use crate::manifest::{Manifest, ManifestError};
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
    skip_default_middleware: bool,
    skip_outbox_publisher: bool,
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
            skip_default_middleware: false,
            skip_outbox_publisher: false,
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

    /// Skip the SDK's built-in middleware stack (CORS, JWT, rate limiting,
    /// timeout, health endpoints, metrics endpoint).
    ///
    /// Use this when the module provides its own middleware — for example
    /// a vertical that already has custom CORS, auth, and health routes.
    /// The SDK still handles infrastructure (DB pool, event bus, consumers,
    /// graceful shutdown, consumer draining).
    pub fn skip_default_middleware(mut self) -> Self {
        self.skip_default_middleware = true;
        self
    }

    /// Skip the SDK's built-in outbox publisher.
    ///
    /// Use this when the module manages its own outbox publishing — for
    /// example a multi-tenant vertical that publishes from per-tenant
    /// databases rather than the management database. The manifest can
    /// still declare `[events.publish].outbox_table` for documentation
    /// and validation without the SDK spawning a publisher for it.
    pub fn skip_outbox_publisher(mut self) -> Self {
        self.skip_outbox_publisher = true;
        self
    }

    /// Run the module through the full startup sequence.
    ///
    /// This blocks until the server receives a shutdown signal (SIGTERM
    /// or Ctrl+C), then drains connections and returns.
    pub async fn run(self) -> Result<(), StartupError> {
        let manifest = self.manifest?;

        // Phase A: infrastructure
        let phase_a = startup::phase_a(&manifest, self.skip_outbox_publisher).await?;

        // Run startup callbacks — each returns a typed value to store.
        let mut extensions = self.extensions;
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
            extensions,
        );

        // Phase A step 8: wire consumers (after EventBus in step 6)
        let consumer_handles = if !self.consumers.is_empty() {
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
        if self.skip_default_middleware {
            startup::phase_b_raw(&manifest, phase_a, module_routes, self.migrator, consumer_handles)
                .await
        } else {
            startup::phase_b(&manifest, phase_a, module_routes, self.migrator, consumer_handles, phase_b_ctx)
                .await
        }
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
