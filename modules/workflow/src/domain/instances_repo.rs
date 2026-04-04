//! Instance repository — all SQL for `workflow_instances`, `workflow_transitions`,
//! and `workflow_idempotency_keys`.

use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use uuid::Uuid;

use super::definitions::WorkflowDefinition;
use super::instances::{
    AdvanceInstanceRequest, InstanceError, ListInstancesQuery, StartInstanceRequest,
    WorkflowInstance, WorkflowTransition,
};
use super::types::InstanceStatus;
use crate::events::{envelope, subjects};
use crate::outbox;

// ── Event payloads ────────────────────────────────────────────

#[derive(Debug, Serialize)]
struct InstanceStartedPayload {
    instance_id: uuid::Uuid,
    tenant_id: String,
    definition_id: uuid::Uuid,
    entity_type: String,
    entity_id: String,
    initial_step_id: String,
}

#[derive(Debug, Serialize)]
struct InstanceAdvancedPayload {
    instance_id: uuid::Uuid,
    tenant_id: String,
    transition_id: uuid::Uuid,
    from_step_id: String,
    to_step_id: String,
    action: String,
}

#[derive(Debug, Serialize)]
struct InstanceCompletedPayload {
    instance_id: uuid::Uuid,
    tenant_id: String,
    final_step_id: String,
}

#[derive(Debug, Serialize)]
struct InstanceCancelledPayload {
    instance_id: uuid::Uuid,
    tenant_id: String,
    step_at_cancellation: String,
}

// ── Idempotency ──────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize, sqlx::FromRow)]
struct IdempotencyHit {
    response_body: serde_json::Value,
    status_code: i32,
}

async fn check_idempotency(
    pool: &PgPool,
    key: &str,
) -> Result<Option<IdempotencyHit>, sqlx::Error> {
    sqlx::query_as::<_, IdempotencyHit>(
        r#"
        SELECT response_body, status_code
        FROM workflow_idempotency_keys
        WHERE app_id = 'workflow' AND idempotency_key = $1
          AND expires_at > now()
        "#,
    )
    .bind(key)
    .fetch_optional(pool)
    .await
}

async fn store_idempotency(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    key: &str,
    response: &serde_json::Value,
    status_code: i32,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        INSERT INTO workflow_idempotency_keys
            (idempotency_key, response_body, status_code, expires_at)
        VALUES ($1, $2, $3, now() + interval '24 hours')
        ON CONFLICT (app_id, idempotency_key) DO NOTHING
        "#,
    )
    .bind(key)
    .bind(response)
    .bind(status_code)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

// ── Repository ────────────────────────────────────────────────

pub struct InstanceRepo;

