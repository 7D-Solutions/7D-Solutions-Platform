use notifications_rs::event_bus::{create_notifications_envelope, enqueue_event};
use sqlx::postgres::PgPoolOptions;
use uuid::Uuid;

const DEFAULT_DB_URL: &str =
    "postgresql://notifications_user:notifications_pass@localhost:5437/notifications_db";

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let db_url = std::env::var("DATABASE_URL").unwrap_or_else(|_| DEFAULT_DB_URL.to_string());
    let pool = PgPoolOptions::new().max_connections(5).connect(&db_url).await?;
    sqlx::migrate!("./db/migrations").run(&pool).await?;

    let tenant_id = format!("drill-tenant-{}", Uuid::new_v4());
    let notif_id = Uuid::new_v4();

    sqlx::query(
        "INSERT INTO scheduled_notifications \
         (id, tenant_id, recipient_ref, channel, template_key, payload_json, deliver_at, status, retry_count, dead_lettered_at, last_error) \
         VALUES ($1, $2, $3, 'email', 'invoice_due_soon', $4, NOW(), 'dead_lettered', 5, NOW(), 'drill seeded dead letter')",
    )
    .bind(notif_id)
    .bind(&tenant_id)
    .bind(format!("{}:ops-user", tenant_id))
    .bind(serde_json::json!({"invoice_id":"INV-DRILL","amount_due_minor":1000}))
    .execute(&pool)
    .await?;

    let pending_before: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM scheduled_notifications \
         WHERE tenant_id = $1 AND status = 'dead_lettered'",
    )
    .bind(&tenant_id)
    .fetch_one(&pool)
    .await?;
    println!("pending_before={pending_before}");

    let mut tx = pool.begin().await?;
    sqlx::query(
        "UPDATE scheduled_notifications \
         SET status = 'pending', deliver_at = NOW(), retry_count = 0, replay_generation = replay_generation + 1, \
             last_error = NULL, dead_lettered_at = NULL, failed_at = NULL \
         WHERE id = $1 AND tenant_id = $2 AND status = 'dead_lettered'",
    )
    .bind(notif_id)
    .bind(&tenant_id)
    .execute(&mut *tx)
    .await?;

    let envelope = create_notifications_envelope(
        Uuid::new_v4(),
        tenant_id.clone(),
        "notifications.dlq.replayed".to_string(),
        None,
        None,
        "LIFECYCLE".to_string(),
        serde_json::json!({
            "notification_id": notif_id,
            "action": "replay",
            "previous_status": "dead_lettered",
            "new_status": "pending"
        }),
    );
    enqueue_event(&mut tx, "notifications.events.dlq.replayed", &envelope).await?;
    tx.commit().await?;

    let pending_after: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM scheduled_notifications \
         WHERE tenant_id = $1 AND status = 'dead_lettered'",
    )
    .bind(&tenant_id)
    .fetch_one(&pool)
    .await?;

    let new_status: Option<String> = sqlx::query_scalar(
        "SELECT status FROM scheduled_notifications WHERE id = $1 AND tenant_id = $2",
    )
    .bind(notif_id)
    .bind(&tenant_id)
    .fetch_optional(&pool)
    .await?;

    let outbox_rows: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM events_outbox \
         WHERE subject = 'notifications.events.dlq.replayed' \
           AND (payload->>'tenant_id') = $1 \
           AND (payload->'payload'->>'notification_id') = $2",
    )
    .bind(&tenant_id)
    .bind(notif_id.to_string())
    .fetch_one(&pool)
    .await?;

    println!("pending_after={pending_after}");
    println!("new_status={}", new_status.clone().unwrap_or_default());
    println!("replay_outbox_rows={outbox_rows}");

    if pending_before < 1 {
        return Err("drill failed: expected dead-lettered row before replay".into());
    }
    if pending_after != 0 {
        return Err("drill failed: dead-lettered row not cleared".into());
    }
    if new_status.as_deref() != Some("pending") {
        return Err("drill failed: replay did not reset status to pending".into());
    }
    if outbox_rows < 1 {
        return Err("drill failed: replay event missing from outbox".into());
    }

    println!("dlq_replay_drill=ok");
    Ok(())
}
