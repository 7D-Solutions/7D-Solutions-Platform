//! Employee service — re-exports from repo layer.
//!
//! The employee domain is pure CRUD with no business logic beyond
//! model-level validation, so the repo IS the service.

pub use super::repo::EmployeeRepo;
