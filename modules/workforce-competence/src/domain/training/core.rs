//! Training delivery mutations.
//!
//! Pattern: Guard → Mutation → Outbox (all in one transaction)
//!
//! Critical invariant for record_training_completion:
//!   outcome=passed → competence_assignment INSERT happens FIRST, then completion INSERT.
//!   wc_training_completions.resulting_competence_assignment_id has a FK to wc_operator_competences,
//!   so the referenced row must exist before the completion row. Both are in the same transaction:
//!   if the competence_assignment insert fails, no completion is persisted either.

use chrono::{Duration, Utc};
use serde::Serialize;
use sqlx::PgPool;
use uuid::Uuid;

use crate::{
    domain::{
        guards::{guard_non_empty, GuardError},
        service::ServiceError,
    },
    events::{
        create_wc_envelope, MUTATION_CLASS_DATA_MUTATION, WC_EVENT_SCHEMA_VERSION,
        EVENT_TYPE_TRAINING_ASSIGNED, EVENT_TYPE_TRAINING_COMPLETED, EVENT_TYPE_TRAINING_PLANNED,
    },
};

use super::{
    CreateTrainingAssignmentRequest, CreateTrainingPlanRequest, RecordCompletionRequest,
    TrainingAssignment, TrainingAssignmentRow, TrainingCompletion, TrainingCompletedPayload,
    TrainingAssignedPayload, TrainingOutcome, TrainingPlan, TrainingPlannedPayload,
    TransitionAssignmentRequest,
};

// ============================================================================
// Create training plan
// ============================================================================

pub async fn create_training_plan(
    pool: &PgPool,
    req: &CreateTrainingPlanRequest,
) -> Result<(TrainingPlan, bool), ServiceError> {
    guard_non_empty(&req.tenant_id, "tenant_id")?;
    guard_non_empty(&req.plan_code, "plan_code")?;
    guard_non_empty(&req.title, "title")?;
    guard_non_empty(&req.idempotency_key, "idempotency_key")?;

    if req.duration_minutes <= 0 {
        return Err(ServiceError::Guard(GuardError::Validation(
            "duration_minutes must be positive".to_string(),
        )));
    }

    let request_hash = serde_json::to_string(req)?;
    if let Some(rec) = find_idem(pool, &req.tenant_id, &req.idempotency_key).await? {
        if rec.request_hash != request_hash {
            return Err(ServiceError::ConflictingIdempotencyKey);
        }
        return Ok((serde_json::from_str(&rec.response_body)?, true));
    }

    // Guard: artifact must exist and belong to tenant
    let artifact_exists: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM wc_competence_artifacts WHERE id = $1 AND tenant_id = $2 AND is_active = true)",
    )
    .bind(req.artifact_id)
    .bind(&req.tenant_id)
    .fetch_one(pool)
    .await?;

    if !artifact_exists {
        return Err(ServiceError::Guard(GuardError::ArtifactNotFound));
    }

    let now = Utc::now();
    let id = Uuid::new_v4();
    let event_id = Uuid::new_v4();
    let corr = req
        .correlation_id
        .clone()
        .unwrap_or_else(|| Uuid::new_v4().to_string());
    let material_refs = req.material_refs.clone().unwrap_or_default();
    let required_for = req.required_for_artifact_codes.clone().unwrap_or_default();

    let mut tx = pool.begin().await?;

    sqlx::query(
        r#"
        INSERT INTO wc_training_plans
            (id, tenant_id, plan_code, title, description, artifact_id,
             duration_minutes, instructor_id, material_refs, required_for_artifact_codes,
             location, scheduled_at, active, created_at, updated_at, updated_by)
        VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,true,$13,$13,$14)
        "#,
    )
    .bind(id)
    .bind(&req.tenant_id)
    .bind(&req.plan_code)
    .bind(&req.title)
    .bind(&req.description)
    .bind(req.artifact_id)
    .bind(req.duration_minutes)
    .bind(req.instructor_id)
    .bind(&material_refs)
    .bind(&required_for)
    .bind(&req.location)
    .bind(req.scheduled_at)
    .bind(now)
    .bind(&req.updated_by)
    .execute(&mut *tx)
    .await?;

    let plan = TrainingPlan {
        id,
        tenant_id: req.tenant_id.clone(),
        plan_code: req.plan_code.clone(),
        title: req.title.clone(),
        description: req.description.clone(),
        artifact_id: req.artifact_id,
        duration_minutes: req.duration_minutes,
        instructor_id: req.instructor_id,
        material_refs: material_refs.clone(),
        required_for_artifact_codes: required_for.clone(),
        location: req.location.clone(),
        scheduled_at: req.scheduled_at,
        active: true,
        created_at: now,
        updated_at: now,
        updated_by: req.updated_by.clone(),
    };

    let payload = TrainingPlannedPayload {
        plan_id: id,
        tenant_id: req.tenant_id.clone(),
        plan_code: req.plan_code.clone(),
        artifact_id: req.artifact_id,
        title: req.title.clone(),
        scheduled_at: req.scheduled_at,
    };
    write_outbox_event(
        &mut tx,
        event_id,
        EVENT_TYPE_TRAINING_PLANNED,
        id,
        &req.tenant_id,
        &corr,
        &req.causation_id,
        payload,
    )
    .await?;

    write_idem_key(&mut tx, &req.tenant_id, &req.idempotency_key, &request_hash, &plan, 201, now)
        .await?;

    tx.commit().await?;
    Ok((plan, false))
}

