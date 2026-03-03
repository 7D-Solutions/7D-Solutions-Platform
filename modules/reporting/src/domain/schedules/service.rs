use super::models::{
    DeliverySchedule, ScheduleCreatedPayload, ScheduleExecution, ScheduleTriggeredPayload,
};
use crate::domain::exports::models::ExportFormat;
use crate::domain::exports::service::run_export;
use event_bus::outbox::validate_and_serialize_envelope;
use event_bus::EventEnvelope;
use sqlx::PgPool;
use uuid::Uuid;

/// Create a new delivery schedule using Guard → Mutation → Outbox.
///
/// 1. Guard: check idempotency_key for duplicates
/// 2. Mutation: insert schedule row
/// 3. Outbox: enqueue schedule.created event atomically
pub async fn create_schedule(
    pool: &PgPool,
    tenant_id: &str,
    report_id: &str,
    schedule_name: &str,
    cron_expr: Option<&str>,
    interval_secs: Option<i32>,
    delivery_channel: &str,
    recipient: &str,
    format: &str,
    idempotency_key: Option<&str>,
) -> Result<DeliverySchedule, anyhow::Error> {
    // ── Guard: idempotency ──────────────────────────────────────────
    if let Some(key) = idempotency_key {
        let existing: Option<DeliverySchedule> = sqlx::query_as(
            "SELECT * FROM rpt_delivery_schedules WHERE tenant_id = $1 AND idempotency_key = $2",
        )
        .bind(tenant_id)
        .bind(key)
        .fetch_optional(pool)
        .await?;

        if let Some(schedule) = existing {
            return Ok(schedule);
        }
    }

    // ── Mutation + Outbox ───────────────────────────────────────────
    let schedule_id = Uuid::new_v4();
    let mut tx = pool.begin().await?;

    sqlx::query(
        r#"INSERT INTO rpt_delivery_schedules
               (id, tenant_id, report_id, schedule_name, cron_expr, interval_secs,
                delivery_channel, recipient, format, status, idempotency_key)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, 'active', $10)"#,
    )
    .bind(schedule_id)
    .bind(tenant_id)
    .bind(report_id)
    .bind(schedule_name)
    .bind(cron_expr)
    .bind(interval_secs)
    .bind(delivery_channel)
    .bind(recipient)
    .bind(format)
    .bind(idempotency_key)
    .execute(&mut *tx)
    .await?;

    // Outbox event
    let event_payload = ScheduleCreatedPayload {
        schedule_id,
        report_id: report_id.to_string(),
        delivery_channel: delivery_channel.to_string(),
        recipient: recipient.to_string(),
        format: format.to_string(),
    };

    let envelope = EventEnvelope::new(
        tenant_id.to_string(),
        "reporting".to_string(),
        "reporting.schedule.created".to_string(),
        event_payload,
    )
    .with_source_version(env!("CARGO_PKG_VERSION").to_string())
    .with_mutation_class(Some("SIDE_EFFECT".to_string()));

    let payload_json = validate_and_serialize_envelope(&envelope)
        .map_err(|e| anyhow::anyhow!("Envelope validation failed: {}", e))?;

    sqlx::query(
        r#"INSERT INTO events_outbox (
               event_id, event_type, aggregate_type, aggregate_id, payload,
               tenant_id, source_module, source_version, schema_version,
               occurred_at, replay_safe, trace_id, correlation_id, causation_id,
               reverses_event_id, supersedes_event_id, side_effect_id, mutation_class
           )
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18)"#,
    )
    .bind(envelope.event_id)
    .bind(&envelope.event_type)
    .bind("delivery_schedule")
    .bind(schedule_id.to_string())
    .bind(payload_json)
    .bind(&envelope.tenant_id)
    .bind(&envelope.source_module)
    .bind(&envelope.source_version)
    .bind(&envelope.schema_version)
    .bind(envelope.occurred_at)
    .bind(envelope.replay_safe)
    .bind(&envelope.trace_id)
    .bind(&envelope.correlation_id)
    .bind(&envelope.causation_id)
    .bind(envelope.reverses_event_id)
    .bind(envelope.supersedes_event_id)
    .bind(&envelope.side_effect_id)
    .bind(&envelope.mutation_class)
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;

    let schedule: DeliverySchedule = sqlx::query_as(
        "SELECT * FROM rpt_delivery_schedules WHERE id = $1 AND tenant_id = $2",
    )
    .bind(schedule_id)
    .bind(tenant_id)
    .fetch_one(pool)
    .await?;

    Ok(schedule)
}

