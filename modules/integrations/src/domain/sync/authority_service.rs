//! Authority flip service — Guard→Mutation→Outbox atomicity.
//!
//! Serializes concurrent flips via a PostgreSQL advisory transaction lock keyed on
//! (app_id, provider, entity_type). Bumps authority_version monotonically, quiesces
//! pending push outbox rows, and emits integrations.sync.authority.changed exactly once.

use sqlx::PgPool;
use uuid::Uuid;

use crate::domain::oauth::repo as oauth_repo;
use crate::events::{
    build_sync_authority_changed_envelope, SyncAuthorityChangedPayload,
    EVENT_TYPE_SYNC_AUTHORITY_CHANGED,
};
use crate::outbox::enqueue_event_tx;

use super::authority::AuthorityRow;
use super::authority_repo;

// ============================================================================
// Error
// ============================================================================

#[derive(Debug, thiserror::Error)]
pub enum FlipError {
    #[error("unsupported authority side '{0}': must be 'platform' or 'external'")]
    InvalidSide(String),
    #[error("no active OAuth connection found for provider '{1}' on app '{0}'")]
    ConnectionNotFound(String, String),
    #[error(transparent)]
    Database(#[from] sqlx::Error),
}

// ============================================================================
// Result type
// ============================================================================

#[derive(Debug)]
pub struct FlipResult {
    pub row: AuthorityRow,
    pub previous_side: String,
}

// ============================================================================
// Service
// ============================================================================

/// Atomically flip the authoritative side for a (app_id, provider, entity_type) triple.
///
/// Guarantees:
/// - Flips are serialized per key via `pg_advisory_xact_lock`.
/// - `authority_version` only ever increments, never resets.
/// - Pending push outbox rows for the entity_type are quiesced with
///   `failure_reason = 'authority_superseded'` before the event is enqueued.
/// - `integrations.sync.authority.changed` is enqueued exactly once, inside
///   the same transaction as the version bump.
pub async fn flip_authority(
    pool: &PgPool,
    app_id: &str,
    provider: &str,
    entity_type: &str,
    new_side_str: &str,
    flipped_by: &str,
    correlation_id: String,
) -> Result<FlipResult, FlipError> {
    // Guard: validate the requested side before touching any rows.
    let new_side = super::authority::AuthoritySide::from_str(new_side_str)
        .ok_or_else(|| FlipError::InvalidSide(new_side_str.to_string()))?;

    // Resolve connector_id from the OAuth connection. Required by the event contract.
    let connection = oauth_repo::get_connection(pool, app_id, provider)
        .await?
        .ok_or_else(|| FlipError::ConnectionNotFound(app_id.to_string(), provider.to_string()))?;
    let connector_id = connection.id;

    let mut tx = pool.begin().await?;

    // Advisory transaction lock on (app_id, provider, entity_type).
    // pg_advisory_xact_lock blocks until the lock is free; it auto-releases at commit/rollback.
    let lock_key = format!("sync-auth:{}:{}:{}", app_id, provider, entity_type);
    sqlx::query("SELECT pg_advisory_xact_lock(hashtext($1::text)::bigint)")
        .bind(&lock_key)
        .execute(&mut *tx)
        .await?;

    // Ensure the authority row exists (defaulting to 'platform') and capture current state.
    let current =
        authority_repo::ensure_authority(&mut tx, app_id, provider, entity_type, "platform")
            .await?;
    let previous_side = current.authoritative_side.clone();

    // Bump version and set new side. Always increments even when side is unchanged,
    // so version tracks every explicit authority assertion.
    let flipped =
        authority_repo::bump_version(&mut tx, current.id, new_side.as_str(), flipped_by).await?;

    // Quiesce any pending push outbox rows for this entity_type.
    // These rows were enqueued before the flip and would push under stale authority.
    sqlx::query(
        r#"
        UPDATE integrations_outbox
        SET failure_reason = 'authority_superseded'
        WHERE app_id = $1
          AND published_at IS NULL
          AND failed_at IS NULL
          AND aggregate_type = 'sync_push_attempt'
          AND payload->>'entity_type' = $2
        "#,
    )
    .bind(app_id)
    .bind(entity_type)
    .execute(&mut *tx)
    .await?;

    // Emit integrations.sync.authority.changed — enqueued atomically within this transaction.
    let event_id = Uuid::new_v4();
    let payload = SyncAuthorityChangedPayload {
        app_id: app_id.to_string(),
        connector_id,
        entity_type: entity_type.to_string(),
        entity_id: None,
        previous_authority: previous_side.clone(),
        new_authority: new_side.as_str().to_string(),
        flipped_by: flipped_by.to_string(),
    };
    let envelope = build_sync_authority_changed_envelope(
        event_id,
        app_id.to_string(),
        correlation_id,
        None,
        payload,
    );
    enqueue_event_tx(
        &mut tx,
        event_id,
        EVENT_TYPE_SYNC_AUTHORITY_CHANGED,
        "sync_authority",
        &flipped.id.to_string(),
        app_id,
        &envelope,
    )
    .await?;

    tx.commit().await?;

    tracing::info!(
        app_id = %app_id,
        provider = %provider,
        entity_type = %entity_type,
        previous_side = %previous_side,
        new_side = %new_side.as_str(),
        authority_version = flipped.authority_version,
        flipped_by = %flipped_by,
        "sync authority flipped"
    );

    Ok(FlipResult {
        row: flipped,
        previous_side,
    })
}
