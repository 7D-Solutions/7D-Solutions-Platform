//! Consumer-side event dispatch: handler registry, router, and context.
//!
//! This crate provides the pure-Rust core of the consumer-side event
//! infrastructure. It contains no database, NATS, or I/O dependencies —
//! only dispatch logic.
//!
//! # Architecture
//!
//! - [`HandlerContext`] — Envelope metadata extracted for handler consumption
//! - [`HandlerRegistry`] — Immutable map of (event_type, schema_version) to handler
//! - [`RegistryBuilder`] — Builder for constructing a `HandlerRegistry`
//! - [`EventRouter`] — Validates envelopes and dispatches through the registry
//! - [`RouteOutcome`] — Result of routing (Handled, Skipped, DeadLettered, etc.)

pub mod context;
pub mod registry;
pub mod router;

pub use context::HandlerContext;
pub use registry::{HandlerError, HandlerFn, HandlerRegistry, LookupResult, RegistryBuilder};
pub use router::{EventRouter, RouteOutcome};
