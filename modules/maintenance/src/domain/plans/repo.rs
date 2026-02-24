//! Plan and assignment repositories — database access layer.

use chrono::Utc;
use sqlx::PgPool;
use uuid::Uuid;

use super::{
    recompute_due, AssignPlanRequest, CreatePlanRequest, ListAssignmentsQuery,
    ListPlansQuery, MaintenancePlan, PlanAssignment, PlanError, UpdatePlanRequest,
};
use crate::domain::work_orders::types::{Priority, ScheduleType};

pub struct PlanRepo;

impl PlanRepo {
    pub async fn create(pool: &PgPool, req: &CreatePlanRequest) -> Result<MaintenancePlan, PlanError> {
        if req.tenant_id.trim().is_empty() {
            return Err(PlanError::Validation("tenant_id is required".into()));
        }
        if req.name.trim().is_empty() {
            return Err(PlanError::Validation("name is required".into()));
        }

        let stype = ScheduleType::from_str_value(&req.schedule_type)
            .map_err(|e| PlanError::Validation(e.to_string()))?;

        // Validate required fields per schedule_type
        match stype {
            ScheduleType::Calendar => {
                if req.calendar_interval_days.is_none() {
                    return Err(PlanError::Validation(
                        "calendar_interval_days required for calendar schedule".into(),
                    ));
                }
            }
            ScheduleType::Meter => {
                if req.meter_type_id.is_none() || req.meter_interval.is_none() {
                    return Err(PlanError::Validation(
                        "meter_type_id and meter_interval required for meter schedule".into(),
                    ));
                }
            }
            ScheduleType::Both => {
                if req.calendar_interval_days.is_none()
                    || req.meter_type_id.is_none()
                    || req.meter_interval.is_none()
                {
                    return Err(PlanError::Validation(
                        "calendar_interval_days, meter_type_id, and meter_interval required for both schedule".into(),
                    ));
                }
            }
        }

        if let Some(days) = req.calendar_interval_days {
            if days <= 0 {
                return Err(PlanError::Validation(
                    "calendar_interval_days must be positive".into(),
                ));
            }
        }
        if let Some(interval) = req.meter_interval {
            if interval <= 0 {
                return Err(PlanError::Validation(
                    "meter_interval must be positive".into(),
                ));
            }
        }

        let priority_str = req.priority.as_deref().unwrap_or("medium");
        Priority::from_str_value(priority_str)
            .map_err(|e| PlanError::Validation(e.to_string()))?;

        // Verify meter_type exists if provided
        if let Some(mt_id) = req.meter_type_id {
            let exists: Option<(Uuid,)> = sqlx::query_as(
                "SELECT id FROM meter_types WHERE id = $1 AND tenant_id = $2",
            )
            .bind(mt_id)
            .bind(&req.tenant_id)
            .fetch_optional(pool)
            .await?;
            if exists.is_none() {
                return Err(PlanError::MeterTypeNotFound);
            }
        }

        sqlx::query_as::<_, MaintenancePlan>(
            r#"
            INSERT INTO maintenance_plans
                (tenant_id, name, description, asset_type_filter, schedule_type,
                 calendar_interval_days, meter_type_id, meter_interval,
                 priority, estimated_duration_minutes, estimated_cost_minor, task_checklist)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)
            RETURNING *
            "#,
        )
        .bind(&req.tenant_id)
        .bind(req.name.trim())
        .bind(req.description.as_deref())
        .bind(req.asset_type_filter.as_deref())
        .bind(&req.schedule_type)
        .bind(req.calendar_interval_days)
        .bind(req.meter_type_id)
        .bind(req.meter_interval)
        .bind(priority_str)
        .bind(req.estimated_duration_minutes)
        .bind(req.estimated_cost_minor)
        .bind(&req.task_checklist)
        .fetch_one(pool)
        .await
        .map_err(PlanError::Database)
    }

    pub async fn find_by_id(
        pool: &PgPool,
        id: Uuid,
        tenant_id: &str,
    ) -> Result<Option<MaintenancePlan>, PlanError> {
        sqlx::query_as::<_, MaintenancePlan>(
            "SELECT * FROM maintenance_plans WHERE id = $1 AND tenant_id = $2",
        )
        .bind(id)
        .bind(tenant_id)
        .fetch_optional(pool)
        .await
        .map_err(PlanError::Database)
    }

    pub async fn list(pool: &PgPool, q: &ListPlansQuery) -> Result<Vec<MaintenancePlan>, PlanError> {
        if q.tenant_id.trim().is_empty() {
            return Err(PlanError::Validation("tenant_id is required".into()));
        }
        let limit = q.limit.unwrap_or(50).min(100).max(1);
        let offset = q.offset.unwrap_or(0).max(0);

        sqlx::query_as::<_, MaintenancePlan>(
            r#"
            SELECT * FROM maintenance_plans
            WHERE tenant_id = $1
              AND ($2::BOOL IS NULL OR is_active = $2)
            ORDER BY created_at DESC
            LIMIT $3 OFFSET $4
            "#,
        )
        .bind(&q.tenant_id)
        .bind(q.is_active)
        .bind(limit)
        .bind(offset)
        .fetch_all(pool)
        .await
        .map_err(PlanError::Database)
    }

    pub async fn update(
        pool: &PgPool,
        id: Uuid,
        tenant_id: &str,
        req: &UpdatePlanRequest,
    ) -> Result<MaintenancePlan, PlanError> {
        if let Some(ref name) = req.name {
            if name.trim().is_empty() {
                return Err(PlanError::Validation("name must not be empty".into()));
            }
        }
        if let Some(ref p) = req.priority {
            Priority::from_str_value(p)
                .map_err(|e| PlanError::Validation(e.to_string()))?;
        }

        sqlx::query_as::<_, MaintenancePlan>(
            r#"
            UPDATE maintenance_plans SET
                name                       = COALESCE($3, name),
                description                = COALESCE($4, description),
                priority                   = COALESCE($5, priority),
                estimated_duration_minutes = COALESCE($6, estimated_duration_minutes),
                estimated_cost_minor       = COALESCE($7, estimated_cost_minor),
                task_checklist             = COALESCE($8, task_checklist),
                is_active                  = COALESCE($9, is_active),
                updated_at                 = NOW()
            WHERE id = $1 AND tenant_id = $2
            RETURNING *
            "#,
        )
        .bind(id)
        .bind(tenant_id)
        .bind(req.name.as_deref())
        .bind(req.description.as_deref())
        .bind(req.priority.as_deref())
        .bind(req.estimated_duration_minutes)
        .bind(req.estimated_cost_minor)
        .bind(&req.task_checklist)
        .bind(req.is_active)
        .fetch_optional(pool)
        .await?
        .ok_or(PlanError::PlanNotFound)
    }
}

