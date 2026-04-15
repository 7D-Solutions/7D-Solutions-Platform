use chrono::{DateTime, Duration, NaiveDate, Utc};
use sqlx::PgPool;
use uuid::Uuid;

/// Flat row returned by the enriched step queries.
/// Holds all routing_steps columns plus the two aliased workcenter columns.
#[derive(sqlx::FromRow)]
struct EnrichedStepRow {
    routing_step_id: Uuid,
    routing_template_id: Uuid,
    sequence_number: i32,
    workcenter_id: Uuid,
    operation_name: String,
    description: Option<String>,
    setup_time_minutes: Option<i32>,
    run_time_minutes: Option<i32>,
    is_required: bool,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
    wc_name: Option<String>,
    wc_code: Option<String>,
}

use crate::domain::idempotency::{check_idempotency, store_idempotency_key, IdempotencyError};
use crate::domain::outbox::enqueue_event;
use crate::events::{self, ProductionEventType};

use super::types::{
    AddRoutingStepRequest, CreateRoutingRequest, RoutingError, RoutingStatus, RoutingStep,
    RoutingStepEnriched, RoutingTemplate, UpdateRoutingRequest, WorkcenterDetails,
};

pub struct RoutingRepo;

impl RoutingRepo {
    pub async fn create(
        pool: &PgPool,
        req: &CreateRoutingRequest,
        correlation_id: &str,
        causation_id: Option<&str>,
    ) -> Result<RoutingTemplate, RoutingError> {
        if req.tenant_id.trim().is_empty() {
            return Err(RoutingError::Validation(
                "tenant_id is required".to_string(),
            ));
        }
        if req.name.trim().is_empty() {
            return Err(RoutingError::Validation("name is required".to_string()));
        }

        let request_hash = serde_json::to_string(req)
            .map_err(|e| RoutingError::Database(sqlx::Error::Protocol(e.to_string())))?;

        let revision = req.revision.as_deref().unwrap_or("1");

        let mut tx = pool.begin().await?;

        // Idempotency check
        if let Some(key) = &req.idempotency_key {
            match check_idempotency(&mut tx, &req.tenant_id, key, &request_hash).await {
                Ok(Some(cached)) => {
                    let rt: RoutingTemplate = serde_json::from_str(&cached).map_err(|e| {
                        RoutingError::Database(sqlx::Error::Protocol(e.to_string()))
                    })?;
                    tx.commit().await?;
                    return Ok(rt);
                }
                Ok(None) => {}
                Err(IdempotencyError::Conflict) => {
                    return Err(RoutingError::ConflictingIdempotencyKey);
                }
                Err(IdempotencyError::Database(e)) => return Err(RoutingError::Database(e)),
                Err(IdempotencyError::Json(e)) => {
                    return Err(RoutingError::Database(sqlx::Error::Protocol(e.to_string())));
                }
            }
        }

        let rt = sqlx::query_as::<_, RoutingTemplate>(
            r#"
            INSERT INTO routing_templates
                (tenant_id, name, description, item_id, bom_revision_id,
                 revision, status, effective_from_date)
            VALUES ($1, $2, $3, $4, $5, $6, 'draft', $7)
            RETURNING *
            "#,
        )
        .bind(&req.tenant_id)
        .bind(&req.name)
        .bind(&req.description)
        .bind(req.item_id)
        .bind(req.bom_revision_id)
        .bind(revision)
        .bind(req.effective_from_date)
        .fetch_one(&mut *tx)
        .await
        .map_err(|e| {
            if let sqlx::Error::Database(ref dbe) = e {
                if dbe.code().as_deref() == Some("23505") {
                    return RoutingError::DuplicateRevision(
                        revision.to_string(),
                        req.tenant_id.clone(),
                    );
                }
            }
            RoutingError::Database(e)
        })?;

        enqueue_event(
            &mut tx,
            &req.tenant_id,
            ProductionEventType::RoutingCreated,
            "routing_template",
            &rt.routing_template_id.to_string(),
            &events::build_routing_created_envelope(
                rt.routing_template_id,
                req.tenant_id.clone(),
                req.name.clone(),
                revision.to_string(),
                correlation_id.to_string(),
                causation_id.map(String::from),
            ),
            correlation_id,
            causation_id,
        )
        .await?;

        // Store idempotency key
        if let Some(key) = &req.idempotency_key {
            let resp = serde_json::to_string(&rt)
                .map_err(|e| RoutingError::Database(sqlx::Error::Protocol(e.to_string())))?;
            store_idempotency_key(
                &mut tx,
                &req.tenant_id,
                key,
                &request_hash,
                &resp,
                201,
                Utc::now() + Duration::hours(24),
            )
            .await?;
        }

        tx.commit().await?;
        Ok(rt)
    }

