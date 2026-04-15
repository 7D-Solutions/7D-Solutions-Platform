//! Scorecard service — Guard→Mutation→Outbox atomicity.

use chrono::Utc;
use rust_decimal::prelude::FromPrimitive;
use rust_decimal::Decimal;
use sqlx::PgPool;
use uuid::Uuid;

use crate::db::{party_repo, scorecard_repo, scorecard_repo::UpdateScorecardData};
use crate::events::{
    build_scorecard_created_envelope, build_scorecard_updated_envelope, ScorecardPayload,
    EVENT_TYPE_SCORECARD_CREATED, EVENT_TYPE_SCORECARD_UPDATED,
};
use crate::outbox::enqueue_event_tx;

use super::party::PartyError;
use super::scorecard::{CreateScorecardRequest, Scorecard, UpdateScorecardRequest};

// ============================================================================
// Reads
// ============================================================================

/// List scorecards for a party, scoped to app_id.
pub async fn list_scorecards(
    pool: &PgPool,
    app_id: &str,
    party_id: Uuid,
) -> Result<Vec<Scorecard>, PartyError> {
    scorecard_repo::list_scorecards(pool, app_id, party_id).await
}

/// Get a single scorecard by ID, scoped to app_id.
pub async fn get_scorecard(
    pool: &PgPool,
    app_id: &str,
    scorecard_id: Uuid,
) -> Result<Option<Scorecard>, PartyError> {
    scorecard_repo::get_scorecard(pool, app_id, scorecard_id).await
}

// ============================================================================
// Writes
// ============================================================================

/// Create a scorecard entry. Idempotent via idempotency_key.
/// Emits `party.scorecard.created` via the outbox.
pub async fn create_scorecard(
    pool: &PgPool,
    app_id: &str,
    party_id: Uuid,
    req: &CreateScorecardRequest,
    correlation_id: String,
) -> Result<Scorecard, PartyError> {
    req.validate()?;

    // Idempotency check
    if let Some(ref idem_key) = req.idempotency_key {
        if let Some(existing) =
            scorecard_repo::find_scorecard_by_idempotency_key(pool, app_id, idem_key).await?
        {
            return Ok(existing);
        }
    }

    let sc_id = Uuid::new_v4();
    let event_id = Uuid::new_v4();
    let now = Utc::now();
    let score = Decimal::from_f64(req.score).unwrap_or_default();
    let max_score = Decimal::from_f64(req.max_score.unwrap_or(100.0)).unwrap_or_default();

    let mut tx = pool.begin().await?;

    party_repo::guard_party_exists_tx(&mut tx, app_id, party_id).await?;

    let sc = scorecard_repo::insert_scorecard_tx(
        &mut tx, sc_id, party_id, app_id, req, score, max_score, now,
    )
    .await?;

    let payload = ScorecardPayload {
        scorecard_id: sc_id,
        party_id,
        app_id: app_id.to_string(),
        metric_name: sc.metric_name.clone(),
        score: req.score,
        review_date: sc.review_date,
    };

    let envelope = build_scorecard_created_envelope(
        event_id,
        app_id.to_string(),
        correlation_id,
        None,
        payload,
    );

    enqueue_event_tx(
        &mut tx,
        event_id,
        EVENT_TYPE_SCORECARD_CREATED,
        "scorecard",
        &sc_id.to_string(),
        app_id,
        &envelope,
    )
    .await?;

    tx.commit().await?;
    Ok(sc)
}

/// Update a scorecard. Emits `party.scorecard.updated`.
pub async fn update_scorecard(
    pool: &PgPool,
    app_id: &str,
    scorecard_id: Uuid,
    req: &UpdateScorecardRequest,
    correlation_id: String,
) -> Result<Scorecard, PartyError> {
    req.validate()?;

    let event_id = Uuid::new_v4();
    let now = Utc::now();

    let mut tx = pool.begin().await?;

    let current = scorecard_repo::fetch_scorecard_for_update_tx(&mut tx, app_id, scorecard_id)
        .await?
        .ok_or(PartyError::NotFound(scorecard_id))?;

    let new_metric = req
        .metric_name
        .as_deref()
        .map(|m| m.trim().to_string())
        .unwrap_or(current.metric_name);
    let new_score = req
        .score
        .and_then(Decimal::from_f64)
        .unwrap_or(current.score);
    let new_max = req
        .max_score
        .and_then(Decimal::from_f64)
        .unwrap_or(current.max_score);
    let new_review_date = req.review_date.unwrap_or(current.review_date);
    let new_reviewer = if req.reviewer.is_some() {
        req.reviewer.clone()
    } else {
        current.reviewer
    };
    let new_notes = if req.notes.is_some() {
        req.notes.clone()
    } else {
        current.notes
    };
    let new_metadata = if req.metadata.is_some() {
        req.metadata.clone()
    } else {
        current.metadata
    };

    let updated = scorecard_repo::update_scorecard_row_tx(
        &mut tx,
        &UpdateScorecardData {
            scorecard_id,
            app_id,
            metric_name: new_metric,
            score: new_score,
            max_score: new_max,
            review_date: new_review_date,
            reviewer: new_reviewer,
            notes: new_notes,
            metadata: new_metadata,
            updated_at: now,
        },
    )
    .await?;

    let score_f64 = req.score.unwrap_or_else(|| {
        use rust_decimal::prelude::ToPrimitive;
        updated.score.to_f64().unwrap_or(0.0)
    });
    let payload = ScorecardPayload {
        scorecard_id,
        party_id: updated.party_id,
        app_id: app_id.to_string(),
        metric_name: updated.metric_name.clone(),
        score: score_f64,
        review_date: updated.review_date,
    };

    let envelope = build_scorecard_updated_envelope(
        event_id,
        app_id.to_string(),
        correlation_id,
        None,
        payload,
    );

    enqueue_event_tx(
        &mut tx,
        event_id,
        EVENT_TYPE_SCORECARD_UPDATED,
        "scorecard",
        &scorecard_id.to_string(),
        app_id,
        &envelope,
    )
    .await?;

    tx.commit().await?;
    Ok(updated)
}
