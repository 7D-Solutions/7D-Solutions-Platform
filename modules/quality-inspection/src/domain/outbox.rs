use crate::events::QualityInspectionEventType;

pub async fn enqueue_event<T: serde::Serialize>(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    tenant_id: &str,
    event_type: QualityInspectionEventType,
    aggregate_type: &str,
    aggregate_id: &str,
    envelope: &event_bus::EventEnvelope<T>,
    correlation_id: &str,
    causation_id: Option<&str>,
) -> Result<(), sqlx::Error> {
    let envelope_json = serde_json::to_string(envelope)
        .map_err(|e| sqlx::Error::Protocol(e.to_string()))?;

    sqlx::query(
        r#"
        INSERT INTO quality_inspection_outbox
            (event_id, event_type, aggregate_type, aggregate_id, tenant_id,
             payload, correlation_id, causation_id, schema_version)
        VALUES
            ($1, $2, $3, $4, $5, $6::JSONB, $7, $8, '1.0.0')
        "#,
    )
    .bind(envelope.event_id)
    .bind(event_type.as_str())
    .bind(aggregate_type)
    .bind(aggregate_id)
    .bind(tenant_id)
    .bind(&envelope_json)
    .bind(correlation_id)
    .bind(causation_id)
    .execute(&mut **tx)
    .await?;

    Ok(())
}
