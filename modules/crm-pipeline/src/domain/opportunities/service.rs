//! Opportunity service.
//!
//! Key invariants enforced here:
//! 1. stage_code must reference an active pipeline_stages row
//! 2. advance-stage cannot target terminal stages
//! 3. close-lost requires close_reason
//! 4. once terminal, no further moves allowed

use chrono::{NaiveDate, Utc};
use sqlx::PgPool;
use uuid::Uuid;

use super::{
    repo, AdvanceStageRequest, CloseLostRequest, CloseWonRequest, CreateOpportunityRequest,
    ListOpportunitiesQuery, Opportunity, OpportunityDetail, OpportunityError,
    OpportunityStageHistory, PipelineSummaryItem, UpdateOpportunityRequest,
};
use crate::domain::pipeline_stages::{repo as stages_repo, service as stages_service};
use crate::events::{
    build_opportunity_closed_lost_envelope, build_opportunity_closed_won_envelope,
    build_opportunity_created_envelope, build_opportunity_stage_advanced_envelope,
    OpportunityClosedLostPayload, OpportunityClosedWonPayload, OpportunityCreatedPayload,
    OpportunityStageAdvancedPayload, EVENT_TYPE_OPPORTUNITY_CLOSED_LOST,
    EVENT_TYPE_OPPORTUNITY_CLOSED_WON, EVENT_TYPE_OPPORTUNITY_CREATED,
    EVENT_TYPE_OPPORTUNITY_STAGE_ADVANCED,
};
use crate::outbox;

pub async fn create_opportunity(
    pool: &PgPool,
    tenant_id: &str,
    req: &CreateOpportunityRequest,
    actor: String,
) -> Result<Opportunity, OpportunityError> {
    if req.title.trim().is_empty() {
        return Err(OpportunityError::Validation("title is required".into()));
    }

    // Ensure default stages exist
    crate::domain::pipeline_stages::service::ensure_default_stages(pool, tenant_id)
        .await
        .map_err(|e| OpportunityError::Validation(e.to_string()))?;

    // Determine initial stage
    let stage_code = if let Some(ref code) = req.stage_code {
        let stage = stages_repo::fetch_stage_active(pool, tenant_id, code)
            .await
            .map_err(|e| OpportunityError::Validation(e.to_string()))?;
        match stage {
            Some(s) if s.is_terminal => {
                return Err(OpportunityError::TerminalStageViaAdvance(code.clone()));
            }
            Some(s) => s.stage_code,
            None => return Err(OpportunityError::InvalidStage(code.clone())),
        }
    } else {
        stages_service::initial_stage(pool, tenant_id)
            .await
            .map_err(|e| OpportunityError::Validation(e.to_string()))?
            .stage_code
    };

    let probability_pct = req.probability_pct.unwrap_or(0);
    if !(0..=100).contains(&probability_pct) {
        return Err(OpportunityError::Validation(
            "probability_pct must be 0-100".into(),
        ));
    }

    let mut tx = pool.begin().await?;
    let opp_number = repo::next_opp_number(&mut *tx, tenant_id).await?;

    let opp = Opportunity {
        id: Uuid::new_v4(),
        tenant_id: tenant_id.to_string(),
        opp_number,
        title: req.title.clone(),
        party_id: req.party_id,
        primary_party_contact_id: req.primary_party_contact_id,
        lead_id: req.lead_id,
        stage_code: stage_code.clone(),
        probability_pct,
        estimated_value_cents: req.estimated_value_cents,
        currency: req.currency.clone().unwrap_or_else(|| "USD".to_string()),
        expected_close_date: req.expected_close_date,
        actual_close_date: None,
        close_reason: None,
        competitor: None,
        opp_type: req
            .opp_type
            .clone()
            .unwrap_or_else(|| "new_business".to_string()),
        priority: req.priority.clone().unwrap_or_else(|| "medium".to_string()),
        description: req.description.clone(),
        requirements: req.requirements.clone(),
        external_quote_ref: req.external_quote_ref.clone(),
        sales_order_id: None,
        owner_id: req.owner_id.clone(),
        created_by: actor.clone(),
        created_at: Utc::now(),
        updated_at: Utc::now(),
        ar_customer_id: None,
    };

    let created = repo::insert_opportunity(&mut *tx, &opp).await?;

    // Initial stage history entry
    let history = OpportunityStageHistory {
        id: Uuid::new_v4(),
        tenant_id: tenant_id.to_string(),
        opportunity_id: created.id,
        from_stage_code: None,
        to_stage_code: stage_code.clone(),
        probability_pct_at_change: Some(probability_pct),
        days_in_previous_stage: None,
        reason: Some("created".to_string()),
        notes: None,
        changed_by: actor,
        changed_at: Utc::now(),
    };
    repo::insert_stage_history(&mut *tx, &history).await?;

    let payload = OpportunityCreatedPayload {
        opportunity_id: created.id,
        tenant_id: tenant_id.to_string(),
        opp_number: created.opp_number.clone(),
        party_id: created.party_id,
        stage_code,
        estimated_value_cents: created.estimated_value_cents,
        created_at: created.created_at,
    };
    let envelope = build_opportunity_created_envelope(
        Uuid::new_v4(),
        tenant_id.to_string(),
        Uuid::new_v4().to_string(),
        payload,
    );
    outbox::enqueue_event_tx(
        &mut tx,
        Uuid::new_v4(),
        EVENT_TYPE_OPPORTUNITY_CREATED,
        "opportunity",
        &created.id.to_string(),
        &envelope,
    )
    .await?;

    tx.commit().await?;
    Ok(created)
}

