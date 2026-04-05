use async_nats::jetstream::{self, stream::Config};
use async_nats::Client;
use std::time::Duration;

pub async fn ensure_streams(nats: Client) -> Result<(), Box<dyn std::error::Error>> {
    let js = jetstream::new(nats);

    // AUTH_EVENTS stream — captures all auth.* subjects including DLQ.
    // Uses create_or_update so it works on both fresh and existing NATS.
    let events_cfg = Config {
        name: "AUTH_EVENTS".to_string(),
        subjects: vec![
            "auth.events.>".to_string(),
        ],
        max_age: Duration::from_secs(60 * 60 * 24 * 14), // 14 days
        ..Default::default()
    };
    js.get_or_create_stream(events_cfg).await?;

    // AUTH_DLQ stream — dead-letter queue for failed auth event processing.
    // Separate from AUTH_EVENTS so DLQ messages don't expire on the same
    // schedule as normal events.
    let dlq_cfg = Config {
        name: "AUTH_DLQ".to_string(),
        subjects: vec!["auth.dlq.>".to_string()],
        max_age: Duration::from_secs(60 * 60 * 24 * 30), // 30 days
        ..Default::default()
    };
    js.get_or_create_stream(dlq_cfg).await?;

    Ok(())
}
