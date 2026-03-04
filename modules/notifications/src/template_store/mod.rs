//! DB-backed versioned notification template store.
//!
//! Templates are keyed by `(tenant_id, template_key)` with auto-incrementing
//! version numbers. Publishing a template always creates a new version.

pub mod models;
pub mod repo;
