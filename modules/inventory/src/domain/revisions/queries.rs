//! Revision read queries.

use chrono::{DateTime, Utc};
use sqlx::PgPool;
use uuid::Uuid;

use super::models::{ItemRevision, RevisionError};

/// Find the revision for an item that is effective at a given timestamp.
///
/// Returns None if no revision covers the requested time.
pub async fn revision_at(
    pool: &PgPool,
    tenant_id: &str,
    item_id: Uuid,
    at: DateTime<Utc>,
) -> Result<Option<ItemRevision>, RevisionError> {
    let rev = sqlx::query_as::<_, ItemRevision>(
        r#"
        SELECT * FROM item_revisions
        WHERE tenant_id = $1 AND item_id = $2
          AND effective_from IS NOT NULL
          AND effective_from <= $3
          AND (effective_to IS NULL OR effective_to > $3)
        ORDER BY effective_from DESC
        LIMIT 1
        "#,
    )
    .bind(tenant_id)
    .bind(item_id)
    .bind(at)
    .fetch_optional(pool)
    .await?;

    Ok(rev)
}

/// List all revisions for an item ordered by revision_number.
pub async fn list_revisions(
    pool: &PgPool,
    tenant_id: &str,
    item_id: Uuid,
) -> Result<Vec<ItemRevision>, RevisionError> {
    let revs = sqlx::query_as::<_, ItemRevision>(
        r#"
        SELECT * FROM item_revisions
        WHERE tenant_id = $1 AND item_id = $2
        ORDER BY revision_number ASC
        "#,
    )
    .bind(tenant_id)
    .bind(item_id)
    .fetch_all(pool)
    .await?;

    Ok(revs)
}
