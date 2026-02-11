//! In-memory implementation of the EventBus trait for testing and development

use crate::{BusMessage, BusResult, EventBus};
use async_trait::async_trait;
use futures::stream::{BoxStream, StreamExt};
use std::sync::Arc;
use tokio::sync::broadcast;

/// EventBus implementation using in-memory channels
///
/// This implementation is suitable for:
/// - Unit tests (no external dependencies)
/// - Local development without Docker
/// - Integration tests that need fast, isolated message buses
///
/// Messages are broadcast to all subscribers via Tokio broadcast channels.
/// Each unique subject pattern gets its own channel.
///
/// # Example
/// ```rust
/// use event_bus::{EventBus, InMemoryBus};
/// use futures::StreamExt;
///
/// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
/// let bus = InMemoryBus::new();
///
/// // Subscribe before publishing
/// let mut stream = bus.subscribe("test.events.>").await?;
///
/// // Publish a message
/// bus.publish("test.events.created", b"hello".to_vec()).await?;
///
/// // Receive it
/// let msg = stream.next().await.unwrap();
/// assert_eq!(msg.subject, "test.events.created");
/// assert_eq!(msg.payload, b"hello");
/// # Ok(())
/// # }
/// ```
#[derive(Clone)]
pub struct InMemoryBus {
    // Global broadcast channel for all messages
    // We use a broadcast channel with a large buffer to avoid dropping messages
    sender: Arc<broadcast::Sender<BusMessage>>,
}

impl InMemoryBus {
    /// Create a new in-memory event bus
    ///
    /// The bus uses a broadcast channel with a buffer of 1000 messages.
    /// If this buffer is exceeded, the oldest messages will be dropped.
    pub fn new() -> Self {
        let (sender, _) = broadcast::channel(1000);
        Self {
            sender: Arc::new(sender),
        }
    }

    /// Create a new in-memory event bus with a custom buffer size
    ///
    /// # Arguments
    /// * `buffer_size` - The maximum number of messages to buffer before dropping old ones
    pub fn with_capacity(buffer_size: usize) -> Self {
        let (sender, _) = broadcast::channel(buffer_size);
        Self {
            sender: Arc::new(sender),
        }
    }

    /// Check if a subject matches a subscription pattern
    ///
    /// Supports NATS-style wildcards:
    /// - `*` matches exactly one token
    /// - `>` matches one or more tokens
    ///
    /// # Examples
    /// - `auth.events.>` matches `auth.events.user.created`
    /// - `auth.*.created` matches `auth.user.created`
    /// - `auth.events.*` does NOT match `auth.events.user.created` (too many tokens)
    fn matches_pattern(subject: &str, pattern: &str) -> bool {
        let subject_tokens: Vec<&str> = subject.split('.').collect();
        let pattern_tokens: Vec<&str> = pattern.split('.').collect();

        let mut s_idx = 0;
        let mut p_idx = 0;

        while s_idx < subject_tokens.len() && p_idx < pattern_tokens.len() {
            let pattern_token = pattern_tokens[p_idx];

            if pattern_token == ">" {
                // `>` matches all remaining tokens
                return true;
            } else if pattern_token == "*" {
                // `*` matches exactly one token
                s_idx += 1;
                p_idx += 1;
            } else if subject_tokens[s_idx] == pattern_token {
                // Exact match
                s_idx += 1;
                p_idx += 1;
            } else {
                // No match
                return false;
            }
        }

        // Both must be exhausted for a full match (unless pattern ended with `>`)
        s_idx == subject_tokens.len() && p_idx == pattern_tokens.len()
    }
}

impl Default for InMemoryBus {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl EventBus for InMemoryBus {
    async fn publish(&self, subject: &str, payload: Vec<u8>) -> BusResult<()> {
        let msg = BusMessage::new(subject.to_string(), payload);

        // Broadcast to all subscribers
        // We ignore the error if there are no receivers (that's fine)
        let _ = self.sender.send(msg);

        Ok(())
    }

    async fn subscribe(&self, pattern: &str) -> BusResult<BoxStream<'static, BusMessage>> {
        let mut receiver = self.sender.subscribe();
        let pattern = pattern.to_string();