// ============================================================================
// Create training assignment
// ============================================================================

pub async fn create_training_assignment(
    pool: &PgPool,
    req: &CreateTrainingAssignmentRequest,
) -> Result<(TrainingAssignment, bool), ServiceError> {
    guard_non_empty(&req.tenant_id, "tenant_id")?;
    guard_non_empty(&req.assigned_by, "assigned_by")?;
    guard_non_empty(&req.idempotency_key, "idempotency_key")?;

    let request_hash = serde_json::to_string(req)?;
    if let Some(rec) = find_idem(pool, &req.tenant_id, &req.idempotency_key).await? {
        if rec.request_hash != request_hash {
            return Err(ServiceError::ConflictingIdempotencyKey);
        }
        return Ok((serde_json::from_str(&rec.response_body)?, true));
    }

    // Guard: plan must exist and be active for this tenant
    let plan_exists: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM wc_training_plans WHERE id = $1 AND tenant_id = $2 AND active = true)",
    )
    .bind(req.plan_id)
    .bind(&req.tenant_id)
    .fetch_one(pool)
    .await?;

    if !plan_exists {
        return Err(ServiceError::Guard(GuardError::Validation(
            "training plan not found or inactive".to_string(),
        )));
    }

    let now = Utc::now();
    let id = Uuid::new_v4();
    let event_id = Uuid::new_v4();
    let assigned_at = Utc::now();
    let corr = req
        .correlation_id
        .clone()
        .unwrap_or_else(|| Uuid::new_v4().to_string());

    let mut tx = pool.begin().await?;

    sqlx::query(
        r#"
        INSERT INTO wc_training_assignments
            (id, tenant_id, plan_id, operator_id, assigned_by, assigned_at,
             status, scheduled_at, notes, updated_at)
        VALUES ($1,$2,$3,$4,$5,$6,'assigned',$7,$8,$9)
        "#,
    )
    .bind(id)
    .bind(&req.tenant_id)
    .bind(req.plan_id)
    .bind(req.operator_id)
    .bind(&req.assigned_by)
    .bind(assigned_at)
    .bind(req.scheduled_at)
    .bind(&req.notes)
    .bind(now)
    .execute(&mut *tx)
    .await?;

    let assignment = TrainingAssignment {
        id,
        tenant_id: req.tenant_id.clone(),
        plan_id: req.plan_id,
        operator_id: req.operator_id,
        assigned_by: req.assigned_by.clone(),
        assigned_at,
        status: super::TrainingStatus::Assigned,
        scheduled_at: req.scheduled_at,
        notes: req.notes.clone(),
        updated_at: now,
    };

    let payload = TrainingAssignedPayload {
        assignment_id: id,
        tenant_id: req.tenant_id.clone(),
        plan_id: req.plan_id,
        operator_id: req.operator_id,
        assigned_by: req.assigned_by.clone(),
        assigned_at,
    };
    write_outbox_event(
        &mut tx,
        event_id,
        EVENT_TYPE_TRAINING_ASSIGNED,
        id,
        &req.tenant_id,
        &corr,
        &req.causation_id,
        payload,
    )
    .await?;

    write_idem_key(
        &mut tx,
        &req.tenant_id,
        &req.idempotency_key,
        &request_hash,
        &assignment,
        201,
        now,
    )
    .await?;

    tx.commit().await?;
    Ok((assignment, false))
}

