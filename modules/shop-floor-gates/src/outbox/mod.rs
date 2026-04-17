use sqlx::PgConnection;
use uuid::Uuid;

pub async fn enqueue_event_tx(
    conn: &mut PgConnection,
    event_id: Uuid,
    event_type: &str,
    aggregate_type: &str,
    aggregate_id: &str,
    payload: &serde_json::Value,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"INSERT INTO events_outbox (event_id, event_type, aggregate_type, aggregate_id, payload)
           VALUES ($1, $2, $3, $4, $5)"#,
    )
    .bind(event_id)
    .bind(event_type)
    .bind(aggregate_type)
    .bind(aggregate_id)
    .bind(payload)
    .execute(conn)
    .await?;
    Ok(())
}
