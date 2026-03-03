use gl_rs::contracts::gl_posting_request_v1::GlPostingRequestV1;
use gl_rs::services::journal_service::process_gl_posting_request;
use sqlx::{postgres::PgPoolOptions, Row};
use uuid::Uuid;

const DEFAULT_DB_URL: &str = "postgresql://gl_user:gl_pass@localhost:5438/gl_db";

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let db_url = std::env::var("DATABASE_URL").unwrap_or_else(|_| DEFAULT_DB_URL.to_string());
    let pool = PgPoolOptions::new().max_connections(5).connect(&db_url).await?;
    sqlx::migrate!("./db/migrations").run(&pool).await?;

    let tenant_id = format!("dlq-drill-{}", Uuid::new_v4());
    setup_fixtures(&pool, &tenant_id).await?;

    let event_id = Uuid::new_v4();
    let envelope = serde_json::json!({
        "event_id": event_id,
        "occurred_at": chrono::Utc::now(),
        "tenant_id": tenant_id,
        "source_module": "ar",
        "source_version": "1.0.0",
        "event_type": "gl.events.posting.requested",
        "correlation_id": Uuid::new_v4().to_string(),
        "payload": {
            "posting_date": "2026-01-15",
            "currency": "USD",
            "source_doc_type": "AR_INVOICE",
            "source_doc_id": format!("drill-invoice-{}", event_id),
            "description": "DLQ replay drill posting",
            "lines": [
                { "account_ref": "1100", "debit": 125.0, "credit": 0.0, "memo": "AR" },
                { "account_ref": "4000", "debit": 0.0, "credit": 125.0, "memo": "Revenue" }
            ]
        }
    });

    sqlx::query(
        r#"
        INSERT INTO failed_events (event_id, subject, tenant_id, envelope_json, error, retry_count)
        VALUES ($1, $2, $3, $4, $5, $6)
        "#,
    )
    .bind(event_id)
    .bind("gl.events.posting.requested")
    .bind(&tenant_id)
    .bind(&envelope)
    .bind("drill: synthetic DLQ entry")
    .bind(3_i32)
    .execute(&pool)
    .await?;

    let pending_before: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM failed_events WHERE subject = 'gl.events.posting.requested' AND tenant_id = $1",
    )
    .bind(&tenant_id)
    .fetch_one(&pool)
    .await?;
    println!("pending_before={pending_before}");

    let failed_row = sqlx::query(
        "SELECT event_id, tenant_id, subject, envelope_json FROM failed_events WHERE event_id = $1",
    )
    .bind(event_id)
    .fetch_one(&pool)
    .await?;

    let replay_event_id: Uuid = failed_row.try_get("event_id")?;
    let replay_tenant_id: String = failed_row.try_get("tenant_id")?;
    let replay_subject: String = failed_row.try_get("subject")?;
    let replay_envelope: serde_json::Value = failed_row.try_get("envelope_json")?;

    let source_module = replay_envelope
        .get("source_module")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");

    let correlation_id = replay_envelope
        .get("correlation_id")
        .and_then(|v| v.as_str())
        .and_then(|s| Uuid::parse_str(s).ok());

    let payload: GlPostingRequestV1 = serde_json::from_value(
        replay_envelope
            .get("payload")
            .cloned()
            .ok_or("missing payload in envelope_json")?,
    )?;

    let posted_entry_id = process_gl_posting_request(
        &pool,
        replay_event_id,
        &replay_tenant_id,
        source_module,
        &replay_subject,
        &payload,
        correlation_id,
    )
    .await?;

    sqlx::query("DELETE FROM failed_events WHERE event_id = $1")
        .bind(replay_event_id)
        .execute(&pool)
        .await?;

    let pending_after: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM failed_events WHERE subject = 'gl.events.posting.requested' AND tenant_id = $1",
    )
    .bind(&tenant_id)
    .fetch_one(&pool)
    .await?;

    let journal_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM journal_entries WHERE source_event_id = $1 AND tenant_id = $2",
    )
    .bind(replay_event_id)
    .bind(&tenant_id)
    .fetch_one(&pool)
    .await?;

    println!("replayed_event_id={replay_event_id}");
    println!("posted_entry_id={posted_entry_id}");
    println!("journal_entries_for_event={journal_count}");
    println!("pending_after={pending_after}");

    if pending_before < 1 {
        return Err("drill failed: expected at least one pending DLQ row before replay".into());
    }
    if journal_count != 1 {
        return Err("drill failed: replay did not produce exactly one journal entry".into());
    }
    if pending_after != 0 {
        return Err("drill failed: replayed DLQ row was not cleared".into());
    }

    println!("dlq_replay_drill=ok");
    Ok(())
}

async fn setup_fixtures(pool: &sqlx::PgPool, tenant_id: &str) -> Result<(), sqlx::Error> {
    let mut tx = pool.begin().await?;

    sqlx::query(
        r#"
        INSERT INTO accounts (id, tenant_id, code, name, type, normal_balance, is_active, created_at)
        VALUES
          ($1, $2, '1100', 'Accounts Receivable', 'asset', 'debit', true, NOW()),
          ($3, $2, '4000', 'Product Revenue', 'revenue', 'credit', true, NOW())
        "#,
    )
    .bind(Uuid::new_v4())
    .bind(tenant_id)
    .bind(Uuid::new_v4())
    .execute(&mut *tx)
    .await?;

    sqlx::query(
        r#"
        INSERT INTO accounting_periods (id, tenant_id, period_start, period_end, is_closed, created_at)
        VALUES ($1, $2, '2026-01-01', '2026-01-31', false, NOW())
        "#,
    )
    .bind(Uuid::new_v4())
    .bind(tenant_id)
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;
    Ok(())
}
