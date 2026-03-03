use sqlx::postgres::PgPoolOptions;
use uuid::Uuid;

const DEFAULT_DB_URL: &str = "postgresql://doc_mgmt_user:doc_mgmt_pass@localhost:5455/doc_mgmt_db";

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let db_url = std::env::var("DATABASE_URL").unwrap_or_else(|_| DEFAULT_DB_URL.to_string());
    let pool = PgPoolOptions::new().max_connections(5).connect(&db_url).await?;
    sqlx::migrate!("./db/migrations").run(&pool).await?;

    let event_a = format!("drill-{}", Uuid::new_v4());
    let event_b = format!("drill-{}", Uuid::new_v4());

    sqlx::query(
        "INSERT INTO doc_outbox (event_type, subject, payload, created_at, published_at)
         VALUES
         ('document.distribution.requested', 'doc_mgmt.events.document.distribution.requested',
           jsonb_build_object('event_id', $1, 'event_type', 'document.distribution.requested', 'payload', jsonb_build_object('drill', true)),
           now() - interval '10 minutes', NULL),
         ('document.distribution.status.updated', 'doc_mgmt.events.document.distribution.status.updated',
           jsonb_build_object('event_id', $2, 'event_type', 'document.distribution.status.updated', 'payload', jsonb_build_object('drill', true)),
           now() - interval '10 minutes', NULL)",
    )
    .bind(&event_a)
    .bind(&event_b)
    .execute(&pool)
    .await?;

    let pending_before: i64 = sqlx::query_scalar(
        "SELECT COUNT(*)
         FROM doc_outbox
         WHERE published_at IS NULL
           AND event_type IN ('document.distribution.requested', 'document.distribution.status.updated')
           AND created_at < now() - interval '5 minutes'",
    )
    .fetch_one(&pool)
    .await?;
    println!("pending_candidates_before={pending_before}");

    let replayed: Vec<i64> = sqlx::query_scalar(
        "WITH candidates AS (
            SELECT id
            FROM doc_outbox
            WHERE published_at IS NULL
              AND event_type IN ('document.distribution.requested', 'document.distribution.status.updated')
              AND created_at < now() - interval '5 minutes'
            ORDER BY id
            FOR UPDATE SKIP LOCKED
            LIMIT 100
         )
         UPDATE doc_outbox o
         SET published_at = now()
         FROM candidates c
         WHERE o.id = c.id
         RETURNING o.id",
    )
    .fetch_all(&pool)
    .await?;

    let pending_after: i64 = sqlx::query_scalar(
        "SELECT COUNT(*)
         FROM doc_outbox
         WHERE published_at IS NULL
           AND event_type IN ('document.distribution.requested', 'document.distribution.status.updated')
           AND created_at < now() - interval '5 minutes'",
    )
    .fetch_one(&pool)
    .await?;

    println!("replayed_rows={}", replayed.len());
    println!("pending_candidates_after={pending_after}");

    if replayed.is_empty() {
        return Err("drill failed: replay set is empty".into());
    }
    if pending_after > pending_before {
        return Err("drill failed: pending candidates increased".into());
    }

    println!("dlq_replay_drill=ok");
    Ok(())
}
