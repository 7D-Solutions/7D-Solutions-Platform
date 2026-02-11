//! NATS-based implementation of the EventBus trait

use crate::{BusError, BusMessage, BusResult, EventBus};
use async_nats::Client;
use async_trait::async_trait;
use futures::stream::{BoxStream, StreamExt};

/// EventBus implementation using NATS JetStream
///
/// This is the production implementation that connects to a NATS server.
/// It wraps an `async_nats::Client` and implements the `EventBus` trait.
///
/// # Example
/// ```rust,no_run
/// use event_bus::{EventBus, NatsBus};
///
/// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
/// let nats_client = async_nats::connect("nats://localhost:4222").await?;
/// let bus = NatsBus::new(nats_client);
///
/// // Use the bus
/// bus.publish("my.subject", b"hello".to_vec()).await?;
/// # Ok(())
/// # }
/// ```
#[derive(Clone)]
pub struct NatsBus {
    client: Client,
}

impl NatsBus {
    /// Create a new NatsBus from an existing NATS client
    ///
    /// # Arguments
    /// * `client` - An already-connected `async_nats::Client`
    pub fn new(client: Client) -> Self {
        Self { client }
    }

    /// Get a reference to the underlying NATS client
    ///
    /// This is useful for advanced use cases that need direct access to NATS features
    /// not exposed through the EventBus trait.
    pub fn client(&self) -> &Client {
        &self.client
    }
}

#[async_trait]
impl EventBus for NatsBus {
    async fn publish(&self, subject: &str, payload: Vec<u8>) -> BusResult<()> {
        self.client
            .publish(subject.to_string(), payload.into())
            .await
            .map_err(|e| BusError::PublishError(e.to_string()))?;

        Ok(())
    }

    async fn subscribe(&self, subject: &str) -> BusResult<BoxStream<'static, BusMessage>> {
        let subscriber = self
            .client
            .subscribe(subject.to_string())
            .await
            .map_err(|e| BusError::SubscribeError(e.to_string()))?;

        // Convert NATS messages to BusMessages
        let stream = subscriber.map(|nats_msg| {
            let mut msg = BusMessage::new(nats_msg.subject.to_string(), nats_msg.payload.to_vec());

            // Extract reply-to if present
            if let Some(reply) = nats_msg.reply {
                msg = msg.with_reply_to(reply.to_string());
            }

            // Extract headers if present
            if let Some(nats_headers) = nats_msg.headers {
                let mut headers = std::collections::HashMap::new();
                for (key, values) in nats_headers.iter() {
                    // Take the first value for each header
                    if let Some(value) = values.first() {
                        headers.insert(key.to_string(), value.to_string());
                    }
                }
                if !headers.is_empty() {
                    msg = msg.with_headers(headers);
                }
            }

            msg
        });

        Ok(stream.boxed())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Note: These tests require a running NATS server
    // For CI, use InMemoryBus tests instead
    // For manual testing: docker run -p 4222:4222 nats:2.10-alpine

    #[tokio::test]
    #[ignore] // Requires NATS server
    async fn test_nats_bus_publish_subscribe() {
        let client = async_nats::connect("nats://localhost:4222")
            .await
            .expect("NATS server must be running on localhost:4222");

        let bus = NatsBus::new(client);

        // Subscribe first
        let mut stream = bus.subscribe("test.nats.>").await.unwrap();

        // Publish a message
        let payload = b"test message".to_vec();
        bus.publish("test.nats.hello", payload.clone())
            .await
            .unwrap();

        // Receive the message
        let msg = tokio::time::timeout(std::time::Duration::from_secs(2), stream.next())
            .await
            .expect("timeout waiting for message")
            .expect("stream ended");

        assert_eq!(msg.subject, "test.nats.hello");
        assert_eq!(msg.payload, payload);
    }
}
