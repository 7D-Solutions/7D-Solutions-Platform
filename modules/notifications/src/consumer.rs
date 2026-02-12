use event_bus::BusMessage;
use serde::de::DeserializeOwned;
use sqlx::PgPool;
use uuid::Uuid;

/// Idempotent event consumer
///
/// Ensures events are processed exactly once by tracking processed event IDs
/// in the processed_events table.
pub struct EventConsumer {
    pool: PgPool,
}

impl EventConsumer {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Check if an event has already been processed
    ///
    /// Returns true if the event_id exists in processed_events table
    pub async fn is_processed(&self, event_id: Uuid) -> Result<bool, sqlx::Error> {
        #[derive(sqlx::FromRow)]
        struct ProcessedEvent {
            id: i32,
        }

        let result: Option<ProcessedEvent> = sqlx::query_as(
            r#"
            SELECT id FROM processed_events
            WHERE event_id = $1
            "#,
        )
        .bind(event_id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(result.is_some())
    }

    /// Mark an event as processed
    ///
    /// Records the event_id in processed_events table to prevent duplicate processing
    pub async fn mark_processed(
        &self,
        event_id: Uuid,
        event_type: &str,
        source_module: &str,
    ) -> Result<(), sqlx::Error> {
        sqlx::query(
            r#"
            INSERT INTO processed_events (event_id, event_type, source_module)
            VALUES ($1, $2, $3)
            ON CONFLICT (event_id) DO NOTHING
            "#,
        )
        .bind(event_id)
        .bind(event_type)
        .bind(source_module)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    /// Process an event with idempotency guarantee
    ///
    /// Checks if event was already processed before calling the handler.
    /// Marks event as processed after successful handling.
    ///
    /// # Example
    /// ```ignore
    /// consumer.process_idempotent(msg, |payload: PaymentSucceededPayload| async {
    ///     // Handle the event
    ///     Ok(())
    /// }).await?;
    /// ```
    pub async fn process_idempotent<T, F, Fut>(
        &self,
        msg: &BusMessage,
        handler: F,
    ) -> Result<(), Box<dyn std::error::Error>>
    where
        T: DeserializeOwned,
        F: FnOnce(T) -> Fut,
        Fut: std::future::Future<Output = Result<(), Box<dyn std::error::Error>>>,
    {
        // Parse event envelope
        let envelope: serde_json::Value = serde_json::from_slice(&msg.payload)?;

        let event_id = envelope
            .get("event_id")
            .and_then(|v| v.as_str())
            .ok_or("Missing event_id")?;
        let event_id = Uuid::parse_str(event_id)?;

        let source_module = envelope
            .get("source_module")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");

        // Check if already processed
        if self.is_processed(event_id).await? {
            tracing::debug!(
                event_id = %event_id,
                subject = %msg.subject,
                "Event already processed, skipping"
            );
            return Ok(());
        }

        // Deserialize payload
        let payload: T = serde_json::from_value(envelope.get("payload").unwrap().clone())?;

        // Call handler
        handler(payload).await?;

        // Mark as processed
        self.mark_processed(event_id, &msg.subject, source_module)
            .await?;

        tracing::info!(
            event_id = %event_id,
            subject = %msg.subject,
            "Event processed successfully"
        );

        Ok(())
    }
}
