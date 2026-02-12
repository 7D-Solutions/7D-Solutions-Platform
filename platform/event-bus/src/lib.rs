//! # EventBus Abstraction
//!
//! A platform-level abstraction for event-driven messaging across modules.
//!
//! ## Why This Lives in Tier 1
//!
//! The EventBus is a **shared runtime capability** that all modules depend on.
//! Placing it in `platform/` (Tier 1) allows:
//! - Modules to depend on platform crates without circular dependencies
//! - Plug-and-play module development (modules don't depend on each other)
//! - Config-driven swap between NATS (production) and InMemory (dev/test)
//!
//! ## Implementations
//!
//! - **NatsBus**: Production implementation using NATS JetStream
//! - **InMemoryBus**: Test/dev implementation using in-memory channels
//!
//! ## Usage
//!
//! ```rust,no_run
//! use event_bus::{EventBus, NatsBus, InMemoryBus, BusMessage};
//! use std::sync::Arc;
//!
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! // Production: NATS
//! let nats_client = async_nats::connect("nats://localhost:4222").await?;
//! let bus: Arc<dyn EventBus> = Arc::new(NatsBus::new(nats_client));
//!
//! // Dev/Test: In-Memory
//! let bus: Arc<dyn EventBus> = Arc::new(InMemoryBus::new());
//!
//! // Publish an event
//! let payload = serde_json::to_vec(&serde_json::json!({
//!     "event_type": "user.created",
//!     "user_id": "123"
//! }))?;
//! bus.publish("auth.events.user.created", payload).await?;
//!
//! // Subscribe to events
//! let mut stream = bus.subscribe("auth.events.>").await?;
//! while let Some(msg) = futures::StreamExt::next(&mut stream).await {
//!     println!("Received: {} bytes on {}", msg.payload.len(), msg.subject);
//! }
//! # Ok(())
//! # }
//! ```

mod envelope;
mod inmemory_bus;
mod nats_bus;

pub use envelope::{validate_envelope_fields, EventEnvelope};
pub use inmemory_bus::InMemoryBus;
pub use nats_bus::NatsBus;

use async_trait::async_trait;
use futures::stream::BoxStream;
use std::fmt;

/// A message received from the event bus
#[derive(Debug, Clone)]
pub struct BusMessage {
    /// The subject/topic this message was published to
    pub subject: String,
    /// The message payload (raw bytes)
    pub payload: Vec<u8>,
    /// Optional headers (reserved for future use)
    pub headers: Option<std::collections::HashMap<String, String>>,
    /// Optional reply-to subject (for request-response patterns)
    pub reply_to: Option<String>,
}

impl BusMessage {
    /// Create a new bus message
    pub fn new(subject: String, payload: Vec<u8>) -> Self {
        Self {
            subject,
            payload,
            headers: None,
            reply_to: None,
        }
    }

    /// Add headers to the message
    pub fn with_headers(mut self, headers: std::collections::HashMap<String, String>) -> Self {
        self.headers = Some(headers);
        self
    }

    /// Add a reply-to subject
    pub fn with_reply_to(mut self, reply_to: String) -> Self {
        self.reply_to = Some(reply_to);
        self
    }
}

/// Errors that can occur when using the event bus
#[derive(Debug, thiserror::Error)]
pub enum BusError {
    #[error("failed to publish message: {0}")]
    PublishError(String),

    #[error("failed to subscribe to subject: {0}")]
    SubscribeError(String),

    #[error("connection error: {0}")]
    ConnectionError(String),

    #[error("serialization error: {0}")]
    SerializationError(String),

    #[error("invalid subject pattern: {0}")]
    InvalidSubject(String),

    #[error("internal error: {0}")]
    InternalError(String),
}

/// Result type for event bus operations
pub type BusResult<T> = Result<T, BusError>;

/// Core event bus abstraction for publish-subscribe messaging
///
/// This trait defines the interface that all event bus implementations must satisfy.
/// It supports both publishing events and subscribing to event streams.
#[async_trait]
pub trait EventBus: Send + Sync {
    /// Publish a message to a subject
    ///
    /// # Arguments
    /// * `subject` - The subject/topic to publish to (e.g., "auth.events.user.created")
    /// * `payload` - The message payload as raw bytes
    ///
    /// # Returns
    /// * `Ok(())` if the message was successfully published
    /// * `Err(BusError)` if publishing failed
    async fn publish(&self, subject: &str, payload: Vec<u8>) -> BusResult<()>;

    /// Subscribe to messages matching a subject pattern
    ///
    /// # Arguments
    /// * `subject` - The subject pattern to subscribe to (supports wildcards: `*`, `>`)
    ///   - `*` matches a single token (e.g., `auth.*.created`)
    ///   - `>` matches one or more tokens (e.g., `auth.events.>`)
    ///
    /// # Returns
    /// * `Ok(BoxStream)` containing a stream of messages
    /// * `Err(BusError)` if subscription failed
    ///
    /// # Example
    /// ```rust,no_run
    /// # use event_bus::{EventBus, InMemoryBus};
    /// # use futures::StreamExt;
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let bus = InMemoryBus::new();
    /// let mut stream = bus.subscribe("auth.events.>").await?;
    /// while let Some(msg) = stream.next().await {
    ///     // Process message
    /// }
    /// # Ok(())
    /// # }
    /// ```
    async fn subscribe(&self, subject: &str) -> BusResult<BoxStream<'static, BusMessage>>;
}

impl fmt::Debug for dyn EventBus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "EventBus")
    }
}
