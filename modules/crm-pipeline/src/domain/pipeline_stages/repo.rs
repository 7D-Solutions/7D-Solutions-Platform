//! Pipeline stage repository.

use sqlx::{PgConnection, PgPool};
use uuid::Uuid;

use super::{PipelineStage, StageError, DEFAULT_STAGES};

pub async fn fetch_stage(
    pool: &PgPool,
    tenant_id: &str,
    stage_code: &str,
) -> Result<Option<PipelineStage>, StageError> {
    let row = sqlx::query_as::<_, PipelineStage>(
        "SELECT * FROM pipeline_stages WHERE tenant_id = $1 AND stage_code = $2",
    )
    .bind(tenant_id)
    .bind(stage_code)
    .fetch_optional(pool)
    .await?;
    Ok(row)
}

pub async fn fetch_stage_active(
    pool: &PgPool,
    tenant_id: &str,
    stage_code: &str,
) -> Result<Option<PipelineStage>, StageError> {
    let row = sqlx::query_as::<_, PipelineStage>(
        "SELECT * FROM pipeline_stages WHERE tenant_id = $1 AND stage_code = $2 AND active = TRUE",
    )
    .bind(tenant_id)
    .bind(stage_code)
    .fetch_optional(pool)
    .await?;
    Ok(row)
}

pub async fn list_stages(pool: &PgPool, tenant_id: &str) -> Result<Vec<PipelineStage>, StageError> {
    let rows = sqlx::query_as::<_, PipelineStage>(
        "SELECT * FROM pipeline_stages WHERE tenant_id = $1 ORDER BY order_rank ASC",
    )
    .bind(tenant_id)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

pub async fn list_active_stages(pool: &PgPool, tenant_id: &str) -> Result<Vec<PipelineStage>, StageError> {
    let rows = sqlx::query_as::<_, PipelineStage>(
        "SELECT * FROM pipeline_stages WHERE tenant_id = $1 AND active = TRUE ORDER BY order_rank ASC",
    )
    .bind(tenant_id)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

/// Returns the initial (lowest order_rank non-terminal active) stage for the tenant.
pub async fn initial_stage(pool: &PgPool, tenant_id: &str) -> Result<Option<PipelineStage>, StageError> {
    let row = sqlx::query_as::<_, PipelineStage>(
        r#"
        SELECT * FROM pipeline_stages
        WHERE tenant_id = $1 AND active = TRUE AND is_terminal = FALSE
        ORDER BY order_rank ASC
        LIMIT 1
        "#,
    )
    .bind(tenant_id)
    .fetch_optional(pool)
    .await?;
    Ok(row)
}

pub async fn insert_stage(
    conn: &mut PgConnection,
    stage: &PipelineStage,
) -> Result<PipelineStage, StageError> {
    let row = sqlx::query_as::<_, PipelineStage>(
        r#"
        INSERT INTO pipeline_stages (
            id, tenant_id, stage_code, display_label, description, order_rank,
            is_terminal, is_win, probability_default_pct, active, created_at, updated_at, updated_by
        ) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13)
        RETURNING *
        "#,
    )
    .bind(stage.id)
    .bind(&stage.tenant_id)
    .bind(&stage.stage_code)
    .bind(&stage.display_label)
    .bind(&stage.description)
    .bind(stage.order_rank)
    .bind(stage.is_terminal)
    .bind(stage.is_win)
    .bind(stage.probability_default_pct)
    .bind(stage.active)
    .bind(stage.created_at)
    .bind(stage.updated_at)
    .bind(&stage.updated_by)
    .fetch_one(conn)
    .await
    .map_err(|e| {
        if let sqlx::Error::Database(ref de) = e {
            if de.constraint() == Some("pipeline_stages_tenant_id_stage_code_key") {
                return StageError::DuplicateCode(stage.stage_code.clone());
            }
        }
        StageError::Database(e)
    })?;
    Ok(row)
}

pub async fn update_stage(
    pool: &PgPool,
    tenant_id: &str,
    stage_code: &str,
    req: &super::UpdateStageRequest,
    actor: &str,
) -> Result<PipelineStage, StageError> {
    let row = sqlx::query_as::<_, PipelineStage>(
        r#"
        UPDATE pipeline_stages SET
            display_label        = COALESCE($3, display_label),
            description          = COALESCE($4, description),
            order_rank           = COALESCE($5, order_rank),
            probability_default_pct = COALESCE($6, probability_default_pct),
            updated_at           = NOW(),
            updated_by           = $7
        WHERE tenant_id = $1 AND stage_code = $2
        RETURNING *
        "#,
    )
    .bind(tenant_id)
    .bind(stage_code)
    .bind(&req.display_label)
    .bind(&req.description)
    .bind(req.order_rank)
    .bind(req.probability_default_pct)
    .bind(actor)
    .fetch_one(pool)
    .await
    .map_err(|e| match e {
        sqlx::Error::RowNotFound => StageError::NotFound(stage_code.to_string()),
        other => StageError::Database(other),
    })?;
    Ok(row)
}

pub async fn deactivate_stage(
    pool: &PgPool,
    tenant_id: &str,
    stage_code: &str,
    actor: &str,
) -> Result<PipelineStage, StageError> {
    let row = sqlx::query_as::<_, PipelineStage>(
        r#"
        UPDATE pipeline_stages SET active = FALSE, updated_at = NOW(), updated_by = $3
        WHERE tenant_id = $1 AND stage_code = $2
        RETURNING *
        "#,
    )
    .bind(tenant_id)
    .bind(stage_code)
    .bind(actor)
    .fetch_one(pool)
    .await
    .map_err(|e| match e {
        sqlx::Error::RowNotFound => StageError::NotFound(stage_code.to_string()),
        other => StageError::Database(other),
    })?;
    Ok(row)
}

pub async fn reorder_stages(
    conn: &mut PgConnection,
    tenant_id: &str,
    items: &[(String, i32)],
) -> Result<(), StageError> {
    for (code, rank) in items {
        sqlx::query(
            "UPDATE pipeline_stages SET order_rank = $3, updated_at = NOW() WHERE tenant_id = $1 AND stage_code = $2",
        )
        .bind(tenant_id)
        .bind(code)
        .bind(rank)
        .execute(&mut *conn)
        .await?;
    }
    Ok(())
}

/// Seed default pipeline stages for a tenant if none exist.
pub async fn seed_default_stages(conn: &mut PgConnection, tenant_id: &str) -> Result<(), StageError> {
    let count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM pipeline_stages WHERE tenant_id = $1")
            .bind(tenant_id)
            .fetch_one(&mut *conn)
            .await?;
    if count > 0 {
        return Ok(());
    }

    let now = chrono::Utc::now();
    for (code, label, rank, terminal, win, prob) in DEFAULT_STAGES {
        let stage = PipelineStage {
            id: Uuid::new_v4(),
            tenant_id: tenant_id.to_string(),
            stage_code: code.to_string(),
            display_label: label.to_string(),
            description: None,
            order_rank: *rank,
            is_terminal: *terminal,
            is_win: *win,
            probability_default_pct: *prob,
            active: true,
            created_at: now,
            updated_at: now,
            updated_by: Some("system".to_string()),
        };
        insert_stage(conn, &stage).await?;
    }
    Ok(())
}