/// Update schedule interval/cron and updated_at timestamp.
pub async fn update_schedule_interval(
    pool: &PgPool,
    tenant_id: &str,
    schedule_id: Uuid,
    cron_expr: Option<&str>,
    interval_secs: Option<i32>,
) -> Result<DeliverySchedule, anyhow::Error> {
    sqlx::query(
        r#"UPDATE rpt_delivery_schedules
           SET cron_expr = $1, interval_secs = $2, updated_at = NOW()
           WHERE id = $3 AND tenant_id = $4"#,
    )
    .bind(cron_expr)
    .bind(interval_secs)
    .bind(schedule_id)
    .bind(tenant_id)
    .execute(pool)
    .await?;

    let schedule: DeliverySchedule = sqlx::query_as(
        "SELECT * FROM rpt_delivery_schedules WHERE id = $1 AND tenant_id = $2",
    )
    .bind(schedule_id)
    .bind(tenant_id)
    .fetch_one(pool)
    .await?;

    Ok(schedule)
}

/// Disable a schedule (sets status to 'disabled').
pub async fn disable_schedule(
    pool: &PgPool,
    tenant_id: &str,
    schedule_id: Uuid,
) -> Result<(), anyhow::Error> {
    sqlx::query(
        r#"UPDATE rpt_delivery_schedules
           SET status = 'disabled', updated_at = NOW()
           WHERE id = $1 AND tenant_id = $2"#,
    )
    .bind(schedule_id)
    .bind(tenant_id)
    .execute(pool)
    .await?;

    Ok(())
}

/// Get a single schedule by ID (tenant-scoped).
pub async fn get_schedule(
    pool: &PgPool,
    tenant_id: &str,
    schedule_id: Uuid,
) -> Result<Option<DeliverySchedule>, sqlx::Error> {
    sqlx::query_as(
        "SELECT * FROM rpt_delivery_schedules WHERE id = $1 AND tenant_id = $2",
    )
    .bind(schedule_id)
    .bind(tenant_id)
    .fetch_optional(pool)
    .await
}

/// List all schedules for a tenant.
pub async fn list_schedules(
    pool: &PgPool,
    tenant_id: &str,
) -> Result<Vec<DeliverySchedule>, sqlx::Error> {
    sqlx::query_as(
        "SELECT * FROM rpt_delivery_schedules WHERE tenant_id = $1 ORDER BY created_at DESC",
    )
    .bind(tenant_id)
    .fetch_all(pool)
    .await
}

