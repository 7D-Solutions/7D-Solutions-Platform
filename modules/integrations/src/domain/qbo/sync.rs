//! QBO full entity resync.
//!
//! Paginates through all entities of each tracked type via QBO query API,
//! writing each entity as a canonical observation row.  Used for:
//! - Initial data load after a new tenant connects
//! - Recovery when CDC watermark drifts past the 30-day lookback limit
//! - Manual resync triggered by setting `full_resync_required = TRUE`

use chrono::Utc;
use sqlx::PgPool;
use std::sync::Arc;
use tokio::sync::Semaphore;

use super::cdc::{comparable_fields, parse_last_updated_time, DbTokenProvider, CDC_ENTITIES};
use super::client::QboClient;
use super::repo;
use super::TokenProvider;
use crate::domain::sync::dedupe::{
    compute_comparable_hash, compute_fingerprint, truncate_to_millis,
};
use crate::domain::sync::observations;

/// Maximum concurrent QBO API requests per realm during full resync.
pub const MAX_CONCURRENT_PER_REALM: usize = 10;

/// Run a full resync for a single tenant: paginate all entity types and write
/// canonical observation rows.
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
        let _permit = semaphore.acquire().await?;
        let count = resync_entity_type(pool, &client, app_id, entity_type).await?;
        total += count;
    }

    // Mark resync complete; watermark set to now so CDC can resume immediately.
    let now = Utc::now();
    repo::mark_resync_complete(pool, app_id, now).await?;

    Ok(total)
}

/// Resync all entities of a single type by paginating through the query API.
async fn resync_entity_type(
    pool: &PgPool,
    client: &QboClient,
    app_id: &str,
    entity_type: &str,
) -> Result<u32, Box<dyn std::error::Error + Send + Sync>> {
    let base_query = format!("SELECT * FROM {}", entity_type);
    let entities = client.query_all(&base_query, entity_type).await?;

    let entity_type_lower = entity_type.to_lowercase();
    let mut count = 0u32;

    for entity in &entities {
        let entity_id = entity
            .get("Id")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");

        let lut = parse_last_updated_time(entity);
        let lut_truncated = truncate_to_millis(lut);

        let sync_token = entity.get("SyncToken").and_then(|v| v.as_str());
        let comparable = comparable_fields(entity);
        let fingerprint = compute_fingerprint(sync_token, Some(lut_truncated), entity);
        let comparable_hash = compute_comparable_hash(&comparable, lut_truncated);

        observations::upsert_observation(
            pool,
            app_id,
            "quickbooks",
            &entity_type_lower,
            entity_id,
            &fingerprint,
            lut_truncated,
            &comparable_hash,
            1,
            entity,
            "full_resync",
            false, // full-resync never returns deleted entities
        )
        .await
        .map_err(|e| {
            tracing::error!(
                app_id, entity_type, entity_id,
                error = %e, "Failed to upsert resync observation"
            );
            e
        })?;

        count += 1;
    }

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
