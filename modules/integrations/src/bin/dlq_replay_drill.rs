use sqlx::{postgres::PgPoolOptions, Row};
use uuid::Uuid;

const DEFAULT_DB_URL: &str =
    "postgres://integrations_user:integrations_pass@localhost:5449/integrations_db";

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let db_url = std::env::var("DATABASE_URL").unwrap_or_else(|_| DEFAULT_DB_URL.to_string());
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&db_url)
        .await?;
    sqlx::migrate!("./db/migrations").run(&pool).await?;

    let app_id = format!("dlq-drill-{}", Uuid::new_v4());
    let event_id = Uuid::new_v4();

    // ── 1. Build a synthetic DLQ entry for an external_ref.created event ──
    let envelope = serde_json::json!({
        "event_id": event_id.to_string(),
        "occurred_at": chrono::Utc::now().to_rfc3339(),
        "tenant_id": &app_id,
        "source_module": "integrations",
        "source_version": env!("CARGO_PKG_VERSION"),
        "event_type": "external_ref.created",
        "schema_version": "1.0.0",
        "mutation_class": "DATA_MUTATION",
        "replay_safe": true,
        "correlation_id": Uuid::new_v4().to_string(),
        "payload": {
            "ref_id": 0,
            "app_id": &app_id,
            "entity_type": "invoice",
            "entity_id": format!("drill-inv-{}", event_id),
            "system": "drill-system",
            "external_id": format!("drill-ext-{}", event_id),
            "label": null,
            "created_at": chrono::Utc::now().to_rfc3339()
        }
    });

    // ── 2. Insert into failed_events (DLQ) ────────────────────────────────
    sqlx::query(
        r#"
        INSERT INTO failed_events (event_id, subject, tenant_id, envelope_json, error, retry_count)
        VALUES ($1, $2, $3, $4, $5, $6)
        "#,
    )
    .bind(event_id)
    .bind("integrations.events.external_ref.created")
    .bind(&app_id)
    .bind(&envelope)
    .bind("drill: synthetic DLQ entry")
    .bind(3_i32)
    .execute(&pool)
    .await?;

    // ── 3. Check pending count before replay ──────────────────────────────
    let pending_before: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM failed_events WHERE tenant_id = $1")
            .bind(&app_id)
            .fetch_one(&pool)
            .await?;
    println!("pending_before={pending_before}");

    // ── 4. Replay: read the DLQ row and re-execute the domain operation ──
    let failed_row = sqlx::query(
        "SELECT event_id, tenant_id, subject, envelope_json FROM failed_events WHERE event_id = $1",
    )
    .bind(event_id)
    .fetch_one(&pool)
    .await?;

    let replay_event_id: Uuid = failed_row.try_get("event_id")?;
    let replay_tenant_id: String = failed_row.try_get("tenant_id")?;
    let replay_envelope: serde_json::Value = failed_row.try_get("envelope_json")?;

    let payload = replay_envelope
        .get("payload")
        .ok_or("missing payload in envelope_json")?;

    let entity_type = payload
        .get("entity_type")
        .and_then(|v| v.as_str())
        .ok_or("missing entity_type")?;
    let entity_id = payload
        .get("entity_id")
        .and_then(|v| v.as_str())
        .ok_or("missing entity_id")?;
    let system = payload
        .get("system")
        .and_then(|v| v.as_str())
        .ok_or("missing system")?;
    let external_id = payload
        .get("external_id")
        .and_then(|v| v.as_str())
        .ok_or("missing external_id")?;

    // Re-execute the create_external_ref operation (Guard → Mutation → Outbox)
    let mut tx = pool.begin().await?;

    let ref_row: (i64,) = sqlx::query_as(
        r#"
        INSERT INTO integrations_external_refs
            (app_id, entity_type, entity_id, system, external_id, label, created_at, updated_at)
        VALUES ($1, $2, $3, $4, $5, NULL, NOW(), NOW())
        ON CONFLICT (app_id, system, external_id) DO UPDATE SET updated_at = NOW()
        RETURNING id
        "#,
    )
    .bind(&replay_tenant_id)
    .bind(entity_type)
    .bind(entity_id)
    .bind(system)
    .bind(external_id)
    .fetch_one(&mut *tx)
    .await?;

    let outbox_event_id = Uuid::new_v4();
    sqlx::query(
        r#"
        INSERT INTO integrations_outbox
            (event_id, event_type, aggregate_type, aggregate_id, app_id, payload, schema_version)
        VALUES ($1, $2, $3, $4, $5, $6, '1.0.0')
        "#,
    )
    .bind(outbox_event_id)
    .bind("external_ref.created")
    .bind("external_ref")
    .bind(ref_row.0.to_string())
    .bind(&replay_tenant_id)
    .bind(payload)
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;

    // ── 5. Clear the DLQ row ──────────────────────────────────────────────
    sqlx::query("DELETE FROM failed_events WHERE event_id = $1")
        .bind(replay_event_id)
        .execute(&pool)
        .await?;

    // ── 6. Verify ─────────────────────────────────────────────────────────
    let pending_after: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM failed_events WHERE tenant_id = $1")
            .bind(&app_id)
            .fetch_one(&pool)
            .await?;

    let ref_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM integrations_external_refs WHERE app_id = $1 AND system = $2 AND external_id = $3",
    )
    .bind(&app_id)
    .bind(system)
    .bind(external_id)
    .fetch_one(&pool)
    .await?;

    let outbox_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM integrations_outbox WHERE app_id = $1 AND event_type = 'external_ref.created'",
    )
    .bind(&app_id)
    .fetch_one(&pool)
    .await?;

    println!("replayed_event_id={replay_event_id}");
    println!("ref_created={ref_count}");
    println!("outbox_events={outbox_count}");
    println!("pending_after={pending_after}");

    // ── 7. Assertions ─────────────────────────────────────────────────────
    if pending_before < 1 {
        return Err("drill failed: expected at least one pending DLQ row before replay".into());
    }
    if ref_count != 1 {
        return Err("drill failed: replay did not produce exactly one external ref".into());
    }
    if outbox_count < 1 {
        return Err("drill failed: replay did not produce outbox event".into());
    }
    if pending_after != 0 {
        return Err("drill failed: replayed DLQ row was not cleared".into());
    }

    // Cleanup drill data
    sqlx::query("DELETE FROM integrations_outbox WHERE app_id = $1")
        .bind(&app_id)
        .execute(&pool)
        .await
        .ok();
    sqlx::query("DELETE FROM integrations_external_refs WHERE app_id = $1")
        .bind(&app_id)
        .execute(&pool)
        .await
        .ok();

    println!("dlq_replay_drill=ok");
    Ok(())
}
