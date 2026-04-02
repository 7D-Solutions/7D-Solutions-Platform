use chrono::{DateTime, Utc};
use event_bus::EventEnvelope;
use serde::Serialize;
use serde_json::{json, Value};
use sqlx::{Postgres, Row, Transaction};
use uuid::Uuid;

#[derive(Debug, Clone, Copy)]
pub enum LifecycleEventType {
    UserCreated,
    RoleAssigned,
    RoleRevoked,
    AccessReviewRecorded,
}

impl LifecycleEventType {
    fn as_str(self) -> &'static str {
        match self {
            Self::UserCreated => "user_created",
            Self::RoleAssigned => "role_assigned",
            Self::RoleRevoked => "role_revoked",
            Self::AccessReviewRecorded => "access_review_recorded",
        }
    }

    fn schema_version(self) -> &'static str {
        match self {
            Self::UserCreated => "auth.user.lifecycle.user_created/v1",
            Self::RoleAssigned => "auth.user.lifecycle.role_assigned/v1",
            Self::RoleRevoked => "auth.user.lifecycle.role_revoked/v1",
            Self::AccessReviewRecorded => "auth.user.lifecycle.access_review_recorded/v1",
        }
    }

    fn event_subject(self) -> &'static str {
        match self {
            Self::UserCreated => "auth.user.lifecycle.user_created",
            Self::RoleAssigned => "auth.user.lifecycle.role_assigned",
            Self::RoleRevoked => "auth.user.lifecycle.role_revoked",
            Self::AccessReviewRecorded => "auth.user.lifecycle.access_review_recorded",
        }
    }
}

#[derive(Debug, Clone)]
pub struct LifecycleAuditContext {
    pub producer: String,
    pub trace_id: String,
    pub causation_id: Option<Uuid>,
    pub idempotency_key: String,
}

#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct LifecycleTimelineEntry {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub user_id: Uuid,
    pub event_type: String,
    pub actor_user_id: Option<Uuid>,
    pub role_id: Option<Uuid>,
    pub review_id: Option<Uuid>,
    pub decision: Option<String>,
    pub idempotency_key: String,
    pub event_payload: Value,
    pub occurred_at: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
}

pub async fn append_lifecycle_event_tx(
    tx: &mut Transaction<'_, Postgres>,
    tenant_id: Uuid,
    user_id: Uuid,
    event_type: LifecycleEventType,
    actor_user_id: Option<Uuid>,
    role_id: Option<Uuid>,
    review_id: Option<Uuid>,
    decision: Option<&str>,
    payload: Value,
    ctx: &LifecycleAuditContext,
) -> Result<Option<Uuid>, sqlx::Error> {
    let occurred_at = Utc::now();

    let inserted = sqlx::query(
        r#"
        INSERT INTO user_lifecycle_audit_events (
            tenant_id,
            user_id,
            event_type,
            actor_user_id,
            role_id,
            review_id,
            decision,
            idempotency_key,
            event_payload,
            occurred_at
        )
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
        ON CONFLICT (tenant_id, idempotency_key) DO NOTHING
        RETURNING id
        "#,
    )
    .bind(tenant_id)
    .bind(user_id)
    .bind(event_type.as_str())
    .bind(actor_user_id)
    .bind(role_id)
    .bind(review_id)
    .bind(decision)
    .bind(&ctx.idempotency_key)
    .bind(payload.clone())
    .bind(occurred_at)
    .fetch_optional(&mut **tx)
    .await?;

    let Some(row) = inserted else {
        return Ok(None);
    };

    let event_id: Uuid = row.get("id");

    let envelope = EventEnvelope::with_event_id(
        event_id,
        tenant_id.to_string(),
        ctx.producer.clone(),
        event_type.event_subject().to_string(),
        payload,
    )
    .with_schema_version(event_type.schema_version().to_string())
    .with_trace_id(Some(ctx.trace_id.clone()))
    .with_causation_id(ctx.causation_id.map(|u| u.to_string()))
    .with_mutation_class(Some("user-data".to_string()));

    let outbox_payload =
        serde_json::to_value(&envelope).map_err(|e| sqlx::Error::Protocol(e.to_string()))?;

    sqlx::query(
        r#"
        INSERT INTO user_lifecycle_events_outbox (
            event_id,
            tenant_id,
            aggregate_id,
            event_type,
            payload
        )
        VALUES ($1, $2, $3, $4, $5)
        "#,
    )
    .bind(event_id)
    .bind(tenant_id)
    .bind(user_id)
    .bind(event_type.event_subject())
    .bind(outbox_payload)
    .execute(&mut **tx)
    .await?;

    Ok(Some(event_id))
}

pub async fn record_access_review_decision(
    pool: &sqlx::PgPool,
    tenant_id: Uuid,
    user_id: Uuid,
    reviewed_by: Uuid,
    decision: &str,
    review_id: Uuid,
    notes: Option<&str>,
    ctx: &LifecycleAuditContext,
) -> Result<Option<Uuid>, sqlx::Error> {
    let mut tx = pool.begin().await?;
    let payload = json!({
        "user_id": user_id,
        "reviewed_by": reviewed_by,
        "review_id": review_id,
        "decision": decision,
        "notes": notes,
    });

    let event_id = append_lifecycle_event_tx(
        &mut tx,
        tenant_id,
        user_id,
        LifecycleEventType::AccessReviewRecorded,
        Some(reviewed_by),
        None,
        Some(review_id),
        Some(decision),
        payload,
        ctx,
    )
    .await?;

    tx.commit().await?;
    Ok(event_id)
}

pub async fn list_user_lifecycle_timeline(
    pool: &sqlx::PgPool,
    tenant_id: Uuid,
    user_id: Uuid,
) -> Result<Vec<LifecycleTimelineEntry>, sqlx::Error> {
    let rows = sqlx::query(
        r#"
        SELECT
            id,
            tenant_id,
            user_id,
            event_type,
            actor_user_id,
            role_id,
            review_id,
            decision,
            idempotency_key,
            event_payload,
            occurred_at,
            created_at
        FROM user_lifecycle_audit_events
        WHERE tenant_id = $1 AND user_id = $2
        ORDER BY occurred_at ASC, created_at ASC, id ASC
        "#,
    )
    .bind(tenant_id)
    .bind(user_id)
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|row| LifecycleTimelineEntry {
            id: row.get("id"),
            tenant_id: row.get("tenant_id"),
            user_id: row.get("user_id"),
            event_type: row.get("event_type"),
            actor_user_id: row.get("actor_user_id"),
            role_id: row.get("role_id"),
            review_id: row.get("review_id"),
            decision: row.get("decision"),
            idempotency_key: row.get("idempotency_key"),
            event_payload: row.get("event_payload"),
            occurred_at: row.get("occurred_at"),
            created_at: row.get("created_at"),
        })
        .collect())
}