        // Filter messages based on the subscription pattern
        let stream = async_stream::stream! {
            loop {
                match receiver.recv().await {
                    Ok(msg) => {
                        if Self::matches_pattern(&msg.subject, &pattern) {
                            yield msg;
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(skipped)) => {
                        // Log that we skipped messages, but continue
                        eprintln!("InMemoryBus: subscriber lagged, skipped {} messages", skipped);
                    }
                    Err(broadcast::error::RecvError::Closed) => {
                        // Channel closed, end the stream
                        break;
                    }
                }
            }
        };

        Ok(stream.boxed())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::StreamExt;

    #[test]
    fn test_pattern_matching() {
        // Exact match
        assert!(InMemoryBus::matches_pattern("auth.events.user.created", "auth.events.user.created"));

        // Single wildcard
        assert!(InMemoryBus::matches_pattern("auth.events.user.created", "auth.*.user.created"));
        assert!(InMemoryBus::matches_pattern("auth.events.user.created", "auth.events.*.created"));
        assert!(!InMemoryBus::matches_pattern("auth.events.user.created", "auth.*.created"));

        // Multi-level wildcard
        assert!(InMemoryBus::matches_pattern("auth.events.user.created", "auth.>"));
        assert!(InMemoryBus::matches_pattern("auth.events.user.created", "auth.events.>"));
        assert!(!InMemoryBus::matches_pattern("auth.events.user.created", "billing.>"));

        // Edge cases
        assert!(InMemoryBus::matches_pattern("single", "single"));
        assert!(InMemoryBus::matches_pattern("single", "*"));
        assert!(InMemoryBus::matches_pattern("single", ">"));
        assert!(!InMemoryBus::matches_pattern("one.two", "one"));
    }

    #[tokio::test]
    async fn test_publish_and_subscribe() {
        let bus = InMemoryBus::new();

        // Subscribe first
        let mut stream = bus.subscribe("test.events.>").await.unwrap();

        // Publish a message
        let payload = b"test message".to_vec();
        bus.publish("test.events.user.created", payload.clone())
            .await
            .unwrap();

        // Receive the message
        let msg = tokio::time::timeout(std::time::Duration::from_secs(1), stream.next())
            .await
            .expect("timeout")
            .expect("stream ended");

        assert_eq!(msg.subject, "test.events.user.created");
        assert_eq!(msg.payload, payload);
    }

    #[tokio::test]
    async fn test_multiple_messages_in_order() {
        let bus = InMemoryBus::new();
        let mut stream = bus.subscribe("test.>").await.unwrap();

        // Publish multiple messages
        for i in 0..5 {
            let payload = format!("message {}", i).into_bytes();
            bus.publish(&format!("test.msg.{}", i), payload)
                .await
                .unwrap();
        }

        // Verify order
        for i in 0..5 {
            let msg = tokio::time::timeout(std::time::Duration::from_secs(1), stream.next())
                .await
                .expect("timeout")
                .expect("stream ended");

            assert_eq!(msg.subject, format!("test.msg.{}", i));
            assert_eq!(msg.payload, format!("message {}", i).into_bytes());
        }
    }

    #[tokio::test]
    async fn test_wildcard_filtering() {
        let bus = InMemoryBus::new();

        // Subscribe to specific pattern
        let mut stream = bus.subscribe("auth.events.*").await.unwrap();

        // Publish matching and non-matching messages
        bus.publish("auth.events.created", b"match".to_vec())
            .await
            .unwrap();
        bus.publish("auth.events.deleted", b"match".to_vec())
            .await
            .unwrap();
        bus.publish("auth.events.user.created", b"no match".to_vec())
            .await
            .unwrap(); // Too deep
        bus.publish("billing.events.created", b"no match".to_vec())
            .await
            .unwrap(); // Wrong prefix

        // Should receive only the two matching messages
        let msg1 = tokio::time::timeout(std::time::Duration::from_millis(100), stream.next())
            .await
            .expect("timeout")
            .expect("stream ended");
        assert_eq!(msg1.subject, "auth.events.created");

        let msg2 = tokio::time::timeout(std::time::Duration::from_millis(100), stream.next())
            .await
            .expect("timeout")
            .expect("stream ended");
        assert_eq!(msg2.subject, "auth.events.deleted");

        // No more messages should arrive
        let result =
            tokio::time::timeout(std::time::Duration::from_millis(100), stream.next()).await;
        assert!(result.is_err(), "should timeout, no more messages");
    }

    #[tokio::test]
    async fn test_multiple_subscribers() {
        let bus = InMemoryBus::new();

        // Create two subscribers
        let mut stream1 = bus.subscribe("test.>").await.unwrap();
        let mut stream2 = bus.subscribe("test.>").await.unwrap();

        // Publish a message
        let payload = b"broadcast".to_vec();
        bus.publish("test.msg", payload.clone()).await.unwrap();

        // Both should receive it
        let msg1 = tokio::time::timeout(std::time::Duration::from_secs(1), stream1.next())
            .await
            .expect("timeout")
            .expect("stream ended");
        let msg2 = tokio::time::timeout(std::time::Duration::from_secs(1), stream2.next())
            .await
            .expect("timeout")
            .expect("stream ended");

        assert_eq!(msg1.payload, payload);
        assert_eq!(msg2.payload, payload);
    }
}
