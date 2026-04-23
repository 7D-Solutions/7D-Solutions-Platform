//! Pipeline stage service.

use chrono::Utc;
use sqlx::PgPool;
use uuid::Uuid;

use super::{
    repo, CreateStageRequest, PipelineStage, ReorderStagesRequest, StageError, UpdateStageRequest,
};

pub async fn list_stages(pool: &PgPool, tenant_id: &str) -> Result<Vec<PipelineStage>, StageError> {
    repo::list_stages(pool, tenant_id).await
}

pub async fn initial_stage(pool: &PgPool, tenant_id: &str) -> Result<PipelineStage, StageError> {
    repo::initial_stage(pool, tenant_id).await?.ok_or_else(|| {
        StageError::Validation("No active non-terminal stages found for tenant".into())
    })
}

pub async fn create_stage(
    pool: &PgPool,
    tenant_id: &str,
    req: &CreateStageRequest,
    actor: &str,
) -> Result<PipelineStage, StageError> {
    if req.stage_code.trim().is_empty() {
        return Err(StageError::Validation("stage_code is required".into()));
    }
    if req.display_label.trim().is_empty() {
        return Err(StageError::Validation("display_label is required".into()));
    }
    if let Some(p) = req.probability_default_pct {
        if !(0..=100).contains(&p) {
            return Err(StageError::Validation(
                "probability_default_pct must be 0-100".into(),
            ));
        }
    }

    // Guard: prevent duplicate order_rank among active non-terminal stages.
    // Two non-terminal stages at the same order_rank make the initial stage ambiguous.
    if !req.is_terminal {
        let conflict: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM pipeline_stages WHERE tenant_id = $1 AND active = TRUE AND is_terminal = FALSE AND order_rank = $2",
        )
        .bind(tenant_id)
        .bind(req.order_rank)
        .fetch_one(pool)
        .await?;
        if conflict > 0 {
            return Err(StageError::MultipleInitialStages);
        }
    }

    // is_win only valid on terminal stages
    let is_win = if req.is_terminal {
        req.is_win.unwrap_or(false)
    } else {
        false
    };

    let now = Utc::now();
    let stage = PipelineStage {
        id: Uuid::new_v4(),
        tenant_id: tenant_id.to_string(),
        stage_code: req.stage_code.trim().to_lowercase(),
        display_label: req.display_label.clone(),
        description: req.description.clone(),
        order_rank: req.order_rank,
        is_terminal: req.is_terminal,
        is_win,
        probability_default_pct: req.probability_default_pct,
        active: true,
        created_at: now,
        updated_at: now,
        updated_by: Some(actor.to_string()),
    };

    let mut conn = pool.acquire().await?;
    repo::insert_stage(&mut conn, &stage).await
}

pub async fn update_stage(
    pool: &PgPool,
    tenant_id: &str,
    stage_code: &str,
    req: &UpdateStageRequest,
    actor: &str,
) -> Result<PipelineStage, StageError> {
    repo::update_stage(pool, tenant_id, stage_code, req, actor).await
}

pub async fn deactivate_stage(
    pool: &PgPool,
    tenant_id: &str,
    stage_code: &str,
    actor: &str,
) -> Result<PipelineStage, StageError> {
    repo::deactivate_stage(pool, tenant_id, stage_code, actor).await
}

pub async fn reorder_stages(
    pool: &PgPool,
    tenant_id: &str,
    req: &ReorderStagesRequest,
    actor: &str,
) -> Result<Vec<PipelineStage>, StageError> {
    let items: Vec<(String, i32)> = req
        .stages
        .iter()
        .map(|s| (s.stage_code.clone(), s.order_rank))
        .collect();
    let mut conn = pool.acquire().await?;
    repo::reorder_stages(&mut conn, tenant_id, &items).await?;
    repo::list_stages(pool, tenant_id).await
}

pub async fn ensure_default_stages(pool: &PgPool, tenant_id: &str) -> Result<(), StageError> {
    let mut conn = pool.acquire().await?;
    repo::seed_default_stages(&mut conn, tenant_id).await
}