pub async fn get_opportunity(
    pool: &PgPool,
    tenant_id: &str,
    id: Uuid,
) -> Result<Opportunity, OpportunityError> {
    repo::fetch_opportunity(pool, tenant_id, id)
        .await?
        .ok_or(OpportunityError::NotFound(id))
}

pub async fn get_opportunity_detail(
    pool: &PgPool,
    tenant_id: &str,
    id: Uuid,
) -> Result<OpportunityDetail, OpportunityError> {
    let opp = get_opportunity(pool, tenant_id, id).await?;
    let history = repo::list_stage_history(pool, tenant_id, id).await?;
    Ok(OpportunityDetail {
        opportunity: opp,
        stage_history: history,
    })
}

pub async fn list_opportunities(
    pool: &PgPool,
    tenant_id: &str,
    query: &ListOpportunitiesQuery,
) -> Result<Vec<Opportunity>, OpportunityError> {
    repo::list_opportunities(pool, tenant_id, query).await
}

pub async fn update_opportunity(
    pool: &PgPool,
    tenant_id: &str,
    id: Uuid,
    req: &UpdateOpportunityRequest,
) -> Result<Opportunity, OpportunityError> {
    let opp = get_opportunity(pool, tenant_id, id).await?;
    // Validate the opportunity is not terminal
    let stage = stages_repo::fetch_stage(pool, tenant_id, &opp.stage_code)
        .await
        .map_err(|e| OpportunityError::Validation(e.to_string()))?;
    if stage.map(|s| s.is_terminal).unwrap_or(false) {
        return Err(OpportunityError::AlreadyTerminal(opp.stage_code.clone()));
    }
    repo::update_opportunity_fields(pool, tenant_id, id, req).await
}

