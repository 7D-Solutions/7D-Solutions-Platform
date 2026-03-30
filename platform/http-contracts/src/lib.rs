//! # Platform HTTP Contracts
//!
//! Shared HTTP response types for the 7D Solutions Platform.
//!
//! Every module uses these types for consistent API responses:
//! - [`PaginatedResponse`] — generic paginated list envelope
//! - [`ApiError`] — standard error body with `IntoResponse` (behind `axum` feature)
//! - [`FieldError`] — per-field validation detail
//!
//! ## Feature flags
//!
//! | Feature | Effect |
//! |---------|--------|
//! | `axum`  | Enables `IntoResponse` impl on `ApiError` |

pub mod error;
pub mod pagination;

pub use error::{ApiError, FieldError};
pub use pagination::{PaginatedResponse, PaginationMeta};
