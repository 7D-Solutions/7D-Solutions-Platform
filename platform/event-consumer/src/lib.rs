//! Consumer-side event dispatch: handler registry, router, context, and persistence.
//!
//! This crate provides the consumer-side event infrastructure:
//!
//! # Core dispatch (pure Rust, no I/O)
//!
//! - [`HandlerContext`] — Envelope metadata extracted for handler consumption
//! - [`HandlerRegistry`] — Immutable map of (event_type, schema_version) to handler
//! - [`RegistryBuilder`] — Builder for constructing a `HandlerRegistry`
//! - [`EventRouter`] — Validates envelopes and dispatches through the registry
//! - [`RouteOutcome`] — Result of routing (Handled, Skipped, DeadLettered, etc.)
//!
//! # Persistence layer (requires sqlx + Postgres)
//!
//! - [`idempotency::with_dedupe`] — Execute handler exactly once per event_id
//! - [`dlq::write_dlq_entry`] — Write failed events to the dead-letter queue
//! - [`dlq::classify_handler_error`] — Map handler errors to DLQ failure kinds
//!
//! ## Migration templates
//!
//! SQL templates for the `event_dedupe` and `event_dlq` tables are in
//! `platform/event-consumer/sql/`. Copy them into your consuming service's
//! migrations directory.

pub mod context;
pub mod dlq;
pub mod idempotency;
pub mod jetstream;
pub mod registry;
pub mod router;

pub use context::HandlerContext;
pub use dlq::{classify_handler_error, write_dlq_entry, DlqEntry, DlqError, FailureKind};
pub use idempotency::{with_dedupe, DedupeError, DedupeOutcome};
pub use jetstream::{ConsumerConfig, ConsumerError, ConsumerHealth, HealthSnapshot, JetStreamConsumer};
pub use registry::{HandlerError, HandlerFn, HandlerRegistry, LookupResult, RegistryBuilder};
pub use router::{EventRouter, RouteOutcome};
