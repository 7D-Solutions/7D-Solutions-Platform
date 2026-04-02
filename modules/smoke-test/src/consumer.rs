use event_bus::EventBus;
use futures::StreamExt;
use sqlx::PgPool;
use std::sync::Arc;

/// Echo consumer — logs received events. Proves consumer wiring works.
pub async fn start_echo_consumer(bus: Arc<dyn EventBus>, _pool: PgPool) {
    tokio::spawn(async move {
        let subject = "smoke_test.item_created";
        tracing::info!(subject, "Starting smoke-test echo consumer");

        let mut stream = match bus.subscribe(subject).await {
            Ok(s) => s,
            Err(e) => {
                tracing::error!("Failed to subscribe to {}: {}", subject, e);
                return;
            }
        };

        tracing::info!("Subscribed to {}", subject);

        while let Some(msg) = stream.next().await {
            let payload = String::from_utf8_lossy(&msg.payload);
            tracing::info!(
                subject = %msg.subject,
                payload = %payload,
                "Echo consumer received event"
            );
        }
    });
}
