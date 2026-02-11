use async_nats::Client;
use async_nats::jetstream::{self, stream::Config};
use std::time::Duration;

pub async fn ensure_streams(nats: Client) -> Result<(), Box<dyn std::error::Error>> {
    let js = jetstream::new(nats);

    // AUTH_EVENTS stream
    let events_cfg = Config {
        name: "AUTH_EVENTS".to_string(),
        subjects: vec!["auth.events.*".to_string()],
        max_age: Duration::from_secs(60 * 60 * 24 * 14), // 14 days (reasonable default)
        ..Default::default()
    };

    if js.get_stream("AUTH_EVENTS").await.is_err() {
        js.create_stream(events_cfg).await?;
    }

    // AUTH_DLQ stream
    let dlq_cfg = Config {
        name: "AUTH_DLQ".to_string(),
        subjects: vec!["auth.dlq.*".to_string()],
        max_age: Duration::from_secs(60 * 60 * 24 * 30), // 30 days
        ..Default::default()
    };

    if js.get_stream("AUTH_DLQ").await.is_err() {
        js.create_stream(dlq_cfg).await?;
    }

    Ok(())
}