/// Trigger a schedule: create export run + delivery request + execution log.
///
/// Uses Guard → Mutation → Outbox:
/// 1. Guard: verify schedule is active
/// 2. Mutation: create export run via exports service, log execution
/// 3. Outbox: enqueue schedule.triggered event atomically with execution log
///
/// Returns None if the schedule is not active (disabled/paused).
pub async fn trigger_schedule(
    pool: &PgPool,
    tenant_id: &str,
    schedule_id: Uuid,
) -> Result<Option<ScheduleExecution>, anyhow::Error> {
    // ── Guard: schedule must be active ──────────────────────────────
    let schedule: DeliverySchedule = sqlx::query_as(
        "SELECT * FROM rpt_delivery_schedules WHERE id = $1 AND tenant_id = $2",
    )
    .bind(schedule_id)
    .bind(tenant_id)
    .fetch_one(pool)
    .await?;

    if schedule.status != "active" {
        return Ok(None);
    }

    // Parse export format from schedule
    let format = match schedule.format.as_str() {
        "csv" => ExportFormat::Csv,
        "xlsx" => ExportFormat::Xlsx,
        "pdf" => ExportFormat::Pdf,
        other => return Err(anyhow::anyhow!("Unknown format: {}", other)),
    };

    // ── Mutation: create export run ─────────────────────────────────
    let export_run = run_export(pool, tenant_id, &schedule.report_id, format, None).await?;

    // ── Mutation + Outbox: log execution atomically ─────────────────
    let execution_id = Uuid::new_v4();
    let mut tx = pool.begin().await?;

    sqlx::query(
        r#"INSERT INTO rpt_schedule_executions
               (id, schedule_id, tenant_id, export_run_id, status, completed_at)
           VALUES ($1, $2, $3, $4, 'completed', NOW())"#,
    )
    .bind(execution_id)
    .bind(schedule_id)
    .bind(tenant_id)
    .bind(export_run.id)
    .execute(&mut *tx)
    .await?;

    // Update schedule last_triggered_at
    sqlx::query(
        r#"UPDATE rpt_delivery_schedules
           SET last_triggered_at = NOW(), updated_at = NOW()
           WHERE id = $1 AND tenant_id = $2"#,
    )
    .bind(schedule_id)
    .bind(tenant_id)
    .execute(&mut *tx)
    .await?;

    // Outbox event for trigger
    let event_payload = ScheduleTriggeredPayload {
        schedule_id,
        execution_id,
        export_run_id: export_run.id,
        report_id: schedule.report_id.clone(),
        delivery_channel: schedule.delivery_channel.clone(),
        recipient: schedule.recipient.clone(),
    };

    let envelope = EventEnvelope::new(
        tenant_id.to_string(),
        "reporting".to_string(),
        "reporting.schedule.triggered".to_string(),
        event_payload,
    )
    .with_source_version(env!("CARGO_PKG_VERSION").to_string())
    .with_mutation_class(Some("SIDE_EFFECT".to_string()));

    let payload_json = validate_and_serialize_envelope(&envelope)
        .map_err(|e| anyhow::anyhow!("Envelope validation failed: {}", e))?;

    sqlx::query(
        r#"INSERT INTO events_outbox (
               event_id, event_type, aggregate_type, aggregate_id, payload,
               tenant_id, source_module, source_version, schema_version,
               occurred_at, replay_safe, trace_id, correlation_id, causation_id,
               reverses_event_id, supersedes_event_id, side_effect_id, mutation_class
           )
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18)"#,
    )
    .bind(envelope.event_id)
    .bind(&envelope.event_type)
    .bind("schedule_execution")
    .bind(execution_id.to_string())
    .bind(payload_json)
    .bind(&envelope.tenant_id)
    .bind(&envelope.source_module)
    .bind(&envelope.source_version)
    .bind(&envelope.schema_version)
    .bind(envelope.occurred_at)
    .bind(envelope.replay_safe)
    .bind(&envelope.trace_id)
    .bind(&envelope.correlation_id)
    .bind(&envelope.causation_id)
    .bind(envelope.reverses_event_id)
    .bind(envelope.supersedes_event_id)
    .bind(&envelope.side_effect_id)
    .bind(&envelope.mutation_class)
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;

    let execution: ScheduleExecution = sqlx::query_as(
        "SELECT * FROM rpt_schedule_executions WHERE id = $1 AND tenant_id = $2",
    )
    .bind(execution_id)
    .bind(tenant_id)
    .fetch_one(pool)
    .await?;

    Ok(Some(execution))
}

/// List executions for a specific schedule (tenant-scoped).
pub async fn list_executions(
    pool: &PgPool,
    tenant_id: &str,
    schedule_id: Uuid,
) -> Result<Vec<ScheduleExecution>, sqlx::Error> {
    sqlx::query_as(
        r#"SELECT * FROM rpt_schedule_executions
           WHERE schedule_id = $1 AND tenant_id = $2
           ORDER BY triggered_at DESC"#,
    )
    .bind(schedule_id)
    .bind(tenant_id)
    .fetch_all(pool)
    .await
}
