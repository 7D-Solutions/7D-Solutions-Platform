//! Feature flag framework for gradual rollout and per-tenant enablement.
//!
//! Flags are stored in the `feature_flags` table (managed by the
//! tenant-registry database migrations).  Per-tenant rows override the global
//! default for that flag; absent flags default to `false` (disabled).
//!
//! # Quick start
//!
//! ```rust,ignore
//! use feature_flags::{is_enabled, set_flag};
//!
//! // Check whether "composite_wo_create" is enabled for a tenant.
//! let enabled = is_enabled(&pool, "composite_wo_create", Some(tenant_id)).await?;
//!
//! // Enable globally.
//! set_flag(&pool, "composite_wo_create", None, true).await?;
//!
//! // Override for a specific tenant.
//! set_flag(&pool, "composite_wo_create", Some(tenant_id), false).await?;
//! ```
//!
//! # Admin endpoint
//!
//! Mount [`admin_router`] under a guarded `/admin` prefix to expose HTTP
//! endpoints for flag management:
//!
//! ```rust,ignore
//! let app = Router::new()
//!     .nest("/admin", feature_flags::admin_router(pool.clone()))
//!     .layer(require_platform_admin_middleware);
//! ```

pub mod admin;
pub mod flags;

pub use admin::admin_router;
pub use flags::{delete_flag, is_enabled, set_flag, FlagError};