    pub async fn find_by_id(
        pool: &PgPool,
        id: Uuid,
        tenant_id: &str,
    ) -> Result<Option<RoutingTemplate>, RoutingError> {
        sqlx::query_as::<_, RoutingTemplate>(
            "SELECT * FROM routing_templates WHERE routing_template_id = $1 AND tenant_id = $2",
        )
        .bind(id)
        .bind(tenant_id)
        .fetch_optional(pool)
        .await
        .map_err(RoutingError::Database)
    }

    pub async fn find_by_item_and_date(
        pool: &PgPool,
        tenant_id: &str,
        item_id: Uuid,
        effective_date: NaiveDate,
    ) -> Result<Vec<RoutingTemplate>, RoutingError> {
        sqlx::query_as::<_, RoutingTemplate>(
            r#"
            SELECT * FROM routing_templates
            WHERE tenant_id = $1
              AND item_id = $2
              AND (effective_from_date IS NULL OR effective_from_date <= $3)
              AND is_active = TRUE
            ORDER BY effective_from_date DESC NULLS LAST
            "#,
        )
        .bind(tenant_id)
        .bind(item_id)
        .bind(effective_date)
        .fetch_all(pool)
        .await
        .map_err(RoutingError::Database)
    }

    pub async fn list(
        pool: &PgPool,
        tenant_id: &str,
        page: i64,
        page_size: i64,
    ) -> Result<(Vec<RoutingTemplate>, i64), RoutingError> {
        let limit = page_size.clamp(1, 200);
        let offset = (page.max(1) - 1) * limit;

        let items = sqlx::query_as::<_, RoutingTemplate>(
            "SELECT * FROM routing_templates WHERE tenant_id = $1 ORDER BY name LIMIT $2 OFFSET $3",
        )
        .bind(tenant_id)
        .bind(limit)
        .bind(offset)
        .fetch_all(pool)
        .await
        .map_err(RoutingError::Database)?;

        let (total,): (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM routing_templates WHERE tenant_id = $1")
                .bind(tenant_id)
                .fetch_one(pool)
                .await
                .map_err(RoutingError::Database)?;

        Ok((items, total))
    }

    pub async fn update(
        pool: &PgPool,
        id: Uuid,
        req: &UpdateRoutingRequest,
        correlation_id: &str,
        causation_id: Option<&str>,
    ) -> Result<RoutingTemplate, RoutingError> {
        if req.tenant_id.trim().is_empty() {
            return Err(RoutingError::Validation(
                "tenant_id is required".to_string(),
            ));
        }

        let mut tx = pool.begin().await?;

        // Check current status — released routings are immutable
        let current = sqlx::query_as::<_, RoutingTemplate>(
            "SELECT * FROM routing_templates WHERE routing_template_id = $1 AND tenant_id = $2 FOR UPDATE",
        )
        .bind(id)
        .bind(&req.tenant_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(RoutingError::NotFound)?;

        if current.status == "released" {
            return Err(RoutingError::ReleasedImmutable);
        }

        let rt = sqlx::query_as::<_, RoutingTemplate>(
            r#"
            UPDATE routing_templates
            SET name               = COALESCE($3, name),
                description        = COALESCE($4, description),
                effective_from_date = COALESCE($5, effective_from_date),
                updated_at         = NOW()
            WHERE routing_template_id = $1 AND tenant_id = $2
            RETURNING *
            "#,
        )
        .bind(id)
        .bind(&req.tenant_id)
        .bind(&req.name)
        .bind(&req.description)
        .bind(req.effective_from_date)
        .fetch_one(&mut *tx)
        .await?;

        enqueue_event(
            &mut tx,
            &req.tenant_id,
            ProductionEventType::RoutingUpdated,
            "routing_template",
            &rt.routing_template_id.to_string(),
            &events::build_routing_updated_envelope(
                rt.routing_template_id,
                req.tenant_id.clone(),
                rt.name.clone(),
                rt.revision.clone(),
                correlation_id.to_string(),
                causation_id.map(String::from),
            ),
            correlation_id,
            causation_id,
        )
        .await?;

        tx.commit().await?;
        Ok(rt)
    }

    pub async fn release(
        pool: &PgPool,
        id: Uuid,
        tenant_id: &str,
        correlation_id: &str,
        causation_id: Option<&str>,
    ) -> Result<RoutingTemplate, RoutingError> {
        let mut tx = pool.begin().await?;

        let current = sqlx::query_as::<_, RoutingTemplate>(
            "SELECT * FROM routing_templates WHERE routing_template_id = $1 AND tenant_id = $2 FOR UPDATE",
        )
        .bind(id)
        .bind(tenant_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(RoutingError::NotFound)?;

        let status = RoutingStatus::from_str(&current.status).ok_or_else(|| {
            RoutingError::Validation(format!("Unknown status: {}", current.status))
        })?;

        if status != RoutingStatus::Draft {
            return Err(RoutingError::InvalidTransition {
                from: current.status.clone(),
                to: "released".to_string(),
            });
        }

        let rt = sqlx::query_as::<_, RoutingTemplate>(
            r#"
            UPDATE routing_templates
            SET status = 'released', updated_at = NOW()
            WHERE routing_template_id = $1 AND tenant_id = $2
            RETURNING *
            "#,
        )
        .bind(id)
        .bind(tenant_id)
        .fetch_one(&mut *tx)
        .await?;

        enqueue_event(
            &mut tx,
            tenant_id,
            ProductionEventType::RoutingReleased,
            "routing_template",
            &rt.routing_template_id.to_string(),
            &events::build_routing_released_envelope(
                rt.routing_template_id,
                tenant_id.to_string(),
                rt.name.clone(),
                rt.revision.clone(),
                correlation_id.to_string(),
                causation_id.map(String::from),
            ),
            correlation_id,
            causation_id,
        )
        .await?;

        tx.commit().await?;
        Ok(rt)
    }

    // ========================================================================
    // Routing steps (operations)
    // ========================================================================

    pub async fn add_step(
        pool: &PgPool,
        routing_template_id: Uuid,
        req: &AddRoutingStepRequest,
        correlation_id: &str,
        causation_id: Option<&str>,
    ) -> Result<RoutingStep, RoutingError> {
        if req.tenant_id.trim().is_empty() {
            return Err(RoutingError::Validation(
                "tenant_id is required".to_string(),
            ));
        }
        if req.operation_name.trim().is_empty() {
            return Err(RoutingError::Validation(
                "operation_name is required".to_string(),
            ));
        }
        if req.sequence_number <= 0 {
            return Err(RoutingError::Validation(
                "sequence_number must be > 0".to_string(),
            ));
        }

        let request_hash = serde_json::to_string(req)
            .map_err(|e| RoutingError::Database(sqlx::Error::Protocol(e.to_string())))?;

        let mut tx = pool.begin().await?;

        // Idempotency check
        if let Some(key) = &req.idempotency_key {
            match check_idempotency(&mut tx, &req.tenant_id, key, &request_hash).await {
                Ok(Some(cached)) => {
                    let step: RoutingStep = serde_json::from_str(&cached).map_err(|e| {
                        RoutingError::Database(sqlx::Error::Protocol(e.to_string()))
                    })?;
                    tx.commit().await?;
                    return Ok(step);
                }
                Ok(None) => {}
                Err(IdempotencyError::Conflict) => {
                    return Err(RoutingError::ConflictingIdempotencyKey);
                }
                Err(IdempotencyError::Database(e)) => return Err(RoutingError::Database(e)),
                Err(IdempotencyError::Json(e)) => {
                    return Err(RoutingError::Database(sqlx::Error::Protocol(e.to_string())));
                }
            }
        }

        // Verify routing exists and belongs to tenant, and is still draft
        let routing = sqlx::query_as::<_, RoutingTemplate>(
            "SELECT * FROM routing_templates WHERE routing_template_id = $1 AND tenant_id = $2 FOR UPDATE",
        )
        .bind(routing_template_id)
        .bind(&req.tenant_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(RoutingError::NotFound)?;

        if routing.status == "released" {
            return Err(RoutingError::ReleasedImmutable);
        }

        // Verify workcenter exists and is active
        let wc_active: Option<(bool,)> = sqlx::query_as(
            "SELECT is_active FROM workcenters WHERE workcenter_id = $1 AND tenant_id = $2",
        )
        .bind(req.workcenter_id)
        .bind(&req.tenant_id)
        .fetch_optional(&mut *tx)
        .await?;

        match wc_active {
            Some((true,)) => {}
            _ => return Err(RoutingError::WorkcenterInvalid(req.workcenter_id)),
        }

        let is_required = req.is_required.unwrap_or(true);

        let step = sqlx::query_as::<_, RoutingStep>(
            r#"
            WITH inserted AS (
                INSERT INTO routing_steps
                    (routing_template_id, sequence_number, workcenter_id, operation_name,
                     description, setup_time_minutes, run_time_minutes, is_required)
                VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
                RETURNING *
            )
            SELECT i.*, w.name as workcenter_name
            FROM inserted i
            LEFT JOIN workcenters w ON w.workcenter_id = i.workcenter_id
            "#,
        )
        .bind(routing_template_id)
        .bind(req.sequence_number)
        .bind(req.workcenter_id)
        .bind(&req.operation_name)
        .bind(&req.description)
        .bind(req.setup_time_minutes)
        .bind(req.run_time_minutes)
        .bind(is_required)
        .fetch_one(&mut *tx)
        .await
        .map_err(|e| {
            if let sqlx::Error::Database(ref dbe) = e {
                if dbe.code().as_deref() == Some("23505") {
                    return RoutingError::DuplicateSequence(req.sequence_number);
                }
            }
            RoutingError::Database(e)
        })?;

        enqueue_event(
            &mut tx,
            &req.tenant_id,
            ProductionEventType::RoutingUpdated,
            "routing_template",
            &routing_template_id.to_string(),
            &events::build_routing_updated_envelope(
                routing_template_id,
                req.tenant_id.clone(),
                routing.name.clone(),
                routing.revision.clone(),
                correlation_id.to_string(),
                causation_id.map(String::from),
            ),
            correlation_id,
            causation_id,
        )
        .await?;

        // Store idempotency key
        if let Some(key) = &req.idempotency_key {
            let resp = serde_json::to_string(&step)
                .map_err(|e| RoutingError::Database(sqlx::Error::Protocol(e.to_string())))?;
            store_idempotency_key(
                &mut tx,
                &req.tenant_id,
                key,
                &request_hash,
                &resp,
                201,
                Utc::now() + Duration::hours(24),
            )
            .await?;
        }

        tx.commit().await?;
        Ok(step)
    }

    pub async fn list_steps(
        pool: &PgPool,
        routing_template_id: Uuid,
        tenant_id: &str,
    ) -> Result<Vec<RoutingStep>, RoutingError> {
        // Verify routing belongs to tenant
        let exists: Option<(Uuid,)> = sqlx::query_as(
            "SELECT routing_template_id FROM routing_templates WHERE routing_template_id = $1 AND tenant_id = $2",
        )
        .bind(routing_template_id)
        .bind(tenant_id)
        .fetch_optional(pool)
        .await?;

        if exists.is_none() {
            return Err(RoutingError::NotFound);
        }

        sqlx::query_as::<_, RoutingStep>(
            r#"
            SELECT rs.*, w.name as workcenter_name
            FROM routing_steps rs
            LEFT JOIN workcenters w ON w.workcenter_id = rs.workcenter_id
            WHERE rs.routing_template_id = $1
              AND rs.routing_template_id IN (SELECT routing_template_id FROM routing_templates WHERE tenant_id = $2)
            ORDER BY rs.sequence_number
            "#,
        )
        .bind(routing_template_id)
        .bind(tenant_id)
        .fetch_all(pool)
        .await
        .map_err(RoutingError::Database)
    }

    pub async fn find_step(
        pool: &PgPool,
        routing_template_id: Uuid,
        step_id: Uuid,
        tenant_id: &str,
    ) -> Result<Option<RoutingStep>, RoutingError> {
        let exists: Option<(Uuid,)> = sqlx::query_as(
            "SELECT routing_template_id FROM routing_templates WHERE routing_template_id = $1 AND tenant_id = $2",
        )
        .bind(routing_template_id)
        .bind(tenant_id)
        .fetch_optional(pool)
        .await?;

        if exists.is_none() {
            return Err(RoutingError::NotFound);
        }

        sqlx::query_as::<_, RoutingStep>(
            r#"
            SELECT rs.*, w.name as workcenter_name
            FROM routing_steps rs
            LEFT JOIN workcenters w ON w.workcenter_id = rs.workcenter_id
            WHERE rs.routing_step_id = $1
              AND rs.routing_template_id = $2
            "#,
        )
        .bind(step_id)
        .bind(routing_template_id)
        .fetch_optional(pool)
        .await
        .map_err(RoutingError::Database)
    }

    /// List routing steps with full workcenter details embedded.
    /// Used when the caller passes `?include=workcenter_details`.
    pub async fn list_steps_enriched(
        pool: &PgPool,
        routing_template_id: Uuid,
        tenant_id: &str,
    ) -> Result<Vec<RoutingStepEnriched>, RoutingError> {
        let exists: Option<(Uuid,)> = sqlx::query_as(
            "SELECT routing_template_id FROM routing_templates WHERE routing_template_id = $1 AND tenant_id = $2",
        )
        .bind(routing_template_id)
        .bind(tenant_id)
        .fetch_optional(pool)
        .await?;

        if exists.is_none() {
            return Err(RoutingError::NotFound);
        }

        let rows = sqlx::query_as::<_, EnrichedStepRow>(
            r#"
            SELECT
                rs.routing_step_id,
                rs.routing_template_id,
                rs.sequence_number,
                rs.workcenter_id,
                rs.operation_name,
                rs.description,
                rs.setup_time_minutes,
                rs.run_time_minutes,
                rs.is_required,
                rs.created_at,
                rs.updated_at,
                w.name  AS wc_name,
                w.code  AS wc_code
            FROM routing_steps rs
            LEFT JOIN workcenters w ON w.workcenter_id = rs.workcenter_id
            WHERE rs.routing_template_id = $1
              AND rs.routing_template_id IN (
                  SELECT routing_template_id FROM routing_templates WHERE tenant_id = $2
              )
            ORDER BY rs.sequence_number
            "#,
        )
        .bind(routing_template_id)
        .bind(tenant_id)
        .fetch_all(pool)
        .await
        .map_err(RoutingError::Database)?;

        Ok(rows
            .into_iter()
            .map(|r| {
                let workcenter = r.wc_name.as_ref().map(|name| WorkcenterDetails {
                    workcenter_id: r.workcenter_id,
                    name: name.clone(),
                    code: r.wc_code.clone().unwrap_or_default(),
                });
                RoutingStepEnriched {
                    routing_step_id: r.routing_step_id,
                    routing_template_id: r.routing_template_id,
                    sequence_number: r.sequence_number,
                    workcenter_id: r.workcenter_id,
                    workcenter_name: r.wc_name,
                    operation_name: r.operation_name,
                    description: r.description,
                    setup_time_minutes: r.setup_time_minutes,
                    run_time_minutes: r.run_time_minutes,
                    is_required: r.is_required,
                    created_at: r.created_at,
                    updated_at: r.updated_at,
                    workcenter,
                }
            })
            .collect())
    }

    /// Fetch a single routing step with full workcenter details embedded.
    /// Used when the caller passes `?include=workcenter_details`.
    pub async fn find_step_enriched(
        pool: &PgPool,
        routing_template_id: Uuid,
        step_id: Uuid,
        tenant_id: &str,
    ) -> Result<Option<RoutingStepEnriched>, RoutingError> {
        let exists: Option<(Uuid,)> = sqlx::query_as(
            "SELECT routing_template_id FROM routing_templates WHERE routing_template_id = $1 AND tenant_id = $2",
        )
        .bind(routing_template_id)
        .bind(tenant_id)
        .fetch_optional(pool)
        .await?;

        if exists.is_none() {
            return Err(RoutingError::NotFound);
        }

        let row = sqlx::query_as::<_, EnrichedStepRow>(
            r#"
            SELECT
                rs.routing_step_id,
                rs.routing_template_id,
                rs.sequence_number,
                rs.workcenter_id,
                rs.operation_name,
                rs.description,
                rs.setup_time_minutes,
                rs.run_time_minutes,
                rs.is_required,
                rs.created_at,
                rs.updated_at,
                w.name  AS wc_name,
                w.code  AS wc_code
            FROM routing_steps rs
            LEFT JOIN workcenters w ON w.workcenter_id = rs.workcenter_id
            WHERE rs.routing_step_id = $1
              AND rs.routing_template_id = $2
            "#,
        )
        .bind(step_id)
        .bind(routing_template_id)
        .fetch_optional(pool)
        .await
        .map_err(RoutingError::Database)?;

        Ok(row.map(|r| {
            let workcenter = r.wc_name.as_ref().map(|name| WorkcenterDetails {
                workcenter_id: r.workcenter_id,
                name: name.clone(),
                code: r.wc_code.clone().unwrap_or_default(),
            });
            RoutingStepEnriched {
                routing_step_id: r.routing_step_id,
                routing_template_id: r.routing_template_id,
                sequence_number: r.sequence_number,
                workcenter_id: r.workcenter_id,
                workcenter_name: r.wc_name,
                operation_name: r.operation_name,
                description: r.description,
                setup_time_minutes: r.setup_time_minutes,
                run_time_minutes: r.run_time_minutes,
                is_required: r.is_required,
                created_at: r.created_at,
                updated_at: r.updated_at,
                workcenter,
            }
        }))
    }

    pub async fn delete_step(
        pool: &PgPool,
        routing_template_id: Uuid,
        step_id: Uuid,
        tenant_id: &str,
        correlation_id: &str,
        causation_id: Option<&str>,
    ) -> Result<(), RoutingError> {
        let mut tx = pool.begin().await?;

        let routing = sqlx::query_as::<_, RoutingTemplate>(
            "SELECT * FROM routing_templates WHERE routing_template_id = $1 AND tenant_id = $2 FOR UPDATE",
        )
        .bind(routing_template_id)
        .bind(tenant_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(RoutingError::NotFound)?;

        if routing.status == "released" {
            return Err(RoutingError::ReleasedImmutable);
        }

        let result = sqlx::query(
            "DELETE FROM routing_steps WHERE routing_step_id = $1 AND routing_template_id = $2",
        )
        .bind(step_id)
        .bind(routing_template_id)
        .execute(&mut *tx)
        .await?;

        if result.rows_affected() == 0 {
            return Err(RoutingError::StepNotFound);
        }

        enqueue_event(
            &mut tx,
            tenant_id,
            ProductionEventType::RoutingUpdated,
            "routing_template",
            &routing_template_id.to_string(),
            &events::build_routing_updated_envelope(
                routing_template_id,
                tenant_id.to_string(),
                routing.name.clone(),
                routing.revision.clone(),
                correlation_id.to_string(),
                causation_id.map(String::from),
            ),
            correlation_id,
            causation_id,
        )
        .await?;

        tx.commit().await?;
        Ok(())
    }
}
