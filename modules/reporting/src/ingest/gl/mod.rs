//! GL event ingestion for the reporting module.
//!
//! Provides a [`TrialBalanceHandler`] that subscribes to `gl.events.posting.requested`
//! and populates the `rpt_trial_balance_cache` table.
//!
//! ## Usage
//!
//! Call [`register_consumers`] at service startup to wire up all GL consumers:
//!
//! ```no_run
//! # use std::sync::Arc;
//! # use sqlx::PgPool;
//! # use event_bus::EventBus;
//! # use reporting::ingest::gl;
//! # async fn example(pool: PgPool, bus: Arc<dyn EventBus>) {
//! gl::register_consumers(pool, bus);
//! # }
//! ```

pub mod trial_balance;

pub use trial_balance::TrialBalanceHandler;

use std::sync::Arc;

use event_bus::EventBus;
use sqlx::PgPool;

use crate::ingest::{start_consumer, IngestConsumer};

/// Subject for GL posting request events.
pub const SUBJECT_GL_POSTING: &str = "gl.events.posting.requested";

/// Consumer name for the trial balance cache builder.
pub const CONSUMER_TRIAL_BALANCE: &str = "reporting.gl_trial_balance";

/// Register all GL ingestion consumers.
///
/// Spawns background tasks that subscribe to GL event subjects and
/// drive the corresponding [`crate::ingest::StreamHandler`] implementations.
///
/// Safe to call multiple times (each call spawns a new task); in practice
/// call once at service startup.
pub fn register_consumers(pool: PgPool, bus: Arc<dyn EventBus>) {
    let handler = Arc::new(TrialBalanceHandler);
    let consumer = IngestConsumer::new(CONSUMER_TRIAL_BALANCE, pool, handler);
    start_consumer(consumer, bus, SUBJECT_GL_POSTING);
}