impl InstanceRepo {
    /// Start a new workflow instance.
    /// Guard: definition must exist and be active; initial_step_id must match.
    /// Mutation: INSERT instance + initial transition record.
    /// Outbox: enqueue instance.started event atomically.
    pub async fn start(
        pool: &PgPool,
        req: &StartInstanceRequest,
    ) -> Result<WorkflowInstance, InstanceError> {
        // ── Idempotency check ──
        if let Some(ref key) = req.idempotency_key {
            if let Some(hit) = check_idempotency(pool, key).await? {
                let instance: WorkflowInstance = serde_json::from_value(hit.response_body)
                    .map_err(|e| {
                        InstanceError::Validation(format!("idempotency replay decode: {}", e))
                    })?;
                return Ok(instance);
            }
        }

        // ── Guard: definition exists and is active ──
        let def = sqlx::query_as::<_, WorkflowDefinition>(
            "SELECT * FROM workflow_definitions WHERE id = $1 AND tenant_id = $2",
        )
        .bind(req.definition_id)
        .bind(&req.tenant_id)
        .fetch_optional(pool)
        .await?
        .ok_or(InstanceError::DefinitionNotFound)?;

        if !def.is_active {
            return Err(InstanceError::Validation(
                "Cannot start instance from inactive definition".into(),
            ));
        }

        // ── Mutation + Outbox (single tx) ──
        let instance_id = Uuid::new_v4();
        let event_id = Uuid::new_v4();
        let transition_id = Uuid::new_v4();
        let context = req.context.clone().unwrap_or(serde_json::json!({}));
        let mut tx = pool.begin().await?;

        let instance = sqlx::query_as::<_, WorkflowInstance>(
            r#"
            INSERT INTO workflow_instances
                (id, tenant_id, definition_id, entity_type, entity_id,
                 current_step_id, status, context)
            VALUES ($1, $2, $3, $4, $5, $6, 'active', $7)
            RETURNING *
            "#,
        )
        .bind(instance_id)
        .bind(&req.tenant_id)
        .bind(req.definition_id)
        .bind(&req.entity_type)
        .bind(&req.entity_id)
        .bind(&def.initial_step_id)
        .bind(&context)
        .fetch_one(&mut *tx)
        .await?;

        // Record initial transition
        sqlx::query(
            r#"
            INSERT INTO workflow_transitions
                (id, tenant_id, instance_id, from_step_id, to_step_id, action, idempotency_key)
            VALUES ($1, $2, $3, '__start__', $4, 'start', $5)
            "#,
        )
        .bind(transition_id)
        .bind(&req.tenant_id)
        .bind(instance_id)
        .bind(&def.initial_step_id)
        .bind(&req.idempotency_key)
        .execute(&mut *tx)
        .await?;

        // Outbox event
        let event_payload = InstanceStartedPayload {
            instance_id: instance.id,
            tenant_id: instance.tenant_id.clone(),
            definition_id: instance.definition_id,
            entity_type: instance.entity_type.clone(),
            entity_id: instance.entity_id.clone(),
            initial_step_id: instance.current_step_id.clone(),
        };

        let env = envelope::create_envelope(
            event_id,
            instance.tenant_id.clone(),
            subjects::INSTANCE_STARTED.to_string(),
            event_payload,
        );
        let validated = envelope::validate_envelope(&env)
            .map_err(|e| InstanceError::Validation(format!("envelope validation: {}", e)))?;

        outbox::enqueue_event_tx(
            &mut tx,
            event_id,
            subjects::INSTANCE_STARTED,
            "workflow_instance",
            &instance.id.to_string(),
            &validated,
        )
        .await?;

        // Store idempotency key
        if let Some(ref key) = req.idempotency_key {
            store_idempotency(
                &mut tx,
                key,
                &serde_json::to_value(&instance).unwrap_or_default(),
                201,
            )
            .await?;
        }

        tx.commit().await?;

        Ok(instance)
    }

