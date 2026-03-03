use super::models::{
    DashboardLayout, DashboardLayoutCreatedPayload, DashboardLayoutUpdatedPayload,
    DashboardWidget, WidgetInput,
};
use event_bus::outbox::validate_and_serialize_envelope;
use event_bus::EventEnvelope;
use sqlx::PgPool;
use uuid::Uuid;

/// Create a dashboard layout with widgets using Guard → Mutation → Outbox.
///
/// 1. Guard: check idempotency_key for duplicates
/// 2. Mutation: insert layout + widgets atomically
/// 3. Outbox: enqueue creation event in same transaction
pub async fn create_layout(
    pool: &PgPool,
    tenant_id: &str,
    name: &str,
    description: Option<&str>,
    widgets: &[WidgetInput],
    idempotency_key: Option<&str>,
) -> Result<DashboardLayout, anyhow::Error> {
    // ── Guard: idempotency ──────────────────────────────────────────
    if let Some(key) = idempotency_key {
        let existing: Option<DashboardLayout> = sqlx::query_as(
            "SELECT * FROM rpt_dashboard_layouts WHERE tenant_id = $1 AND idempotency_key = $2",
        )
        .bind(tenant_id)
        .bind(key)
        .fetch_optional(pool)
        .await?;

        if let Some(layout) = existing {
            return Ok(layout);
        }
    }

    // ── Mutation + Outbox: create layout, widgets, and event atomically ──
    let layout_id = Uuid::new_v4();
    let mut tx = pool.begin().await?;

    sqlx::query(
        r#"INSERT INTO rpt_dashboard_layouts (id, tenant_id, name, description, idempotency_key)
           VALUES ($1, $2, $3, $4, $5)"#,
    )
    .bind(layout_id)
    .bind(tenant_id)
    .bind(name)
    .bind(description)
    .bind(idempotency_key)
    .execute(&mut *tx)
    .await?;

    for widget in widgets {
        sqlx::query(
            r#"INSERT INTO rpt_dashboard_widgets
                   (id, layout_id, tenant_id, widget_type, title, report_query,
                    position_x, position_y, width, height, display_config)
               VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)"#,
        )
        .bind(Uuid::new_v4())
        .bind(layout_id)
        .bind(tenant_id)
        .bind(&widget.widget_type)
        .bind(&widget.title)
        .bind(&widget.report_query)
        .bind(widget.position_x)
        .bind(widget.position_y)
        .bind(widget.width)
        .bind(widget.height)
        .bind(&widget.display_config)
        .execute(&mut *tx)
        .await?;
    }

    // Outbox event
    let event_payload = DashboardLayoutCreatedPayload {
        layout_id,
        name: name.to_string(),
        widget_count: widgets.len() as i32,
    };

    let envelope = EventEnvelope::new(
        tenant_id.to_string(),
        "reporting".to_string(),
        "reporting.dashboard_layout.created".to_string(),
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
    .bind("dashboard_layout")
    .bind(layout_id.to_string())
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

    // ── Return created layout ─────────────────────────────────────────
    let layout: DashboardLayout = sqlx::query_as(
        "SELECT * FROM rpt_dashboard_layouts WHERE id = $1 AND tenant_id = $2",
    )
    .bind(layout_id)
    .bind(tenant_id)
    .fetch_one(pool)
    .await?;

    Ok(layout)
}

/// Get a dashboard layout by ID (tenant-scoped).
pub async fn get_layout(
    pool: &PgPool,
    tenant_id: &str,
    layout_id: Uuid,
) -> Result<Option<DashboardLayout>, sqlx::Error> {
    sqlx::query_as(
        "SELECT * FROM rpt_dashboard_layouts WHERE id = $1 AND tenant_id = $2",
    )
    .bind(layout_id)
    .bind(tenant_id)
    .fetch_optional(pool)
    .await
}

/// List all dashboard layouts for a tenant.
pub async fn list_layouts(
    pool: &PgPool,
    tenant_id: &str,
) -> Result<Vec<DashboardLayout>, sqlx::Error> {
    sqlx::query_as(
        "SELECT * FROM rpt_dashboard_layouts WHERE tenant_id = $1 ORDER BY created_at DESC",
    )
    .bind(tenant_id)
    .fetch_all(pool)
    .await
}

/// Get all widgets for a layout (tenant-scoped).
pub async fn get_widgets(
    pool: &PgPool,
    tenant_id: &str,
    layout_id: Uuid,
) -> Result<Vec<DashboardWidget>, sqlx::Error> {
    sqlx::query_as(
        r#"SELECT * FROM rpt_dashboard_widgets
           WHERE layout_id = $1 AND tenant_id = $2
           ORDER BY position_y, position_x"#,
    )
    .bind(layout_id)
    .bind(tenant_id)
    .fetch_all(pool)
    .await
}

/// Update widget positions within a layout using Guard → Mutation → Outbox.
///
/// 1. Guard: verify layout exists and belongs to tenant
/// 2. Mutation: update widget positions, bump layout version
/// 3. Outbox: enqueue update event atomically
pub async fn update_widget_positions(
    pool: &PgPool,
    tenant_id: &str,
    layout_id: Uuid,
    updates: &[(Uuid, i32, i32)], // (widget_id, new_x, new_y)
) -> Result<DashboardLayout, anyhow::Error> {
    // ── Guard: verify layout ownership ──────────────────────────────
    let layout: Option<DashboardLayout> = sqlx::query_as(
        "SELECT * FROM rpt_dashboard_layouts WHERE id = $1 AND tenant_id = $2",
    )
    .bind(layout_id)
    .bind(tenant_id)
    .fetch_optional(pool)
    .await?;

    let layout = layout.ok_or_else(|| anyhow::anyhow!("Layout not found or access denied"))?;

    // ── Mutation + Outbox: update positions and emit event atomically ──
    let new_version = layout.version + 1;
    let mut tx = pool.begin().await?;

    sqlx::query(
        r#"UPDATE rpt_dashboard_layouts
           SET version = $1, updated_at = NOW()
           WHERE id = $2 AND tenant_id = $3"#,
    )
    .bind(new_version)
    .bind(layout_id)
    .bind(tenant_id)
    .execute(&mut *tx)
    .await?;

    for (widget_id, new_x, new_y) in updates {
        sqlx::query(
            r#"UPDATE rpt_dashboard_widgets
               SET position_x = $1, position_y = $2, updated_at = NOW()
               WHERE id = $3 AND layout_id = $4 AND tenant_id = $5"#,
        )
        .bind(new_x)
        .bind(new_y)
        .bind(widget_id)
        .bind(layout_id)
        .bind(tenant_id)
        .execute(&mut *tx)
        .await?;
    }

    // Outbox event
    let event_payload = DashboardLayoutUpdatedPayload {
        layout_id,
        name: layout.name.clone(),
        version: new_version,
    };

    let envelope = EventEnvelope::new(
        tenant_id.to_string(),
        "reporting".to_string(),
        "reporting.dashboard_layout.updated".to_string(),
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
    .bind("dashboard_layout")
    .bind(layout_id.to_string())
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

    // ── Return updated layout ─────────────────────────────────────────
    let updated: DashboardLayout = sqlx::query_as(
        "SELECT * FROM rpt_dashboard_layouts WHERE id = $1 AND tenant_id = $2",
    )
    .bind(layout_id)
    .bind(tenant_id)
    .fetch_one(pool)
    .await?;

    Ok(updated)
}