// ============================================================================
// Transition assignment status
// ============================================================================

pub async fn transition_assignment_status(
    pool: &PgPool,
    req: &TransitionAssignmentRequest,
) -> Result<TrainingAssignment, ServiceError> {
    guard_non_empty(&req.tenant_id, "tenant_id")?;

    let current_status: Option<String> = sqlx::query_scalar(
        "SELECT status FROM wc_training_assignments WHERE id = $1 AND tenant_id = $2",
    )
    .bind(req.assignment_id)
    .bind(&req.tenant_id)
    .fetch_optional(pool)
    .await?;

    let current = current_status.ok_or_else(|| {
        ServiceError::Guard(GuardError::Validation(
            "training assignment not found".to_string(),
        ))
    })?;

    // Terminal states cannot transition further
    if matches!(current.as_str(), "completed" | "cancelled" | "no_show") {
        return Err(ServiceError::Guard(GuardError::Validation(format!(
            "assignment in terminal status '{current}' cannot transition"
        ))));
    }

    let now = Utc::now();
    sqlx::query(
        r#"
        UPDATE wc_training_assignments
        SET status = $1, scheduled_at = COALESCE($2, scheduled_at),
            notes = COALESCE($3, notes), updated_at = $4
        WHERE id = $5 AND tenant_id = $6
        "#,
    )
    .bind(req.new_status.to_string())
    .bind(req.scheduled_at)
    .bind(&req.notes)
    .bind(now)
    .bind(req.assignment_id)
    .bind(&req.tenant_id)
    .execute(pool)
    .await?;

    let row = sqlx::query_as::<_, TrainingAssignmentRow>(
        r#"
        SELECT id, tenant_id, plan_id, operator_id, assigned_by, assigned_at,
               status, scheduled_at, notes, updated_at
        FROM wc_training_assignments WHERE id = $1 AND tenant_id = $2
        "#,
    )
    .bind(req.assignment_id)
    .bind(&req.tenant_id)
    .fetch_one(pool)
    .await?;

    Ok(row.into())
}

// ============================================================================
// Record training completion (atomic: completion + competence_assignment)
// ============================================================================