    /// Advance a workflow instance to a new step.
    /// Guard: instance must be active; to_step_id must exist in definition.
    /// Mutation: UPDATE instance + INSERT transition.
    /// Outbox: enqueue instance.advanced (or instance.completed) event.
    pub async fn advance(
        pool: &PgPool,
        instance_id: Uuid,
        req: &AdvanceInstanceRequest,
    ) -> Result<(WorkflowInstance, WorkflowTransition), InstanceError> {
        // ── Idempotency check ──
        if let Some(ref key) = req.idempotency_key {
            if let Some(hit) = check_idempotency(pool, key).await? {
                let result: (WorkflowInstance, WorkflowTransition) =
                    serde_json::from_value(hit.response_body).map_err(|e| {
                        InstanceError::Validation(format!("idempotency replay decode: {}", e))
                    })?;
                return Ok(result);
            }
        }

        let mut tx = pool.begin().await?;

        // ── Guard: lock instance row, check active ──
        let instance = sqlx::query_as::<_, WorkflowInstance>(
            "SELECT * FROM workflow_instances WHERE id = $1 AND tenant_id = $2 FOR UPDATE",
        )
        .bind(instance_id)
        .bind(&req.tenant_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(InstanceError::NotFound)?;

        if instance.status != InstanceStatus::Active {
            return Err(InstanceError::NotActive(instance.status.to_string()));
        }

        // ── Guard: validate to_step_id exists in definition ──
        let def = sqlx::query_as::<_, WorkflowDefinition>(
            "SELECT * FROM workflow_definitions WHERE id = $1 AND tenant_id = $2",
        )
        .bind(instance.definition_id)
        .bind(&req.tenant_id)
        .fetch_one(&mut *tx)
        .await?;

        // Allow special "__completed__" and "__cancelled__" terminal pseudo-steps.
        // Check terminals first (cheap) before scanning the definition array.
        let valid_target = req.to_step_id == "__completed__"
            || req.to_step_id == "__cancelled__"
            || def.steps.as_array().is_some_and(|arr| {
                arr.iter()
                    .any(|s| s.get("step_id").and_then(|v| v.as_str()) == Some(&req.to_step_id))
            });

        if !valid_target {
            return Err(InstanceError::InvalidTransition(format!(
                "step '{}' not found in definition",
                req.to_step_id
            )));
        }

        let from_step = instance.current_step_id.clone();

        // ── Determine terminal state ──
        let (new_status, event_type) = if req.to_step_id == "__completed__" {
            ("completed", subjects::INSTANCE_COMPLETED)
        } else if req.to_step_id == "__cancelled__" {
            ("cancelled", subjects::INSTANCE_CANCELLED)
        } else {
            ("active", subjects::INSTANCE_ADVANCED)
        };

        // ── Mutation ──
        let updated_instance = sqlx::query_as::<_, WorkflowInstance>(
            r#"
            UPDATE workflow_instances
            SET current_step_id = $1,
                status = $2,
                completed_at = CASE WHEN $2 = 'completed' THEN now() ELSE completed_at END,
                cancelled_at = CASE WHEN $2 = 'cancelled' THEN now() ELSE cancelled_at END,
                updated_at = now()
            WHERE id = $3 AND tenant_id = $4
            RETURNING *
            "#,
        )
        .bind(&req.to_step_id)
        .bind(new_status)
        .bind(instance_id)
        .bind(&req.tenant_id)
        .fetch_one(&mut *tx)
        .await?;

        let transition_id = Uuid::new_v4();
        let transition = sqlx::query_as::<_, WorkflowTransition>(
            r#"
            INSERT INTO workflow_transitions
                (id, tenant_id, instance_id, from_step_id, to_step_id,
                 action, actor_id, actor_type, comment, metadata, idempotency_key)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)
            RETURNING *
            "#,
        )
        .bind(transition_id)
        .bind(&req.tenant_id)
        .bind(instance_id)
        .bind(&from_step)
        .bind(&req.to_step_id)
        .bind(&req.action)
        .bind(req.actor_id)
        .bind(&req.actor_type)
        .bind(&req.comment)
        .bind(&req.metadata)
        .bind(&req.idempotency_key)
        .fetch_one(&mut *tx)
        .await?;

        // ── Outbox event ──
        let event_id = Uuid::new_v4();

        let event_value: serde_json::Value = match new_status {
            "completed" => {
                let payload = InstanceCompletedPayload {
                    instance_id: updated_instance.id,
                    tenant_id: updated_instance.tenant_id.clone(),
                    final_step_id: from_step.clone(),
                };
                let env = envelope::create_envelope(
                    event_id,
                    updated_instance.tenant_id.clone(),
                    event_type.to_string(),
                    payload,
                );
                envelope::validate_envelope(&env)
                    .map_err(|e| InstanceError::Validation(format!("envelope validation: {}", e)))?
            }
            "cancelled" => {
                let payload = InstanceCancelledPayload {
                    instance_id: updated_instance.id,
                    tenant_id: updated_instance.tenant_id.clone(),
                    step_at_cancellation: from_step.clone(),
                };
                let env = envelope::create_envelope(
                    event_id,
                    updated_instance.tenant_id.clone(),
                    event_type.to_string(),
                    payload,
                );
                envelope::validate_envelope(&env)
                    .map_err(|e| InstanceError::Validation(format!("envelope validation: {}", e)))?
            }
            _ => {
                let payload = InstanceAdvancedPayload {
                    instance_id: updated_instance.id,
                    tenant_id: updated_instance.tenant_id.clone(),
                    transition_id: transition.id,
                    from_step_id: from_step.clone(),
                    to_step_id: req.to_step_id.clone(),
                    action: req.action.clone(),
                };
                let env = envelope::create_envelope(
                    event_id,
                    updated_instance.tenant_id.clone(),
                    event_type.to_string(),
                    payload,
                );
                envelope::validate_envelope(&env)
                    .map_err(|e| InstanceError::Validation(format!("envelope validation: {}", e)))?
            }
        };

        outbox::enqueue_event_tx(
            &mut tx,
            event_id,
            event_type,
            "workflow_instance",
            &updated_instance.id.to_string(),
            &event_value,
        )
        .await?;

        // Store idempotency key
        if let Some(ref key) = req.idempotency_key {
            let response =
                serde_json::to_value((&updated_instance, &transition)).unwrap_or_default();
            store_idempotency(&mut tx, key, &response, 200).await?;
        }

        tx.commit().await?;

        Ok((updated_instance, transition))
    }

