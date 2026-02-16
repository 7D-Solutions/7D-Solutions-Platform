//! Outbox repository for reliable event publishing
//!
//! Uses the transactional outbox pattern to ensure events are persisted
//! within the same transaction as domain changes.
//!
//! ## Phase 16 Migration Note
//!
//! GL currently uses a legacy outbox pattern (raw parameters) instead of
//! EventEnvelope. This provides basic validation but should be migrated
//! to use the platform EventEnvelope + validation helper in a future bead.

use sqlx::{Postgres, Transaction};
use uuid::Uuid;

/// Validate outbox event parameters
///
/// Provides basic validation for GL's legacy outbox pattern.
/// This is a temporary measure - GL should be migrated to use
/// EventEnvelope + validate_and_serialize_envelope in the future.
fn validate_outbox_event_params(
    event_type: &str,
    aggregate_type: &str,
    aggregate_id: &str,
) -> Result<(), String> {
    if event_type.is_empty() {
        return Err("event_type cannot be empty".to_string());
    }
    if aggregate_type.is_empty() {
        return Err("aggregate_type cannot be empty".to_string());
    }
    if aggregate_id.is_empty() {
        return Err("aggregate_id cannot be empty".to_string());
    }
    Ok(())
}

/// Insert an event into the outbox for later publishing
///
/// **IMPORTANT**: This function validates required fields at the boundary
/// to prevent invalid events from being enqueued.
///
/// Note: Envelope metadata columns are nullable for backward compatibility during Phase 16 migration.
/// Future work: Migrate GL to use EventEnvelope + platform validation helper.
///
/// # Arguments
///
/// * `reverses_event_id` - Optional ID of the event being reversed (for compensating transactions)
/// * `supersedes_event_id` - Optional ID of the event being superseded (for corrections)
pub async fn insert_outbox_event(
    tx: &mut Transaction<'_, Postgres>,
    event_id: Uuid,
    event_type: &str,
    aggregate_type: &str,
    aggregate_id: &str,
    payload: serde_json::Value,
) -> Result<(), sqlx::Error> {
    insert_outbox_event_with_linkage(
        tx,
        event_id,
        event_type,
        aggregate_type,
        aggregate_id,
        payload,
        None,
        None,
    )
    .await
}

/// Insert an event into the outbox with reversal/supersession linkage
///
/// This function supports reverses_event_id and supersedes_event_id for
/// compensating transactions and corrections, enabling deterministic replay.
pub async fn insert_outbox_event_with_linkage(
    tx: &mut Transaction<'_, Postgres>,
    event_id: Uuid,
    event_type: &str,
    aggregate_type: &str,
    aggregate_id: &str,
    payload: serde_json::Value,
    reverses_event_id: Option<Uuid>,
    supersedes_event_id: Option<Uuid>,
) -> Result<(), sqlx::Error> {
    // Validate required fields at boundary
    validate_outbox_event_params(event_type, aggregate_type, aggregate_id)
        .map_err(|e| sqlx::Error::Protocol(format!("Outbox validation failed: {}", e)))?;
    sqlx::query(
        r#"
        INSERT INTO events_outbox (
            event_id, event_type, aggregate_type, aggregate_id, payload,
            tenant_id, source_module, source_version, schema_version,
            occurred_at, replay_safe, trace_id, correlation_id, causation_id,
            reverses_event_id, supersedes_event_id, side_effect_id, mutation_class
        )
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18)
        "#,
    )
    .bind(event_id)
    .bind(event_type)
    .bind(aggregate_type)
    .bind(aggregate_id)
    .bind(payload)
    // Envelope metadata - defaulted to NULL/empty for now (Phase 16 migration compatibility)
    .bind(Option::<String>::None) // tenant_id
    .bind(Option::<String>::None) // source_module
    .bind(Option::<String>::None) // source_version
    .bind(Option::<String>::None) // schema_version
    .bind(Option::<chrono::DateTime<chrono::Utc>>::None) // occurred_at
    .bind(Option::<bool>::None) // replay_safe
    .bind(Option::<String>::None) // trace_id
    .bind(Option::<String>::None) // correlation_id
    .bind(Option::<String>::None) // causation_id
    .bind(reverses_event_id) // reverses_event_id - NOW WIRED!
    .bind(supersedes_event_id) // supersedes_event_id - NOW WIRED!
    .bind(Option::<String>::None) // side_effect_id
    .bind(Option::<String>::None) // mutation_class
    .execute(&mut **tx)
    .await?;

    Ok(())
}
