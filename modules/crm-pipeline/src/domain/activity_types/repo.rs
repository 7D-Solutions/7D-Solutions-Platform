//! Activity type repository.

use chrono::Utc;
use sqlx::PgPool;
use uuid::Uuid;

use super::{ActivityType, ActivityTypeError, CreateActivityTypeRequest, UpdateActivityTypeRequest};

pub async fn list_activity_types(pool: &PgPool, tenant_id: &str) -> Result<Vec<ActivityType>, ActivityTypeError> {
    let rows = sqlx::query_as::<_, ActivityType>(
        "SELECT * FROM activity_types WHERE tenant_id = $1 AND active = TRUE ORDER BY display_label ASC",
    )
    .bind(tenant_id)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

pub async fn create_activity_type(
    pool: &PgPool,
    tenant_id: &str,
    req: &CreateActivityTypeRequest,
    actor: &str,
) -> Result<ActivityType, ActivityTypeError> {
    if req.activity_type_code.trim().is_empty() {
        return Err(ActivityTypeError::Validation("activity_type_code is required".into()));
    }
    if req.display_label.trim().is_empty() {
        return Err(ActivityTypeError::Validation("display_label is required".into()));
    }

    let now = Utc::now();
    let row = sqlx::query_as::<_, ActivityType>(
        r#"
        INSERT INTO activity_types (id, tenant_id, activity_type_code, display_label, icon, active, created_at, updated_at, updated_by)
        VALUES ($1, $2, $3, $4, $5, TRUE, $6, $7, $8)
        RETURNING *
        "#,
    )
    .bind(Uuid::new_v4())
    .bind(tenant_id)
    .bind(req.activity_type_code.trim().to_lowercase())
    .bind(&req.display_label)
    .bind(&req.icon)
    .bind(now)
    .bind(now)
    .bind(actor)
    .fetch_one(pool)
    .await
    .map_err(|e| {
        if let sqlx::Error::Database(ref de) = e {
            if de.constraint() == Some("activity_types_tenant_id_activity_type_code_key") {
                return ActivityTypeError::DuplicateCode(req.activity_type_code.clone());
            }
        }
        ActivityTypeError::Database(e)
    })?;
    Ok(row)
}

pub async fn update_activity_type(
    pool: &PgPool,
    tenant_id: &str,
    code: &str,
    req: &UpdateActivityTypeRequest,
    actor: &str,
) -> Result<ActivityType, ActivityTypeError> {
    let row = sqlx::query_as::<_, ActivityType>(
        r#"
        UPDATE activity_types SET
            display_label = COALESCE($3, display_label),
            icon          = COALESCE($4, icon),
            active        = COALESCE($5, active),
            updated_at    = NOW(),
            updated_by    = $6
        WHERE tenant_id = $1 AND activity_type_code = $2
        RETURNING *
        "#,
    )
    .bind(tenant_id)
    .bind(code)
    .bind(&req.display_label)
    .bind(&req.icon)
    .bind(req.active)
    .bind(actor)
    .fetch_one(pool)
    .await
    .map_err(|e| match e {
        sqlx::Error::RowNotFound => ActivityTypeError::NotFound(code.to_string()),
        other => ActivityTypeError::Database(other),
    })?;
    Ok(row)
}