    pub async fn get(
        pool: &PgPool,
        tenant_id: &str,
        id: Uuid,
    ) -> Result<WorkflowInstance, InstanceError> {
        sqlx::query_as::<_, WorkflowInstance>(
            "SELECT * FROM workflow_instances WHERE id = $1 AND tenant_id = $2",
        )
        .bind(id)
        .bind(tenant_id)
        .fetch_optional(pool)
        .await?
        .ok_or(InstanceError::NotFound)
    }

    pub async fn count(
        pool: &PgPool,
        q: &ListInstancesQuery,
    ) -> Result<i64, InstanceError> {
        let mut conditions = vec!["tenant_id = $1".to_string()];
        let mut param_idx = 2u32;

        if q.entity_type.is_some() {
            conditions.push(format!("entity_type = ${}", param_idx));
            param_idx += 1;
        }
        if q.entity_id.is_some() {
            conditions.push(format!("entity_id = ${}", param_idx));
            param_idx += 1;
        }
        if q.status.is_some() {
            conditions.push(format!("status = ${}", param_idx));
            param_idx += 1;
        }
        if q.definition_id.is_some() {
            conditions.push(format!("definition_id = ${}", param_idx));
        }

        let where_clause = conditions.join(" AND ");
        let query_str = format!(
            "SELECT COUNT(*) FROM workflow_instances WHERE {}",
            where_clause
        );

        let mut query = sqlx::query_as::<_, (i64,)>(&query_str).bind(&q.tenant_id);

        if let Some(ref et) = q.entity_type {
            query = query.bind(et);
        }
        if let Some(ref ei) = q.entity_id {
            query = query.bind(ei);
        }
        if let Some(ref st) = q.status {
            query = query.bind(st);
        }
        if let Some(ref di) = q.definition_id {
            query = query.bind(di);
        }

        let row = query.fetch_one(pool).await?;
        Ok(row.0)
    }

    pub async fn list(
        pool: &PgPool,
        q: &ListInstancesQuery,
    ) -> Result<Vec<WorkflowInstance>, InstanceError> {
        let limit = q.limit.unwrap_or(50).min(200);
        let offset = q.offset.unwrap_or(0);

        // Build dynamic WHERE clause
        let mut conditions = vec!["tenant_id = $1".to_string()];
        let mut param_idx = 2u32;

        if q.entity_type.is_some() {
            conditions.push(format!("entity_type = ${}", param_idx));
            param_idx += 1;
        }
        if q.entity_id.is_some() {
            conditions.push(format!("entity_id = ${}", param_idx));
            param_idx += 1;
        }
        if q.status.is_some() {
            conditions.push(format!("status = ${}", param_idx));
            param_idx += 1;
        }
        if q.definition_id.is_some() {
            conditions.push(format!("definition_id = ${}", param_idx));
            param_idx += 1;
        }

        let where_clause = conditions.join(" AND ");
        let query_str = format!(
            "SELECT * FROM workflow_instances WHERE {} ORDER BY created_at DESC LIMIT ${} OFFSET ${}",
            where_clause, param_idx, param_idx + 1
        );

        let mut query = sqlx::query_as::<_, WorkflowInstance>(&query_str).bind(&q.tenant_id);

        if let Some(ref et) = q.entity_type {
            query = query.bind(et);
        }
        if let Some(ref ei) = q.entity_id {
            query = query.bind(ei);
        }
        if let Some(ref st) = q.status {
            query = query.bind(st);
        }
        if let Some(ref di) = q.definition_id {
            query = query.bind(di);
        }

        query = query.bind(limit).bind(offset);

        Ok(query.fetch_all(pool).await?)
    }

    pub async fn list_transitions(
        pool: &PgPool,
        tenant_id: &str,
        instance_id: Uuid,
    ) -> Result<Vec<WorkflowTransition>, InstanceError> {
        Ok(sqlx::query_as::<_, WorkflowTransition>(
            r#"
            SELECT * FROM workflow_transitions
            WHERE instance_id = $1 AND tenant_id = $2
            ORDER BY transitioned_at ASC
            "#,
        )
        .bind(instance_id)
        .bind(tenant_id)
        .fetch_all(pool)
        .await?)
    }
}