pub async fn record_training_completion(
    pool: &PgPool,
    req: &RecordCompletionRequest,
) -> Result<(TrainingCompletion, bool), ServiceError> {
    guard_non_empty(&req.tenant_id, "tenant_id")?;
    guard_non_empty(&req.idempotency_key, "idempotency_key")?;

    let request_hash = serde_json::to_string(req)?;
    if let Some(rec) = find_idem(pool, &req.tenant_id, &req.idempotency_key).await? {
        if rec.request_hash != request_hash {
            return Err(ServiceError::ConflictingIdempotencyKey);
        }
        return Ok((serde_json::from_str(&rec.response_body)?, true));
    }

    // Fetch assignment to get operator_id and plan_id
    #[derive(sqlx::FromRow)]
    struct AssignmentLookup {
        operator_id: Uuid,
        plan_id: Uuid,
    }
    let assignment = sqlx::query_as::<_, AssignmentLookup>(
        "SELECT operator_id, plan_id FROM wc_training_assignments WHERE id = $1 AND tenant_id = $2",
    )
    .bind(req.assignment_id)
    .bind(&req.tenant_id)
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| {
        ServiceError::Guard(GuardError::Validation(
            "training assignment not found".to_string(),
        ))
    })?;

    // Fetch training plan to get artifact_id
    #[derive(sqlx::FromRow)]
    struct PlanLookup {
        artifact_id: Uuid,
    }
    let plan = sqlx::query_as::<_, PlanLookup>(
        "SELECT artifact_id FROM wc_training_plans WHERE id = $1 AND tenant_id = $2",
    )
    .bind(assignment.plan_id)
    .bind(&req.tenant_id)
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| {
        ServiceError::Guard(GuardError::Validation(
            "training plan not found".to_string(),
        ))
    })?;

    // If outcome=passed, fetch artifact for expiry calculation
    let (competence_assignment_id, expires_at) = if req.outcome == TrainingOutcome::Passed {
        #[derive(sqlx::FromRow)]
        struct ArtifactLookup {
            valid_duration_days: Option<i32>,
        }
        let artifact = sqlx::query_as::<_, ArtifactLookup>(
            "SELECT valid_duration_days FROM wc_competence_artifacts WHERE id = $1 AND tenant_id = $2",
        )
        .bind(plan.artifact_id)
        .bind(&req.tenant_id)
        .fetch_optional(pool)
        .await?
        .ok_or(ServiceError::Guard(GuardError::ArtifactNotFound))?;

        let exp = artifact
            .valid_duration_days
            .map(|d| req.completed_at + Duration::days(d as i64));
        (Some(Uuid::new_v4()), exp)
    } else {
        (None, None)
    };

    let now = Utc::now();
    let completion_id = Uuid::new_v4();
    let event_id = Uuid::new_v4();
    let corr = req
        .correlation_id
        .clone()
        .unwrap_or_else(|| Uuid::new_v4().to_string());

    let mut tx = pool.begin().await?;

    // Step 1: If passed, INSERT competence_assignment FIRST.
    // wc_training_completions.resulting_competence_assignment_id is a FK to wc_operator_competences,
    // so the referenced row must exist in the DB before the completion row can be inserted.
    // Both are in this transaction: if this insert fails, the completion is never inserted.
    if let Some(ca_id) = competence_assignment_id {
        let idem_key_ca = format!("training-completion-{completion_id}");
        sqlx::query(
            r#"
            INSERT INTO wc_operator_competences
                (id, tenant_id, operator_id, artifact_id, awarded_at, expires_at,
                 evidence_ref, awarded_by, is_revoked, created_at, idempotency_key)
            VALUES ($1,$2,$3,$4,$5,$6,$7,$8,false,$9,$10)
            "#,
        )
        .bind(ca_id)
        .bind(&req.tenant_id)
        .bind(assignment.operator_id)
        .bind(plan.artifact_id)
        .bind(req.completed_at)
        .bind(expires_at)
        .bind(&req.evidence_ref)
        .bind(req.verified_by.as_deref().unwrap_or("training-completion"))
        .bind(now)
        .bind(&idem_key_ca)
        .execute(&mut *tx)
        .await?;

        // Outbox event for the auto-created competence assignment
        let ca_event_id = Uuid::new_v4();
        let ca_payload = crate::domain::service::core::CompetenceAssignedPayload {
            assignment_id: ca_id,
            tenant_id: req.tenant_id.clone(),
            operator_id: assignment.operator_id,
            artifact_id: plan.artifact_id,
            awarded_at: req.completed_at,
            expires_at,
        };
        let ca_envelope = create_wc_envelope(
            ca_event_id,
            req.tenant_id.clone(),
            crate::events::EVENT_TYPE_COMPETENCE_ASSIGNED.to_string(),
            corr.clone(),
            req.causation_id.clone(),
            MUTATION_CLASS_DATA_MUTATION.to_string(),
            ca_payload,
        );
        let ca_json = serde_json::to_string(&ca_envelope)?;
        sqlx::query(
            r#"
            INSERT INTO wc_outbox
                (event_id, event_type, aggregate_type, aggregate_id, tenant_id,
                 payload, correlation_id, causation_id, schema_version)
            VALUES ($1,$2,'operator_competence',$3,$4,$5::JSONB,$6,$7,$8)
            "#,
        )
        .bind(ca_event_id)
        .bind(crate::events::EVENT_TYPE_COMPETENCE_ASSIGNED)
        .bind(ca_id.to_string())
        .bind(&req.tenant_id)
        .bind(&ca_json)
        .bind(&corr)
        .bind(&req.causation_id)
        .bind(WC_EVENT_SCHEMA_VERSION)
        .execute(&mut *tx)
        .await?;
    }

    // Step 2: INSERT training completion (resulting_competence_assignment_id FK is now satisfied)
    sqlx::query(
        r#"
        INSERT INTO wc_training_completions
            (id, tenant_id, assignment_id, operator_id, plan_id, completed_at,
             verified_by, outcome, notes, resulting_competence_assignment_id, created_at)
        VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11)
        "#,
    )
    .bind(completion_id)
    .bind(&req.tenant_id)
    .bind(req.assignment_id)
    .bind(assignment.operator_id)
    .bind(assignment.plan_id)
    .bind(req.completed_at)
    .bind(&req.verified_by)
    .bind(req.outcome.to_string())
    .bind(&req.notes)
    .bind(competence_assignment_id)
    .bind(now)
    .execute(&mut *tx)
    .await?;

    // Step 3: Outbox event for training_completed
    let payload = TrainingCompletedPayload {
        completion_id,
        tenant_id: req.tenant_id.clone(),
        assignment_id: req.assignment_id,
        operator_id: assignment.operator_id,
        plan_id: assignment.plan_id,
        completed_at: req.completed_at,
        outcome: req.outcome.to_string(),
        resulting_competence_assignment_id: competence_assignment_id,
    };
    write_outbox_event(
        &mut tx,
        event_id,
        EVENT_TYPE_TRAINING_COMPLETED,
        completion_id,
        &req.tenant_id,
        &corr,
        &req.causation_id,
        payload,
    )
    .await?;

    let completion = TrainingCompletion {
        id: completion_id,
        tenant_id: req.tenant_id.clone(),
        assignment_id: req.assignment_id,
        operator_id: assignment.operator_id,
        plan_id: assignment.plan_id,
        completed_at: req.completed_at,
        verified_by: req.verified_by.clone(),
        outcome: req.outcome.clone(),
        notes: req.notes.clone(),
        resulting_competence_assignment_id: competence_assignment_id,
        created_at: now,
    };

    write_idem_key(
        &mut tx,
        &req.tenant_id,
        &req.idempotency_key,
        &request_hash,
        &completion,
        201,
        now,
    )
    .await?;

    tx.commit().await?;
    Ok((completion, false))
}

