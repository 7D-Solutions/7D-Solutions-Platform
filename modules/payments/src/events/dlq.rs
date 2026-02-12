use event_bus::BusMessage;
use sqlx::PgPool;
use uuid::Uuid;

/// Insert a failed event into the Dead Letter Queue
///
/// This function stores events that failed to process after all retries.
/// It ensures no events are silently dropped.
pub async fn insert_failed_event(
    pool: &PgPool,
    event_id: Uuid,
    subject: &str,
    tenant_id: &str,
    envelope_json: &serde_json::Value,
    error: &str,
    retry_count: i32,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        INSERT INTO failed_events (event_id, subject, tenant_id, envelope_json, error, retry_count)
        VALUES ($1, $2, $3, $4, $5, $6)
        ON CONFLICT (event_id) DO UPDATE
        SET retry_count = EXCLUDED.retry_count,
            error = EXCLUDED.error,
            failed_at = NOW()
        "#,
    )
    .bind(event_id)
    .bind(subject)
    .bind(tenant_id)
    .bind(envelope_json)
    .bind(error)
    .bind(retry_count)
    .execute(pool)
    .await?;

    tracing::error!(
        event_id = %event_id,
        subject = %subject,
        tenant_id = %tenant_id,
        error = %error,
        "Event moved to DLQ after failure"
    );

    Ok(())
}

/// Handle processing error by logging and sending to DLQ
///
/// This helper extracts event metadata from the message payload and writes
/// the failure to the DLQ table. It centralizes error handling to ensure
/// consistent logging and DLQ behavior across all consumers.
///
/// # Arguments
/// * `pool` - Database connection pool
/// * `msg` - The original bus message that failed processing
/// * `error` - The error message from processing
/// * `retry_count` - Number of retry attempts made before sending to DLQ
pub async fn handle_processing_error(
    pool: &PgPool,
    msg: &BusMessage,
    error: &str,
    retry_count: i32,
) {
    // Extract event_id, tenant_id, and envelope for DLQ
    match serde_json::from_slice::<serde_json::Value>(&msg.payload) {
        Ok(env) => {
            let event_id_opt = env
                .get("event_id")
                .and_then(|v| v.as_str())
                .and_then(|s| Uuid::parse_str(s).ok());

            let tenant_id_opt = env.get("tenant_id").and_then(|v| v.as_str());

            // Extract correlation fields for observability
            let correlation_id = env
                .get("correlation_id")
                .and_then(|v| v.as_str())
                .unwrap_or("none");

            let source_module = env
                .get("source_module")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown");

            match (event_id_opt, tenant_id_opt) {
                (Some(event_id), Some(tenant_id)) => {
                    // Write to DLQ with correlation fields
                    if let Err(dlq_err) =
                        insert_failed_event(pool, event_id, &msg.subject, tenant_id, &env, error, retry_count)
                            .await
                    {
                        tracing::error!(
                            event_id = %event_id,
                            subject = %msg.subject,
                            tenant_id = %tenant_id,
                            correlation_id = %correlation_id,
                            source_module = %source_module,
                            retry_count = retry_count,
                            error = %error,
                            dlq_error = %dlq_err,
                            "Failed to write to DLQ - event may be lost!"
                        );
                    } else {
                        // Log successful DLQ write with all correlation fields
                        tracing::error!(
                            event_id = %event_id,
                            subject = %msg.subject,
                            tenant_id = %tenant_id,
                            correlation_id = %correlation_id,
                            source_module = %source_module,
                            retry_count = retry_count,
                            error = %error,
                            "Event moved to DLQ after retries exhausted"
                        );
                    }
                }
                _ => {
                    tracing::error!(
                        subject = %msg.subject,
                        error = %error,
                        "Failed to extract event_id or tenant_id from envelope for DLQ"
                    );
                }
            }
        }
        Err(parse_err) => {
            tracing::error!(
                subject = %msg.subject,
                error = %error,
                parse_error = %parse_err,
                "Failed to process event and could not parse envelope for DLQ"
            );
        }
    }
}
