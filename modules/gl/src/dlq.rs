//! Dead Letter Queue (DLQ) handling
//!
//! This module provides functions to handle failed events and send them to the DLQ.

use event_bus::BusMessage;
use sqlx::PgPool;
use uuid::Uuid;

use crate::repos::failed_repo;

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
        Ok(envelope) => {
            let event_id_opt = envelope
                .get("event_id")
                .and_then(|v| v.as_str())
                .and_then(|s| Uuid::parse_str(s).ok());

            let tenant_id_opt = envelope
                .get("tenant_id")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());

            // Extract correlation fields for observability
            let correlation_id = envelope
                .get("correlation_id")
                .and_then(|v| v.as_str())
                .unwrap_or("none")
                .to_string();

            let source_module = envelope
                .get("source_module")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown")
                .to_string();

            match (event_id_opt, tenant_id_opt) {
                (Some(event_id), Some(tenant_id)) => {
                    // Begin transaction for DLQ write
                    match pool.begin().await {
                        Ok(mut tx) => {
                            match failed_repo::insert(
                                &mut tx,
                                event_id,
                                &msg.subject,
                                &tenant_id,
                                envelope, // Move happens here
                                error,
                                retry_count,
                            )
                            .await
                            {
                                Ok(_) => {
                                    if let Err(commit_err) = tx.commit().await {
                                        tracing::error!(
                                            event_id = %event_id,
                                            subject = %msg.subject,
                                            tenant_id = %tenant_id,
                                            correlation_id = %correlation_id,
                                            source_module = %source_module,
                                            retry_count = retry_count,
                                            error = %error,
                                            commit_error = %commit_err,
                                            "Failed to commit DLQ transaction - event may be lost!"
                                        );
                                    } else {
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
                                Err(dlq_err) => {
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
                                }
                            }
                        }
                        Err(tx_err) => {
                            tracing::error!(
                                event_id = %event_id,
                                subject = %msg.subject,
                                tenant_id = %tenant_id,
                                correlation_id = %correlation_id,
                                source_module = %source_module,
                                retry_count = retry_count,
                                error = %error,
                                tx_error = %tx_err,
                                "Failed to begin DLQ transaction - event may be lost!"
                            );
                        }
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
