use chrono::{DateTime, Utc};
use event_bus::EventEnvelope;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sqlx::{Postgres, Row, Transaction};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SodPolicy {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub action_key: String,
    pub primary_role_id: Uuid,
    pub conflicting_role_id: Uuid,
    pub allow_override: bool,
    pub override_requires_approval: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SodPolicyUpsert {
    pub tenant_id: Uuid,
    pub action_key: String,
    pub primary_role_id: Uuid,
    pub conflicting_role_id: Uuid,
    pub allow_override: bool,
    pub override_requires_approval: bool,
    pub actor_user_id: Option<Uuid>,
    pub idempotency_key: String,
    pub trace_id: String,
    pub causation_id: Option<Uuid>,
    pub producer: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SodPolicyUpsertResult {
    pub policy: SodPolicy,
    pub idempotent_replay: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SodDecisionRequest {
    pub tenant_id: Uuid,
    pub action_key: String,
    pub actor_user_id: Uuid,
    pub subject_user_id: Option<Uuid>,
    pub override_granted_by: Option<Uuid>,
    pub override_ticket: Option<String>,
    pub idempotency_key: String,
    pub trace_id: String,
    pub causation_id: Option<Uuid>,
    pub producer: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SodDecisionResult {
    pub decision: String,
    pub reason: String,
    pub matched_policy_ids: Vec<Uuid>,
    pub idempotent_replay: bool,
}

pub async fn upsert_policy(
    pool: &sqlx::PgPool,
    req: SodPolicyUpsert,
) -> Result<SodPolicyUpsertResult, sqlx::Error> {
    let mut tx = pool.begin().await?;

    let mutation = sqlx::query(
        r#"
        INSERT INTO sod_policy_mutations (
            tenant_id,
            idempotency_key,
            actor_user_id,
            mutation_type,
            mutation_payload
        )
        VALUES ($1, $2, $3, 'policy_upsert', $4)
        ON CONFLICT (tenant_id, idempotency_key) DO NOTHING
        RETURNING id
        "#,
    )
    .bind(req.tenant_id)
    .bind(&req.idempotency_key)
    .bind(req.actor_user_id)
    .bind(json!({
        "action_key": req.action_key,
        "primary_role_id": req.primary_role_id,
        "conflicting_role_id": req.conflicting_role_id,
        "allow_override": req.allow_override,
        "override_requires_approval": req.override_requires_approval,
    }))
    .fetch_optional(&mut *tx)
    .await?;

    let replay = mutation.is_none();

    let policy = sqlx::query(
        r#"
        INSERT INTO sod_policies (
            tenant_id,
            action_key,
            primary_role_id,
            conflicting_role_id,
            allow_override,
            override_requires_approval
        )
        VALUES ($1, $2, $3, $4, $5, $6)
        ON CONFLICT (
            tenant_id,
            action_key,
            LEAST(primary_role_id, conflicting_role_id),
            GREATEST(primary_role_id, conflicting_role_id)
        )
        DO UPDATE SET
            allow_override = EXCLUDED.allow_override,
            override_requires_approval = EXCLUDED.override_requires_approval,
            updated_at = NOW()
        RETURNING id, tenant_id, action_key, primary_role_id, conflicting_role_id,
                  allow_override, override_requires_approval, created_at, updated_at
        "#,
    )
    .bind(req.tenant_id)
    .bind(req.action_key.clone())
    .bind(req.primary_role_id)
    .bind(req.conflicting_role_id)
    .bind(req.allow_override)
    .bind(req.override_requires_approval)
    .fetch_one(&mut *tx)
    .await?;

    let policy = SodPolicy {
        id: policy.get("id"),
        tenant_id: policy.get("tenant_id"),
        action_key: policy.get("action_key"),
        primary_role_id: policy.get("primary_role_id"),
        conflicting_role_id: policy.get("conflicting_role_id"),
        allow_override: policy.get("allow_override"),
        override_requires_approval: policy.get("override_requires_approval"),
        created_at: policy.get("created_at"),
        updated_at: policy.get("updated_at"),
    };

    if !replay {
        append_sod_outbox_event_tx(
            &mut tx,
            req.tenant_id,
            policy.id,
            "auth.sod.policy.upserted",
            "auth.sod.policy.upserted/v1",
            req.trace_id,
            req.causation_id,
            req.producer,
            json!({
                "policy_id": policy.id,
                "tenant_id": policy.tenant_id,
                "action_key": policy.action_key,
                "primary_role_id": policy.primary_role_id,
                "conflicting_role_id": policy.conflicting_role_id,
                "allow_override": policy.allow_override,
                "override_requires_approval": policy.override_requires_approval,
                "idempotency_key": req.idempotency_key,
            }),
        )
        .await?;
    }

    tx.commit().await?;

    Ok(SodPolicyUpsertResult {
        policy,
        idempotent_replay: replay,
    })
}

pub async fn evaluate_decision(
    pool: &sqlx::PgPool,
    req: SodDecisionRequest,
) -> Result<SodDecisionResult, sqlx::Error> {
    let mut tx = pool.begin().await?;

    // Guard: evaluate the actor's active role set against tenant policy rules.
    let actor_roles = sqlx::query_scalar::<_, Uuid>(
        r#"
        SELECT role_id
        FROM user_role_bindings
        WHERE tenant_id = $1 AND user_id = $2 AND revoked_at IS NULL
        "#,
    )
    .bind(req.tenant_id)
    .bind(req.actor_user_id)
    .fetch_all(&mut *tx)
    .await?;

    let policy_rows = if actor_roles.is_empty() {
        vec![]
    } else {
        sqlx::query(
            r#"
            SELECT id, allow_override, override_requires_approval
            FROM sod_policies
            WHERE tenant_id = $1
              AND action_key = $2
              AND primary_role_id = ANY($3)
              AND conflicting_role_id = ANY($3)
            "#,
        )
        .bind(req.tenant_id)
        .bind(req.action_key.clone())
        .bind(&actor_roles)
        .fetch_all(&mut *tx)
        .await?
    };

    let matched_ids = policy_rows
        .iter()
        .map(|r| r.get::<Uuid, _>("id"))
        .collect::<Vec<_>>();

    let mut decision = "allow".to_string();
    let mut reason = "no_sod_conflict".to_string();

    if !policy_rows.is_empty() {
        let all_allow_override = policy_rows
            .iter()
            .all(|r| r.get::<bool, _>("allow_override"));
        let any_requires_approval = policy_rows
            .iter()
            .any(|r| r.get::<bool, _>("override_requires_approval"));

        let override_present = req.override_granted_by.is_some();
        if all_allow_override && override_present {
            decision = "allow_with_override".to_string();
            reason = if any_requires_approval {
                "sod_override_approved".to_string()
            } else {
                "sod_override_allowed".to_string()
            };
        } else {
            decision = "deny".to_string();
            reason = if all_allow_override {
                "sod_conflict_override_required".to_string()
            } else {
                "sod_conflict_forbidden_combination".to_string()
            };
        }
    }

    let insert = sqlx::query(
        r#"
        INSERT INTO sod_decision_logs (
            tenant_id,
            idempotency_key,
            action_key,
            actor_user_id,
            subject_user_id,
            decision,
            reason,
            matched_policy_ids,
            override_granted_by,
            override_ticket,
            decision_payload
        )
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)
        ON CONFLICT (tenant_id, idempotency_key) DO NOTHING
        RETURNING id
        "#,
    )
    .bind(req.tenant_id)
    .bind(&req.idempotency_key)
    .bind(req.action_key.clone())
    .bind(req.actor_user_id)
    .bind(req.subject_user_id)
    .bind(decision.clone())
    .bind(reason.clone())
    .bind(&matched_ids)
    .bind(req.override_granted_by)
    .bind(req.override_ticket.clone())
    .bind(json!({
        "actor_roles": actor_roles,
        "matched_policy_ids": matched_ids,
    }))
    .fetch_optional(&mut *tx)
    .await?;

    let replay = insert.is_none();

    if replay {
        let existing = sqlx::query(
            r#"
            SELECT decision, reason, matched_policy_ids
            FROM sod_decision_logs
            WHERE tenant_id = $1 AND idempotency_key = $2
            "#,
        )
        .bind(req.tenant_id)
        .bind(&req.idempotency_key)
        .fetch_one(&mut *tx)
        .await?;

        tx.commit().await?;

        return Ok(SodDecisionResult {
            decision: existing.get("decision"),
            reason: existing.get("reason"),
            matched_policy_ids: existing.get("matched_policy_ids"),
            idempotent_replay: true,
        });
    }

    append_sod_outbox_event_tx(
        &mut tx,
        req.tenant_id,
        req.actor_user_id,
        "auth.sod.decision.recorded",
        "auth.sod.decision.recorded/v1",
        req.trace_id,
        req.causation_id,
        req.producer,
        json!({
            "action_key": req.action_key,
            "actor_user_id": req.actor_user_id,
            "subject_user_id": req.subject_user_id,
            "decision": decision,
            "reason": reason,
            "matched_policy_ids": matched_ids,
            "override_granted_by": req.override_granted_by,
            "override_ticket": req.override_ticket,
            "idempotency_key": req.idempotency_key,
        }),
    )
    .await?;

    tx.commit().await?;

    Ok(SodDecisionResult {
        decision,
        reason,
        matched_policy_ids: matched_ids,
        idempotent_replay: false,
    })
}

pub struct SodPolicyDeleteRequest {
    pub tenant_id: Uuid,
    pub policy_id: Uuid,
    pub actor_user_id: Option<Uuid>,
    pub idempotency_key: String,
    pub trace_id: String,
    pub causation_id: Option<Uuid>,
    pub producer: String,
}

pub struct SodPolicyDeleteResult {
    pub deleted: bool,
    pub idempotent_replay: bool,
}

pub async fn delete_policy(
    pool: &sqlx::PgPool,
    req: SodPolicyDeleteRequest,
) -> Result<SodPolicyDeleteResult, sqlx::Error> {
    let mut tx = pool.begin().await?;

    let mutation = sqlx::query(
        r#"
        INSERT INTO sod_policy_mutations (
            tenant_id,
            idempotency_key,
            policy_id,
            actor_user_id,
            mutation_type,
            mutation_payload
        )
        VALUES ($1, $2, $3, $4, 'policy_delete', $5)
        ON CONFLICT (tenant_id, idempotency_key) DO NOTHING
        RETURNING id
        "#,
    )
    .bind(req.tenant_id)
    .bind(&req.idempotency_key)
    .bind(req.policy_id)
    .bind(req.actor_user_id)
    .bind(json!({"policy_id": req.policy_id}))
    .fetch_optional(&mut *tx)
    .await?;

    let replay = mutation.is_none();

    if replay {
        tx.commit().await?;
        return Ok(SodPolicyDeleteResult {
            deleted: false,
            idempotent_replay: true,
        });
    }

    let res = sqlx::query(
        "DELETE FROM sod_policies WHERE id = $1 AND tenant_id = $2",
    )
    .bind(req.policy_id)
    .bind(req.tenant_id)
    .execute(&mut *tx)
    .await?;

    let deleted = res.rows_affected() > 0;

    if deleted {
        append_sod_outbox_event_tx(
            &mut tx,
            req.tenant_id,
            req.policy_id,
            "auth.sod.policy.deleted",
            "auth.sod.policy.deleted/v1",
            req.trace_id,
            req.causation_id,
            req.producer,
            json!({
                "policy_id": req.policy_id,
                "tenant_id": req.tenant_id,
                "idempotency_key": req.idempotency_key,
            }),
        )
        .await?;
    }

    tx.commit().await?;

    Ok(SodPolicyDeleteResult {
        deleted,
        idempotent_replay: false,
    })
}

pub async fn list_policies(
    pool: &sqlx::PgPool,
    tenant_id: Uuid,
    action_key: &str,
) -> Result<Vec<SodPolicy>, sqlx::Error> {
    let rows = sqlx::query(
        r#"
        SELECT id, tenant_id, action_key, primary_role_id, conflicting_role_id,
               allow_override, override_requires_approval, created_at, updated_at
        FROM sod_policies
        WHERE tenant_id = $1 AND action_key = $2
        ORDER BY created_at ASC, id ASC
        "#,
    )
    .bind(tenant_id)
    .bind(action_key)
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|r| SodPolicy {
            id: r.get("id"),
            tenant_id: r.get("tenant_id"),
            action_key: r.get("action_key"),
            primary_role_id: r.get("primary_role_id"),
            conflicting_role_id: r.get("conflicting_role_id"),
            allow_override: r.get("allow_override"),
            override_requires_approval: r.get("override_requires_approval"),
            created_at: r.get("created_at"),
            updated_at: r.get("updated_at"),
        })
        .collect())
}

#[allow(clippy::too_many_arguments)]
async fn append_sod_outbox_event_tx(
    tx: &mut Transaction<'_, Postgres>,
    tenant_id: Uuid,
    aggregate_id: Uuid,
    event_type: &str,
    schema_version: &str,
    trace_id: String,
    causation_id: Option<Uuid>,
    producer: String,
    data: Value,
) -> Result<(), sqlx::Error> {
    let envelope = EventEnvelope::new(
        tenant_id.to_string(),
        producer,
        event_type.to_string(),
        data,
    )
    .with_schema_version(schema_version.to_string())
    .with_trace_id(Some(trace_id))
    .with_causation_id(causation_id.map(|u| u.to_string()))
    .with_mutation_class(Some("user-data".to_string()));

    let payload =
        serde_json::to_value(&envelope).map_err(|e| sqlx::Error::Protocol(e.to_string()))?;

    sqlx::query(
        r#"
        INSERT INTO sod_events_outbox (event_id, tenant_id, aggregate_id, event_type, payload)
        VALUES ($1, $2, $3, $4, $5)
        "#,
    )
    .bind(envelope.event_id)
    .bind(tenant_id)
    .bind(aggregate_id)
    .bind(event_type)
    .bind(payload)
    .execute(&mut **tx)
    .await?;

    Ok(())
}
