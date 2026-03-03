//! DLQ replay drill for the workflow outbox.
//!
//! Inserts synthetic stuck outbox rows, runs the replay procedure,
//! and verifies the result. Runs against real Postgres.
//!
//! Usage:
//!   cargo run --manifest-path modules/workflow/Cargo.toml --bin dlq_replay_drill
//!
//! Environment:
//!   DATABASE_URL - workflow database connection string

use sqlx::postgres::PgPoolOptions;
use uuid::Uuid;

#[tokio::main]
async fn main() {
    dotenvy::dotenv().ok();
    let url = std::env::var("DATABASE_URL").expect("DATABASE_URL must be set");

    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&url)
        .await
        .expect("Failed to connect to workflow DB");

    sqlx::migrate!("db/migrations")
        .run(&pool)
        .await
        .expect("Failed to run migrations");

    let tenant_id = format!("drill-{}", Uuid::new_v4());

    // Insert 3 synthetic stuck outbox rows (published_at = NULL, old created_at)
    let mut inserted_ids = Vec::new();
    for i in 0..3 {
        let event_id = Uuid::new_v4();
        let event_type = format!("workflow.drill_event_{}", i);
        let payload = serde_json::json!({
            "event_id": event_id.to_string(),
            "event_type": &event_type,
            "tenant_id": &tenant_id,
            "source_module": "workflow",
            "source_version": "0.1.0",
            "schema_version": "1.0.0",
            "replay_safe": true,
            "mutation_class": "DATA_MUTATION",
            "occurred_at": "2026-01-01T00:00:00Z",
            "payload": {"drill": true}
        });

        let id: i32 = sqlx::query_scalar(
            r#"
            INSERT INTO events_outbox
                (event_id, event_type, aggregate_type, aggregate_id,
                 payload, created_at)
            VALUES ($1, $2, 'drill', $3, $4,
                    now() - interval '10 minutes')
            RETURNING id
            "#,
        )
        .bind(event_id)
        .bind(&event_type)
        .bind(format!("drill-{}", i))
        .bind(&payload)
        .fetch_one(&pool)
        .await
        .expect("insert drill row");

        inserted_ids.push(id);
    }

    // Count candidates before replay
    let (before_count,): (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM events_outbox \
         WHERE published_at IS NULL \
           AND created_at < now() - interval '5 minutes'",
    )
    .fetch_one(&pool)
    .await
    .expect("count before");

    println!("pending_candidates_before={}", before_count);
    assert!(
        before_count >= 3,
        "expected at least 3 candidates, got {}",
        before_count
    );

    // Replay: mark stuck rows as published (same SQL as runbook)
    let replayed: Vec<(i32, String)> = sqlx::query_as(
        r#"
        WITH candidates AS (
          SELECT id
          FROM events_outbox
          WHERE published_at IS NULL
            AND created_at < now() - interval '5 minutes'
          ORDER BY id
          FOR UPDATE SKIP LOCKED
          LIMIT 100
        )
        UPDATE events_outbox o
        SET published_at = now()
        FROM candidates c
        WHERE o.id = c.id
        RETURNING o.id, o.event_type
        "#,
    )
    .fetch_all(&pool)
    .await
    .expect("replay");

    println!("replayed_rows={}", replayed.len());
    assert!(
        replayed.len() >= 3,
        "expected at least 3 replayed rows, got {}",
        replayed.len()
    );

    // Verify idempotency: running replay again should find 0 candidates
    let replayed_again: Vec<(i32, String)> = sqlx::query_as(
        r#"
        WITH candidates AS (
          SELECT id
          FROM events_outbox
          WHERE published_at IS NULL
            AND created_at < now() - interval '5 minutes'
          ORDER BY id
          FOR UPDATE SKIP LOCKED
          LIMIT 100
        )
        UPDATE events_outbox o
        SET published_at = now()
        FROM candidates c
        WHERE o.id = c.id
        RETURNING o.id, o.event_type
        "#,
    )
    .fetch_all(&pool)
    .await
    .expect("replay again");

    println!("idempotent_replay_rows={}", replayed_again.len());
    assert_eq!(
        replayed_again.len(),
        0,
        "idempotent replay should return 0 rows, got {}",
        replayed_again.len()
    );

    // Count candidates after replay
    let (after_count,): (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM events_outbox \
         WHERE published_at IS NULL \
           AND created_at < now() - interval '5 minutes'",
    )
    .fetch_one(&pool)
    .await
    .expect("count after");

    println!("pending_candidates_after={}", after_count);
    assert_eq!(
        after_count, 0,
        "expected 0 candidates after replay, got {}",
        after_count
    );

    // Clean up drill rows
    for id in &inserted_ids {
        sqlx::query("DELETE FROM events_outbox WHERE id = $1")
            .bind(id)
            .execute(&pool)
            .await
            .expect("cleanup drill row");
    }

    println!("dlq_replay_drill=ok");
}
