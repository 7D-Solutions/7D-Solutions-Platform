//! DLQ replay drill for the pdf-editor outbox.
//!
//! Inserts synthetic stuck outbox rows, runs the replay procedure,
//! and verifies the result. Runs against real Postgres.
//!
//! Usage:
//!   cargo run --manifest-path modules/pdf-editor/Cargo.toml --bin dlq_replay_drill
//!
//! Environment:
//!   DATABASE_URL — pdf-editor database connection string

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
        .expect("Failed to connect to pdf-editor DB");

    sqlx::migrate!("db/migrations")
        .run(&pool)
        .await
        .expect("Failed to run migrations");

    let tenant_id = format!("drill-{}", Uuid::new_v4());

    // Insert 3 synthetic stuck outbox rows (status = 'pending', old created_at)
    let mut inserted_ids = Vec::new();
    for i in 0..3 {
        let event_id = Uuid::new_v4();
        let event_type = format!("pdf.form.drill_event_{}", i);
        let payload = serde_json::json!({
            "event_id": event_id.to_string(),
            "event_type": &event_type,
            "tenant_id": &tenant_id,
            "source_module": "pdf-editor",
            "source_version": "0.1.0",
            "schema_version": "1.0.0",
            "replay_safe": true,
            "mutation_class": "DATA_MUTATION",
            "occurred_at": "2026-01-01T00:00:00Z",
            "payload": {"drill": true}
        });

        let id: i64 = sqlx::query_scalar(
            r#"
            INSERT INTO events_outbox
                (event_id, subject, payload, tenant_id, status,
                 event_type, source_module, source_version, schema_version,
                 occurred_at, replay_safe, mutation_class, created_at)
            VALUES ($1, $2, $3, $4, 'pending', $5, 'pdf-editor', '0.1.0', '1.0.0',
                    now() - interval '10 minutes', true, 'DATA_MUTATION',
                    now() - interval '10 minutes')
            RETURNING id
            "#,
        )
        .bind(event_id)
        .bind(&event_type)
        .bind(&payload)
        .bind(&tenant_id)
        .bind(&event_type)
        .fetch_one(&pool)
        .await
        .expect("insert drill row");

        inserted_ids.push(id);
    }

    // Count candidates before replay
    let (before_count,): (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM events_outbox \
         WHERE status = 'pending' \
           AND tenant_id = $1 \
           AND created_at < now() - interval '5 minutes'",
    )
    .bind(&tenant_id)
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
    let replayed: Vec<(i64, String)> = sqlx::query_as(
        r#"
        WITH candidates AS (
          SELECT id
          FROM events_outbox
          WHERE status = 'pending'
            AND tenant_id = $1
            AND created_at < now() - interval '5 minutes'
          ORDER BY id
          FOR UPDATE SKIP LOCKED
          LIMIT 100
        )
        UPDATE events_outbox o
        SET status = 'published', published_at = now()
        FROM candidates c
        WHERE o.id = c.id
        RETURNING o.id, o.event_type
        "#,
    )
    .bind(&tenant_id)
    .fetch_all(&pool)
    .await
    .expect("replay");

    println!("replayed_rows={}", replayed.len());
    assert!(
        replayed.len() >= 3,
        "expected at least 3 replayed rows, got {}",
        replayed.len()
    );

    // Count candidates after replay
    let (after_count,): (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM events_outbox \
         WHERE status = 'pending' \
           AND tenant_id = $1 \
           AND created_at < now() - interval '5 minutes'",
    )
    .bind(&tenant_id)
    .fetch_one(&pool)
    .await
    .expect("count after");

    println!("pending_candidates_after={}", after_count);
    assert_eq!(
        after_count, 0,
        "expected 0 candidates after replay, got {}",
        after_count
    );

    // Verify idempotency: replaying again should find 0 candidates
    let replayed_again: Vec<(i64, String)> = sqlx::query_as(
        r#"
        WITH candidates AS (
          SELECT id
          FROM events_outbox
          WHERE status = 'pending'
            AND tenant_id = $1
            AND created_at < now() - interval '5 minutes'
          ORDER BY id
          FOR UPDATE SKIP LOCKED
          LIMIT 100
        )
        UPDATE events_outbox o
        SET status = 'published', published_at = now()
        FROM candidates c
        WHERE o.id = c.id
        RETURNING o.id, o.event_type
        "#,
    )
    .bind(&tenant_id)
    .fetch_all(&pool)
    .await
    .expect("replay again");

    println!("idempotent_replay_rows={}", replayed_again.len());
    assert_eq!(
        replayed_again.len(),
        0,
        "idempotent replay must return 0 rows"
    );

    // Clean up drill rows
    sqlx::query("DELETE FROM events_outbox WHERE tenant_id = $1")
        .bind(&tenant_id)
        .execute(&pool)
        .await
        .expect("cleanup drill rows");

    println!("dlq_replay_drill=ok");
}
