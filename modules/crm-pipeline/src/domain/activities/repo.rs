//! Activity repository.

use chrono::Utc;
use sqlx::PgPool;
use uuid::Uuid;

use super::{Activity, ActivityError, ListActivitiesQuery};

pub async fn insert_activity(
    conn: &mut sqlx::PgConnection,
    act: &Activity,
) -> Result<Activity, ActivityError> {
    let row = sqlx::query_as::<_, Activity>(
        r#"
        INSERT INTO activities (
            id, tenant_id, activity_type_code, subject, description, activity_date,
            duration_minutes, lead_id, opportunity_id, party_id, party_contact_id,
            due_date, is_completed, completed_at, assigned_to, created_by, created_at, updated_at
        ) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$18)
        RETURNING *
        "#,
    )
    .bind(act.id)
    .bind(&act.tenant_id)
    .bind(&act.activity_type_code)
    .bind(&act.subject)
    .bind(&act.description)
    .bind(act.activity_date)
    .bind(act.duration_minutes)
    .bind(act.lead_id)
    .bind(act.opportunity_id)
    .bind(act.party_id)
    .bind(act.party_contact_id)
    .bind(act.due_date)
    .bind(act.is_completed)
    .bind(act.completed_at)
    .bind(&act.assigned_to)
    .bind(&act.created_by)
    .bind(act.created_at)
    .bind(act.updated_at)
    .fetch_one(conn)
    .await?;
    Ok(row)
}

pub async fn fetch_activity(
    pool: &PgPool,
    tenant_id: &str,
    id: Uuid,
) -> Result<Option<Activity>, ActivityError> {
    let row =
        sqlx::query_as::<_, Activity>("SELECT * FROM activities WHERE id = $1 AND tenant_id = $2")
            .bind(id)
            .bind(tenant_id)
            .fetch_optional(pool)
            .await?;
    Ok(row)
}

pub async fn list_activities(
    pool: &PgPool,
    tenant_id: &str,
    query: &ListActivitiesQuery,
) -> Result<Vec<Activity>, ActivityError> {
    let include_completed = query.include_completed.unwrap_or(false);
    let rows = sqlx::query_as::<_, Activity>(
        r#"
        SELECT * FROM activities
        WHERE tenant_id = $1
          AND ($2::text IS NULL OR assigned_to = $2)
          AND ($3::uuid IS NULL OR lead_id = $3)
          AND ($4::uuid IS NULL OR opportunity_id = $4)
          AND ($5::uuid IS NULL OR party_id = $5)
          AND ($6::boolean = TRUE OR is_completed = FALSE)
          AND ($7::date IS NULL OR due_date <= $7)
        ORDER BY activity_date DESC
        "#,
    )
    .bind(tenant_id)
    .bind(&query.assigned_to)
    .bind(query.lead_id)
    .bind(query.opportunity_id)
    .bind(query.party_id)
    .bind(include_completed)
    .bind(query.due_before)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

pub async fn complete_activity(
    conn: &mut sqlx::PgConnection,
    tenant_id: &str,
    id: Uuid,
) -> Result<Activity, ActivityError> {
    let row = sqlx::query_as::<_, Activity>(
        r#"
        UPDATE activities SET is_completed = TRUE, completed_at = NOW(), updated_at = NOW()
        WHERE id = $1 AND tenant_id = $2
        RETURNING *
        "#,
    )
    .bind(id)
    .bind(tenant_id)
    .fetch_one(conn)
    .await
    .map_err(|e| match e {
        sqlx::Error::RowNotFound => ActivityError::NotFound(id),
        other => ActivityError::Database(other),
    })?;
    Ok(row)
}

pub async fn update_activity(
    pool: &PgPool,
    tenant_id: &str,
    id: Uuid,
    req: &super::UpdateActivityRequest,
) -> Result<Activity, ActivityError> {
    let row = sqlx::query_as::<_, Activity>(
        r#"
        UPDATE activities SET
            subject          = COALESCE($3, subject),
            description      = COALESCE($4, description),
            activity_date    = COALESCE($5, activity_date),
            duration_minutes = COALESCE($6, duration_minutes),
            due_date         = COALESCE($7, due_date),
            assigned_to      = COALESCE($8, assigned_to),
            updated_at       = NOW()
        WHERE id = $1 AND tenant_id = $2
        RETURNING *
        "#,
    )
    .bind(id)
    .bind(tenant_id)
    .bind(&req.subject)
    .bind(&req.description)
    .bind(req.activity_date)
    .bind(req.duration_minutes)
    .bind(req.due_date)
    .bind(&req.assigned_to)
    .fetch_one(pool)
    .await
    .map_err(|e| match e {
        sqlx::Error::RowNotFound => ActivityError::NotFound(id),
        other => ActivityError::Database(other),
    })?;
    Ok(row)
}
