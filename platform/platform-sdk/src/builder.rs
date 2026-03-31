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

use std::future::Future;
use std::path::Path;
use std::sync::Arc;

use axum::Router;
use event_bus::EventEnvelope;

use crate::consumer::{BoxedHandler, ConsumerDef, ConsumerError};
use crate::context::ModuleContext;
use crate::manifest::{Manifest, ManifestError};
use crate::startup::{self, StartupError};

/// Builder for a platform module HTTP runtime.
pub struct ModuleBuilder {
    manifest: Result<Manifest, ManifestError>,
    routes_fn: Option<Box<dyn FnOnce(ModuleContext) -> Router + Send>>,
    migrator: Option<&'static sqlx::migrate::Migrator>,
    consumers: Vec<ConsumerDef>,
}

impl ModuleBuilder {
    /// Load a module manifest from the given path.
    ///
    /// If the file cannot be read or parsed, the error is deferred
    /// until [`run`](ModuleBuilder::run) is called — this lets the
    /// builder chain stay ergonomic.
    pub fn from_manifest(path: impl AsRef<Path>) -> Self {
        Self {
            manifest: Manifest::from_file(path.as_ref()),
            routes_fn: None,
            migrator: None,
            consumers: Vec::new(),
        }
    }

    /// Register module-specific Axum routes.
    ///
    /// The closure receives a [`ModuleContext`] that provides access
    /// to the database pool and configuration.
    pub fn routes<F>(mut self, f: F) -> Self
    where
        F: FnOnce(ModuleContext) -> Router + Send + 'static,
    {
        self.routes_fn = Some(Box::new(f));
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

    /// Run the module through the full startup sequence.
    ///
    /// This blocks until the server receives a shutdown signal (SIGTERM
    /// or Ctrl+C), then drains connections and returns.
    pub async fn run(self) -> Result<(), StartupError> {
        let manifest = self.manifest?;

        // Phase A: infrastructure
        let phase_a = startup::phase_a(&manifest).await?;

        // Build module context for the route factory and consumers
        let ctx = ModuleContext::new(phase_a.pool.clone(), manifest.clone(), phase_a.bus.clone());

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

        // Build routes (or empty router if none provided)
        let module_routes = match self.routes_fn {
            Some(f) => f(ctx),
            None => {
                tracing::warn!(
                    module = %manifest.module.name,
                    "no routes registered — module will only serve health endpoints"
                );
                Router::new()
            }
        };

        // Phase B: HTTP stack + serve
        startup::phase_b(&manifest, phase_a, module_routes, self.migrator, consumer_handles).await
    }
}
