use async_nats::jetstream::{self, stream::Config};
use async_nats::Client;
use std::time::Duration;

pub async fn ensure_streams(nats: Client) -> Result<(), Box<dyn std::error::Error>> {
    let js = jetstream::new(nats);

    // AUTH_EVENTS stream — captures all auth.> subjects (events and DLQ).
    // DLQ consumers use a filtered consumer on auth.dlq.> rather than a
    // separate stream.
    let events_cfg = Config {
        name: "AUTH_EVENTS".to_string(),
        subjects: vec!["auth.>".to_string()],
        max_age: Duration::from_secs(60 * 60 * 24 * 14), // 14 days
        ..Default::default()
    };
    js.get_or_create_stream(events_cfg).await?;

    Ok(())
}
