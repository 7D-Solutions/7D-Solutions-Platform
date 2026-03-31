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

use std::path::Path;

use axum::Router;

use crate::context::ModuleContext;
use crate::manifest::{Manifest, ManifestError};
use crate::startup::{self, StartupError};

/// Builder for a platform module HTTP runtime.
pub struct ModuleBuilder {
    manifest: Result<Manifest, ManifestError>,
    routes_fn: Option<Box<dyn FnOnce(ModuleContext) -> Router + Send>>,
    migrator: Option<&'static sqlx::migrate::Migrator>,
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

    /// Run the module through the full startup sequence.
    ///
    /// This blocks until the server receives a shutdown signal (SIGTERM
    /// or Ctrl+C), then drains connections and returns.
    pub async fn run(self) -> Result<(), StartupError> {
        let manifest = self.manifest?;

        // Phase A: infrastructure
        let phase_a = startup::phase_a(&manifest).await?;

        // Build module context for the route factory
        let ctx = ModuleContext::new(phase_a.pool.clone(), manifest.clone());

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
        startup::phase_b(&manifest, phase_a, module_routes, self.migrator).await
    }
}