// ============================================================================
// Helpers (private)
// ============================================================================

struct IdempotencyRecord {
    response_body: String,
    request_hash: String,
}

async fn find_idem(
    pool: &PgPool,
    tenant_id: &str,
    key: &str,
) -> Result<Option<IdempotencyRecord>, sqlx::Error> {
    struct Row {
        response_body: String,
        request_hash: String,
    }
    impl sqlx::FromRow<'_, sqlx::postgres::PgRow> for Row {
        fn from_row(row: &sqlx::postgres::PgRow) -> Result<Self, sqlx::Error> {
            use sqlx::Row;
            Ok(Self {
                response_body: row.try_get("response_body")?,
                request_hash: row.try_get("request_hash")?,
            })
        }
    }
    let row = sqlx::query_as::<_, Row>(
        "SELECT response_body::TEXT AS response_body, request_hash
         FROM wc_idempotency_keys WHERE tenant_id = $1 AND idempotency_key = $2",
    )
    .bind(tenant_id)
    .bind(key)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(|r| IdempotencyRecord {
        response_body: r.response_body,
        request_hash: r.request_hash,
    }))
}

async fn write_outbox_event<T: Serialize>(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    event_id: Uuid,
    event_type: &str,
    aggregate_id: Uuid,
    tenant_id: &str,
    correlation_id: &str,
    causation_id: &Option<String>,
    payload: T,
) -> Result<(), ServiceError> {
    let envelope = create_wc_envelope(
        event_id,
        tenant_id.to_string(),
        event_type.to_string(),
        correlation_id.to_string(),
        causation_id.clone(),
        MUTATION_CLASS_DATA_MUTATION.to_string(),
        payload,
    );
    let json = serde_json::to_string(&envelope)?;
    sqlx::query(
        r#"
        INSERT INTO wc_outbox
            (event_id, event_type, aggregate_type, aggregate_id, tenant_id,
             payload, correlation_id, causation_id, schema_version)
        VALUES ($1,$2,'training_delivery',$3,$4,$5::JSONB,$6,$7,$8)
        "#,
    )
    .bind(event_id)
    .bind(event_type)
    .bind(aggregate_id.to_string())
    .bind(tenant_id)
    .bind(&json)
    .bind(correlation_id)
    .bind(causation_id)
    .bind(WC_EVENT_SCHEMA_VERSION)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

async fn write_idem_key<T: Serialize>(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    tenant_id: &str,
    key: &str,
    hash: &str,
    response: &T,
    status_code: i32,
    now: chrono::DateTime<Utc>,
) -> Result<(), ServiceError> {
    let json = serde_json::to_string(response)?;
    sqlx::query(
        r#"
        INSERT INTO wc_idempotency_keys
            (tenant_id, idempotency_key, request_hash, response_body, status_code, expires_at)
        VALUES ($1,$2,$3,$4::JSONB,$5,$6)
        "#,
    )
    .bind(tenant_id)
    .bind(key)
    .bind(hash)
    .bind(&json)
    .bind(status_code)
    .bind(now + Duration::days(7))
    .execute(&mut **tx)
    .await?;
    Ok(())
}