pub struct AssignmentRepo;

impl AssignmentRepo {
    /// Assign a plan to an asset. Computes initial next_due fields.
    pub async fn assign(
        pool: &PgPool,
        plan_id: Uuid,
        req: &AssignPlanRequest,
    ) -> Result<PlanAssignment, PlanError> {
        if req.tenant_id.trim().is_empty() {
            return Err(PlanError::Validation("tenant_id is required".into()));
        }

        let mut tx = pool.begin().await?;

        // Verify plan exists
        let plan = sqlx::query_as::<_, MaintenancePlan>(
            "SELECT * FROM maintenance_plans WHERE id = $1 AND tenant_id = $2",
        )
        .bind(plan_id)
        .bind(&req.tenant_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(PlanError::PlanNotFound)?;

        // Verify asset exists
        let asset_exists: Option<(Uuid,)> = sqlx::query_as(
            "SELECT id FROM maintainable_assets WHERE id = $1 AND tenant_id = $2",
        )
        .bind(req.asset_id)
        .bind(&req.tenant_id)
        .fetch_optional(&mut *tx)
        .await?;
        if asset_exists.is_none() {
            return Err(PlanError::AssetNotFound);
        }

        // Fetch latest meter reading if plan is meter-based
        let latest_reading: Option<i64> = if plan.meter_type_id.is_some() {
            sqlx::query_scalar(
                r#"
                SELECT MAX(reading_value) FROM meter_readings
                WHERE tenant_id = $1 AND asset_id = $2 AND meter_type_id = $3
                "#,
            )
            .bind(&req.tenant_id)
            .bind(req.asset_id)
            .bind(plan.meter_type_id)
            .fetch_one(&mut *tx)
            .await?
        } else {
            None
        };

        let now = Utc::now();
        let schedule_type = ScheduleType::from_str_value(plan.schedule_type.as_str())
            .map_err(|e| PlanError::Validation(e.to_string()))?;

        let (next_due_date, next_due_meter) = recompute_due(
            schedule_type,
            plan.calendar_interval_days,
            plan.meter_interval,
            None,
            None,
            latest_reading,
            now,
        );

        let assignment = sqlx::query_as::<_, PlanAssignment>(
            r#"
            INSERT INTO maintenance_plan_assignments
                (tenant_id, plan_id, asset_id, next_due_date, next_due_meter)
            VALUES ($1, $2, $3, $4, $5)
            RETURNING *
            "#,
        )
        .bind(&req.tenant_id)
        .bind(plan_id)
        .bind(req.asset_id)
        .bind(next_due_date)
        .bind(next_due_meter)
        .fetch_one(&mut *tx)
        .await
        .map_err(|e| {
            if let sqlx::Error::Database(ref dbe) = e {
                if dbe.code().as_deref() == Some("23505") {
                    return PlanError::DuplicateAssignment;
                }
            }
            PlanError::Database(e)
        })?;

        // Enqueue outbox event
        let event_payload = serde_json::json!({
            "plan_id": plan_id,
            "asset_id": req.asset_id,
            "next_due_date": next_due_date,
            "next_due_meter": next_due_meter,
        });
        crate::outbox::enqueue_event_tx(
            &mut tx,
            Uuid::new_v4(),
            "maintenance.plan.assigned",
            "plan_assignment",
            &assignment.id.to_string(),
            &event_payload,
        )
        .await?;

        tx.commit().await?;
        Ok(assignment)
    }

    pub async fn list(
        pool: &PgPool,
        q: &ListAssignmentsQuery,
    ) -> Result<Vec<PlanAssignment>, PlanError> {
        if q.tenant_id.trim().is_empty() {
            return Err(PlanError::Validation("tenant_id is required".into()));
        }
        let limit = q.limit.unwrap_or(50).min(100).max(1);
        let offset = q.offset.unwrap_or(0).max(0);

        sqlx::query_as::<_, PlanAssignment>(
            r#"
            SELECT * FROM maintenance_plan_assignments
            WHERE tenant_id = $1
              AND ($2::UUID IS NULL OR plan_id = $2)
              AND ($3::UUID IS NULL OR asset_id = $3)
            ORDER BY created_at DESC
            LIMIT $4 OFFSET $5
            "#,
        )
        .bind(&q.tenant_id)
        .bind(q.plan_id)
        .bind(q.asset_id)
        .bind(limit)
        .bind(offset)
        .fetch_all(pool)
        .await
        .map_err(PlanError::Database)
    }

    pub async fn find_by_id(
        pool: &PgPool,
        id: Uuid,
        tenant_id: &str,
    ) -> Result<Option<PlanAssignment>, PlanError> {
        sqlx::query_as::<_, PlanAssignment>(
            "SELECT * FROM maintenance_plan_assignments WHERE id = $1 AND tenant_id = $2",
        )
        .bind(id)
        .bind(tenant_id)
        .fetch_optional(pool)
        .await
        .map_err(PlanError::Database)
    }
}
