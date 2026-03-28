//! QBO full entity resync.
//!
//! Paginates through all entities of each tracked type via QBO query API,
//! publishing each entity to the outbox. Used for:
//! - Initial data load after a new tenant connects
//! - Recovery when CDC watermark drifts past the 30-day lookback limit
//! - Manual resync triggered by setting `full_resync_required = TRUE`

use chrono::Utc;
use sqlx::PgPool;
use std::sync::Arc;
use tokio::sync::Semaphore;
use uuid::Uuid;

use super::cdc::{
    DbTokenProvider, QboCdcEntityPayload, CDC_ENTITIES, EVENT_TYPE_QBO_ENTITY_SYNCED,
};
use super::client::QboClient;
use super::TokenProvider;
use crate::events::envelope::create_integrations_envelope;
use crate::events::MUTATION_CLASS_DATA_MUTATION;
use crate::outbox::enqueue_event_tx;

/// Maximum concurrent QBO API requests per realm during full resync.
pub const MAX_CONCURRENT_PER_REALM: usize = 10;

/// Run a full resync for a single tenant: paginate all entity types and enqueue.
///
/// After all entities are synced, clears `full_resync_required` and sets
/// `cdc_watermark` to now.
pub async fn full_resync(
    pool: &PgPool,
    base_url: &str,
    app_id: &str,
    realm_id: &str,
) -> Result<u32, Box<dyn std::error::Error + Send + Sync>> {
    let tokens: Arc<dyn TokenProvider> = Arc::new(DbTokenProvider {
        pool: pool.clone(),
        app_id: app_id.to_string(),
    });
    let client = QboClient::new(base_url, realm_id, tokens);
    let semaphore = Arc::new(Semaphore::new(MAX_CONCURRENT_PER_REALM));

    let mut total = 0u32;

    for entity_type in CDC_ENTITIES {
        // Acquire permit to respect rate limits
        let _permit = semaphore.acquire().await?;
        let count = resync_entity_type(pool, &client, app_id, realm_id, entity_type).await?;
        total += count;
    }

    // Mark resync complete
    let now = Utc::now();
    sqlx::query(
        "UPDATE integrations_oauth_connections \
         SET full_resync_required = FALSE, cdc_watermark = $1, updated_at = $1 \
         WHERE app_id = $2 AND provider = 'quickbooks'",
    )
    .bind(now)
    .bind(app_id)
    .execute(pool)
    .await?;

    Ok(total)
}

/// Resync all entities of a single type by paginating through the query API.
async fn resync_entity_type(
    pool: &PgPool,
    client: &QboClient,
    app_id: &str,
    realm_id: &str,
    entity_type: &str,
) -> Result<u32, Box<dyn std::error::Error + Send + Sync>> {
    let base_query = format!("SELECT * FROM {}", entity_type);
    let entities = client.query_all(&base_query, entity_type).await?;

    let mut tx = pool.begin().await?;
    let now = Utc::now();
    let correlation_id = Uuid::new_v4().to_string();
    let mut count = 0u32;

    // Hoist invariant string allocations out of the per-entity loop
    let entity_type_str = entity_type.to_string();
    let realm_id_str = realm_id.to_string();
    let app_id_str = app_id.to_string();
    let event_type_str = EVENT_TYPE_QBO_ENTITY_SYNCED.to_string();
    let mutation_class_str = MUTATION_CLASS_DATA_MUTATION.to_string();
    let aggregate_type = format!("qbo_{}", entity_type.to_lowercase());

    for entity in &entities {
        let entity_id = entity
            .get("Id")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");

        let event_id = Uuid::new_v4();
        let payload = QboCdcEntityPayload {
            entity_type: entity_type_str.clone(),
            entity_id: entity_id.to_string(),
            realm_id: realm_id_str.clone(),
            source: "full_resync".to_string(),
            entity_data: entity.clone(),
            synced_at: now,
        };

        let envelope = create_integrations_envelope(
            event_id,
            app_id_str.clone(),
            event_type_str.clone(),
            correlation_id.clone(),
            None,
            mutation_class_str.clone(),
            payload,
        );

        enqueue_event_tx(
            &mut tx,
            event_id,
            EVENT_TYPE_QBO_ENTITY_SYNCED,
            &aggregate_type,
            entity_id,
            app_id,
            &envelope,
        )
        .await?;

        count += 1;
    }

    tx.commit().await?;

    tracing::info!(entity_type, count, "Full resync completed for entity type");

    Ok(count)
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn max_concurrent_per_realm_is_ten() {
        assert_eq!(MAX_CONCURRENT_PER_REALM, 10);
    }

    #[test]
    fn semaphore_limits_to_ten() {
        let sem = Semaphore::new(MAX_CONCURRENT_PER_REALM);
        assert_eq!(sem.available_permits(), 10);
    }
}