/// Advance to a non-terminal stage. Terminal stage targets are rejected.
pub async fn advance_stage(
    pool: &PgPool,
    tenant_id: &str,
    id: Uuid,
    req: &AdvanceStageRequest,
    actor: String,
) -> Result<Opportunity, OpportunityError> {
    let opp = get_opportunity(pool, tenant_id, id).await?;

    // Check current stage is not terminal
    let current_stage = stages_repo::fetch_stage(pool, tenant_id, &opp.stage_code)
        .await
        .map_err(|e| OpportunityError::Validation(e.to_string()))?;
    if current_stage
        .as_ref()
        .map(|s| s.is_terminal)
        .unwrap_or(false)
    {
        return Err(OpportunityError::AlreadyTerminal(opp.stage_code.clone()));
    }

    // Validate target stage exists and is active
    let target_stage = stages_repo::fetch_stage_active(pool, tenant_id, &req.stage_code)
        .await
        .map_err(|e| OpportunityError::Validation(e.to_string()))?
        .ok_or_else(|| OpportunityError::InvalidStage(req.stage_code.clone()))?;

    // INVARIANT: advance-stage cannot reach terminal stages
    if target_stage.is_terminal {
        return Err(OpportunityError::TerminalStageViaAdvance(
            req.stage_code.clone(),
        ));
    }

    let probability_pct = req
        .probability_pct
        .or(target_stage.probability_default_pct)
        .unwrap_or(opp.probability_pct);

    if !(0..=100).contains(&probability_pct) {
        return Err(OpportunityError::Validation(
            "probability_pct must be 0-100".into(),
        ));
    }

    // Compute days in previous stage
    let last_history = repo::list_stage_history(pool, tenant_id, id).await?;
    let days_in_prev = last_history.last().map(|h| {
        let duration = Utc::now().signed_duration_since(h.changed_at);
        duration.num_days() as i32
    });

    let mut tx = pool.begin().await?;
    let updated = repo::update_opportunity_stage(
        &mut *tx,
        tenant_id,
        id,
        &req.stage_code,
        probability_pct,
        None,
        None,
        None,
        None,
    )
    .await?;

    // Append stage history (invariant: append-only)
    let history = OpportunityStageHistory {
        id: Uuid::new_v4(),
        tenant_id: tenant_id.to_string(),
        opportunity_id: id,
        from_stage_code: Some(opp.stage_code.clone()),
        to_stage_code: req.stage_code.clone(),
        probability_pct_at_change: Some(probability_pct),
        days_in_previous_stage: days_in_prev,
        reason: req.reason.clone(),
        notes: req.notes.clone(),
        changed_by: actor,
        changed_at: Utc::now(),
    };
    repo::insert_stage_history(&mut *tx, &history).await?;

    let payload = OpportunityStageAdvancedPayload {
        opportunity_id: updated.id,
        tenant_id: tenant_id.to_string(),
        from_stage_code: opp.stage_code.clone(),
        to_stage_code: req.stage_code.clone(),
        probability_pct,
        days_in_previous_stage: days_in_prev,
    };
    let envelope = build_opportunity_stage_advanced_envelope(
        Uuid::new_v4(),
        tenant_id.to_string(),
        Uuid::new_v4().to_string(),
        payload,
    );
    outbox::enqueue_event_tx(
        &mut tx,
        Uuid::new_v4(),
        EVENT_TYPE_OPPORTUNITY_STAGE_ADVANCED,
        "opportunity",
        &updated.id.to_string(),
        &envelope,
    )
    .await?;

    tx.commit().await?;
    Ok(updated)
}

/// Close an opportunity as won. Uses the terminal is_win=true stage.
pub async fn close_won(
    pool: &PgPool,
    tenant_id: &str,
    id: Uuid,
    req: &CloseWonRequest,
    actor: String,
) -> Result<Opportunity, OpportunityError> {
    let opp = get_opportunity(pool, tenant_id, id).await?;

    let current_stage = stages_repo::fetch_stage(pool, tenant_id, &opp.stage_code)
        .await
        .map_err(|e| OpportunityError::Validation(e.to_string()))?;
    if current_stage
        .as_ref()
        .map(|s| s.is_terminal)
        .unwrap_or(false)
    {
        return Err(OpportunityError::AlreadyTerminal(opp.stage_code.clone()));
    }

    // Find the win terminal stage
    let stages = stages_repo::list_active_stages(pool, tenant_id)
        .await
        .map_err(|e| OpportunityError::Validation(e.to_string()))?;
    let won_stage = stages
        .into_iter()
        .find(|s| s.is_terminal && s.is_win)
        .ok_or_else(|| {
            OpportunityError::Validation("No active closed_won stage configured".into())
        })?;

    let days_in_prev = repo::list_stage_history(pool, tenant_id, id)
        .await?
        .last()
        .map(|h| Utc::now().signed_duration_since(h.changed_at).num_days() as i32);

    let today = chrono::Local::now().date_naive();
    let mut tx = pool.begin().await?;
    let updated = repo::update_opportunity_stage(
        &mut *tx,
        tenant_id,
        id,
        &won_stage.stage_code,
        100,
        Some(today),
        req.reason.as_deref(),
        None,
        req.sales_order_id,
    )
    .await?;

    let history = OpportunityStageHistory {
        id: Uuid::new_v4(),
        tenant_id: tenant_id.to_string(),
        opportunity_id: id,
        from_stage_code: Some(opp.stage_code.clone()),
        to_stage_code: won_stage.stage_code.clone(),
        probability_pct_at_change: Some(100),
        days_in_previous_stage: days_in_prev,
        reason: req
            .reason
            .clone()
            .or_else(|| Some("closed_won".to_string())),
        notes: req.notes.clone(),
        changed_by: actor,
        changed_at: Utc::now(),
    };
    repo::insert_stage_history(&mut *tx, &history).await?;

    let payload = OpportunityClosedWonPayload {
        opportunity_id: updated.id,
        tenant_id: tenant_id.to_string(),
        party_id: updated.party_id,
        actual_close_date: updated.actual_close_date.unwrap_or(today),
        estimated_value_cents: updated.estimated_value_cents,
        sales_order_id: updated.sales_order_id,
    };
    let envelope = build_opportunity_closed_won_envelope(
        Uuid::new_v4(),
        tenant_id.to_string(),
        Uuid::new_v4().to_string(),
        payload,
    );
    outbox::enqueue_event_tx(
        &mut tx,
        Uuid::new_v4(),
        EVENT_TYPE_OPPORTUNITY_CLOSED_WON,
        "opportunity",
        &updated.id.to_string(),
        &envelope,
    )
    .await?;

    tx.commit().await?;
    Ok(updated)
}

