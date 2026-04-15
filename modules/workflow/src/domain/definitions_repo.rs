//! Definition repository — all SQL for `workflow_definitions`.

use sqlx::PgPool;
use uuid::Uuid;

use crate::events::{envelope, subjects};
use crate::outbox;

use super::definitions::{
    CreateDefinitionRequest, DefError, ListDefinitionsQuery, WorkflowDefinition,
};

// ── Event payloads ────────────────────────────────────────────

#[derive(Debug, serde::Serialize)]
struct DefinitionCreatedPayload {
    definition_id: Uuid,
    tenant_id: String,
    name: String,
    version: i32,
    initial_step_id: String,
    step_count: usize,
}

// ── Repository ────────────────────────────────────────────────

pub struct DefinitionRepo;

impl DefinitionRepo {
    /// Create a new workflow definition.
    /// Guard: validate steps array, initial_step_id exists in steps.
    /// Mutation: INSERT definition.
    /// Outbox: enqueue definition.created event atomically.
    pub async fn create(
        pool: &PgPool,
        req: &CreateDefinitionRequest,
    ) -> Result<WorkflowDefinition, DefError> {
        // ── Guard ──
        let steps_arr = req
            .steps
            .as_array()
            .ok_or_else(|| DefError::Validation("steps must be a JSON array".into()))?;

        if steps_arr.is_empty() {
            return Err(DefError::Validation("steps cannot be empty".into()));
        }

        let step_ids: Vec<&str> = steps_arr
            .iter()
            .filter_map(|s| s.get("step_id").and_then(|v| v.as_str()))
            .collect();

        if !step_ids.contains(&req.initial_step_id.as_str()) {
            return Err(DefError::Validation(format!(
                "initial_step_id '{}' not found in steps",
                req.initial_step_id
            )));
        }

        // Check for duplicate step_ids
        let mut seen = std::collections::HashSet::new();
        for sid in &step_ids {
            if !seen.insert(sid) {
                return Err(DefError::Validation(format!("duplicate step_id: {}", sid)));
            }
        }

        // ── Mutation + Outbox (single tx) ──
        let id = Uuid::new_v4();
        let event_id = Uuid::new_v4();
        let mut tx = pool.begin().await?;

        let def = sqlx::query_as::<_, WorkflowDefinition>(
            r#"
            INSERT INTO workflow_definitions
                (id, tenant_id, name, description, steps, initial_step_id)
            VALUES ($1, $2, $3, $4, $5, $6)
            RETURNING *
            "#,
        )
        .bind(id)
        .bind(&req.tenant_id)
        .bind(&req.name)
        .bind(&req.description)
        .bind(&req.steps)
        .bind(&req.initial_step_id)
        .fetch_one(&mut *tx)
        .await
        .map_err(|e| {
            if let sqlx::Error::Database(ref db_err) = e {
                if db_err
                    .message()
                    .contains("duplicate key value violates unique constraint")
                {
                    return DefError::Duplicate;
                }
            }
            DefError::Database(e)
        })?;

        let event_payload = DefinitionCreatedPayload {
            definition_id: def.id,
            tenant_id: def.tenant_id.clone(),
            name: def.name.clone(),
            version: def.version,
            initial_step_id: def.initial_step_id.clone(),
            step_count: steps_arr.len(),
        };

        let env = envelope::create_envelope(
            event_id,
            def.tenant_id.clone(),
            subjects::DEFINITION_CREATED.to_string(),
            event_payload,
        );
        let validated = envelope::validate_envelope(&env)
            .map_err(|e| DefError::Validation(format!("envelope validation: {}", e)))?;

        outbox::enqueue_event_tx(
            &mut tx,
            event_id,
            subjects::DEFINITION_CREATED,
            "workflow_definition",
            &def.id.to_string(),
            &validated,
        )
        .await?;

        tx.commit().await?;

        Ok(def)
    }

    pub async fn get(
        pool: &PgPool,
        tenant_id: &str,
        id: Uuid,
    ) -> Result<WorkflowDefinition, DefError> {
        sqlx::query_as::<_, WorkflowDefinition>(
            "SELECT * FROM workflow_definitions WHERE id = $1 AND tenant_id = $2",
        )
        .bind(id)
        .bind(tenant_id)
        .fetch_optional(pool)
        .await?
        .ok_or(DefError::NotFound)
    }

    pub async fn count(pool: &PgPool, q: &ListDefinitionsQuery) -> Result<i64, DefError> {
        let active_only = q.active_only.unwrap_or(false);
        let row: (i64,) = if active_only {
            sqlx::query_as(
                "SELECT COUNT(*) FROM workflow_definitions WHERE tenant_id = $1 AND is_active = true",
            )
            .bind(&q.tenant_id)
            .fetch_one(pool)
            .await?
        } else {
            sqlx::query_as("SELECT COUNT(*) FROM workflow_definitions WHERE tenant_id = $1")
                .bind(&q.tenant_id)
                .fetch_one(pool)
                .await?
        };
        Ok(row.0)
    }

    pub async fn list(
        pool: &PgPool,
        q: &ListDefinitionsQuery,
    ) -> Result<Vec<WorkflowDefinition>, DefError> {
        let limit = q.limit.unwrap_or(50).min(200);
        let offset = q.offset.unwrap_or(0);
        let active_only = q.active_only.unwrap_or(false);

        if active_only {
            Ok(sqlx::query_as::<_, WorkflowDefinition>(
                r#"
                SELECT * FROM workflow_definitions
                WHERE tenant_id = $1 AND is_active = true
                ORDER BY name, version DESC
                LIMIT $2 OFFSET $3
                "#,
            )
            .bind(&q.tenant_id)
            .bind(limit)
            .bind(offset)
            .fetch_all(pool)
            .await?)
        } else {
            Ok(sqlx::query_as::<_, WorkflowDefinition>(
                r#"
                SELECT * FROM workflow_definitions
                WHERE tenant_id = $1
                ORDER BY name, version DESC
                LIMIT $2 OFFSET $3
                "#,
            )
            .bind(&q.tenant_id)
            .bind(limit)
            .bind(offset)
            .fetch_all(pool)
            .await?)
        }
    }
}
