use crate::events::{envelope::EventEnvelope, validate};
use async_nats::Client;
use serde::Serialize;
use serde_json::Value;

#[derive(Clone)]
pub struct EventPublisher {
    nats: Client,
}

impl EventPublisher {
    pub fn new(nats: Client) -> Self {
        Self { nats }
    }

    pub async fn publish<T: Serialize>(
        &self,
        subject: &str,
        schema_file: &str,
        envelope: &EventEnvelope<T>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        // Validate against schema (publish-time)
        let v: Value = serde_json::to_value(envelope)?;
        validate::validate(schema_file, &v).map_err(|e| format!("schema validation failed: {e}"))?;

        let bytes = serde_json::to_vec(&v)?;
        self.nats.publish(subject.to_string(), bytes.into()).await?;
        Ok(())
    }
}
