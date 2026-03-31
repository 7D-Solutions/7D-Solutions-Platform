use chrono::NaiveDate;
use sqlx::PgPool;
use uuid::Uuid;

use crate::domain::outbox::enqueue_event;
use crate::events::{self, ProductionEventType};

use super::types::{
    AddRoutingStepRequest, CreateRoutingRequest, RoutingError, RoutingStatus, RoutingStep,
    RoutingTemplate, UpdateRoutingRequest,
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

        let revision = req.revision.as_deref().unwrap_or("1");

        let mut tx = pool.begin().await?;

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
    ) -> Result<Vec<RoutingTemplate>, RoutingError> {
        sqlx::query_as::<_, RoutingTemplate>(
            "SELECT * FROM routing_templates WHERE tenant_id = $1 ORDER BY name",
        )
        .bind(tenant_id)
        .fetch_all(pool)
        .await
        .map_err(RoutingError::Database)
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

        let mut tx = pool.begin().await?;

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
            INSERT INTO routing_steps
                (routing_template_id, sequence_number, workcenter_id, operation_name,
                 description, setup_time_minutes, run_time_minutes, is_required)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
            RETURNING *
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
            "SELECT * FROM routing_steps WHERE routing_template_id = $1 ORDER BY sequence_number",
        )
        .bind(routing_template_id)
        .fetch_all(pool)
        .await
        .map_err(RoutingError::Database)
    }
}
