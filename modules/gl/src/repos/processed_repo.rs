use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

/// Check if an event has already been processed (idempotency check)
pub async fn exists(pool: &PgPool, event_id: Uuid) -> Result<bool, sqlx::Error> {
    let result = sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS(SELECT 1 FROM processed_events WHERE event_id = $1)"
    )
    .bind(event_id)
    .fetch_one(pool)
    .await?;

    Ok(result)
}

/// Insert a processed event record within a transaction
pub async fn insert(
    tx: &mut Transaction<'_, Postgres>,
    event_id: Uuid,
    event_type: &str,
    processor: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        INSERT INTO processed_events (event_id, event_type, processor)
        VALUES ($1, $2, $3)
        "#
    )
    .bind(event_id)
    .bind(event_type)
    .bind(processor)
    .execute(&mut **tx)
    .await?;

    Ok(())
}