/// Close an opportunity as lost. Requires close_reason.
pub async fn close_lost(
    pool: &PgPool,
    tenant_id: &str,
    id: Uuid,
    req: &CloseLostRequest,
    actor: String,
) -> Result<Opportunity, OpportunityError> {
    // INVARIANT: close_reason required
    if req.close_reason.trim().is_empty() {
        return Err(OpportunityError::CloseLostRequiresReason);
    }

    let opp = get_opportunity(pool, tenant_id, id).await?;
    let current_stage = stages_repo::fetch_stage(pool, tenant_id, &opp.stage_code)
        .await
        .map_err(|e| OpportunityError::Validation(e.to_string()))?;
    if current_stage
        .as_ref()
        .map(|s| s.is_terminal)
        .unwrap_or(false)
    {
        return Err(OpportunityError::AlreadyTerminal(opp.stage_code.clone()));
    }

    // Find the lost terminal stage
    let stages = stages_repo::list_active_stages(pool, tenant_id)
        .await
        .map_err(|e| OpportunityError::Validation(e.to_string()))?;
    let lost_stage = stages
        .into_iter()
        .find(|s| s.is_terminal && !s.is_win)
        .ok_or_else(|| {
            OpportunityError::Validation("No active closed_lost stage configured".into())
        })?;

    let days_in_prev = repo::list_stage_history(pool, tenant_id, id)
        .await?
        .last()
        .map(|h| Utc::now().signed_duration_since(h.changed_at).num_days() as i32);

    let today = chrono::Local::now().date_naive();
    let mut tx = pool.begin().await?;
    let updated = repo::update_opportunity_stage(
        &mut *tx,
        tenant_id,
        id,
        &lost_stage.stage_code,
        0,
        Some(today),
        Some(&req.close_reason),
        req.competitor.as_deref(),
        None,
    )
    .await?;

    let history = OpportunityStageHistory {
        id: Uuid::new_v4(),
        tenant_id: tenant_id.to_string(),
        opportunity_id: id,
        from_stage_code: Some(opp.stage_code.clone()),
        to_stage_code: lost_stage.stage_code.clone(),
        probability_pct_at_change: Some(0),
        days_in_previous_stage: days_in_prev,
        reason: Some(req.close_reason.clone()),
        notes: req.notes.clone(),
        changed_by: actor,
        changed_at: Utc::now(),
    };
    repo::insert_stage_history(&mut *tx, &history).await?;

    let payload = OpportunityClosedLostPayload {
        opportunity_id: updated.id,
        tenant_id: tenant_id.to_string(),
        party_id: updated.party_id,
        actual_close_date: updated.actual_close_date.unwrap_or(today),
        close_reason: req.close_reason.clone(),
        competitor: req.competitor.clone(),
    };
    let envelope = build_opportunity_closed_lost_envelope(
        Uuid::new_v4(),
        tenant_id.to_string(),
        Uuid::new_v4().to_string(),
        payload,
    );
    outbox::enqueue_event_tx(
        &mut tx,
        Uuid::new_v4(),
        EVENT_TYPE_OPPORTUNITY_CLOSED_LOST,
        "opportunity",
        &updated.id.to_string(),
        &envelope,
    )
    .await?;

    tx.commit().await?;
    Ok(updated)
}

pub async fn list_stage_history(
    pool: &PgPool,
    tenant_id: &str,
    id: Uuid,
) -> Result<Vec<OpportunityStageHistory>, OpportunityError> {
    get_opportunity(pool, tenant_id, id).await?;
    repo::list_stage_history(pool, tenant_id, id).await
}

pub async fn pipeline_summary(
    pool: &PgPool,
    tenant_id: &str,
    owner_id: Option<&str>,
) -> Result<Vec<PipelineSummaryItem>, OpportunityError> {
    repo::pipeline_summary(pool, tenant_id, owner_id).await
}
