//! Scorecard service — Guard→Mutation→Outbox atomicity.

use chrono::Utc;
use rust_decimal::Decimal;
use rust_decimal::prelude::FromPrimitive;
use sqlx::PgPool;
use uuid::Uuid;

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
    let rows: Vec<Scorecard> = sqlx::query_as(
        r#"
        SELECT id, party_id, app_id, metric_name, score, max_score,
               review_date, reviewer, notes, idempotency_key, metadata,
               created_at, updated_at
        FROM party_scorecards
        WHERE party_id = $1 AND app_id = $2
        ORDER BY review_date DESC, metric_name ASC
        "#,
    )
    .bind(party_id)
    .bind(app_id)
    .fetch_all(pool)
    .await?;

    Ok(rows)
}

/// Get a single scorecard by ID, scoped to app_id.
pub async fn get_scorecard(
    pool: &PgPool,
    app_id: &str,
    scorecard_id: Uuid,
) -> Result<Option<Scorecard>, PartyError> {
    let row: Option<Scorecard> = sqlx::query_as(
        r#"
        SELECT id, party_id, app_id, metric_name, score, max_score,
               review_date, reviewer, notes, idempotency_key, metadata,
               created_at, updated_at
        FROM party_scorecards
        WHERE id = $1 AND app_id = $2
        "#,
    )
    .bind(scorecard_id)
    .bind(app_id)
    .fetch_optional(pool)
    .await?;

    Ok(row)
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
        let existing: Option<Scorecard> = sqlx::query_as(
            r#"
            SELECT id, party_id, app_id, metric_name, score, max_score,
                   review_date, reviewer, notes, idempotency_key, metadata,
                   created_at, updated_at
            FROM party_scorecards
            WHERE app_id = $1 AND idempotency_key = $2
            "#,
        )
        .bind(app_id)
        .bind(idem_key)
        .fetch_optional(pool)
        .await?;

        if let Some(existing) = existing {
            return Ok(existing);
        }
    }

    let sc_id = Uuid::new_v4();
    let event_id = Uuid::new_v4();
    let now = Utc::now();
    let score = Decimal::from_f64(req.score).unwrap_or_default();
    let max_score = Decimal::from_f64(req.max_score.unwrap_or(100.0)).unwrap_or_default();

    let mut tx = pool.begin().await?;

    // Guard: party must exist
    guard_party_exists_tx(&mut tx, app_id, party_id).await?;

    // Mutation
    let sc: Scorecard = sqlx::query_as(
        r#"
        INSERT INTO party_scorecards (
            id, party_id, app_id, metric_name, score, max_score,
            review_date, reviewer, notes, idempotency_key, metadata,
            created_at, updated_at
        )
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $12)
        RETURNING id, party_id, app_id, metric_name, score, max_score,
                  review_date, reviewer, notes, idempotency_key, metadata,
                  created_at, updated_at
        "#,
    )
    .bind(sc_id)
    .bind(party_id)
    .bind(app_id)
    .bind(req.metric_name.trim())
    .bind(score)
    .bind(max_score)
    .bind(req.review_date)
    .bind(&req.reviewer)
    .bind(&req.notes)
    .bind(&req.idempotency_key)
    .bind(&req.metadata)
    .bind(now)
    .fetch_one(&mut *tx)
    .await?;

    // Outbox
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

    // Guard
    let existing: Option<Scorecard> = sqlx::query_as(
        r#"
        SELECT id, party_id, app_id, metric_name, score, max_score,
               review_date, reviewer, notes, idempotency_key, metadata,
               created_at, updated_at
        FROM party_scorecards
        WHERE id = $1 AND app_id = $2
        FOR UPDATE
        "#,
    )
    .bind(scorecard_id)
    .bind(app_id)
    .fetch_optional(&mut *tx)
    .await?;

    let current = existing.ok_or(PartyError::NotFound(scorecard_id))?;

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

    // Mutation
    let updated: Scorecard = sqlx::query_as(
        r#"
        UPDATE party_scorecards
        SET metric_name = $1, score = $2, max_score = $3, review_date = $4,
            reviewer = $5, notes = $6, metadata = $7, updated_at = $8
        WHERE id = $9 AND app_id = $10
        RETURNING id, party_id, app_id, metric_name, score, max_score,
                  review_date, reviewer, notes, idempotency_key, metadata,
                  created_at, updated_at
        "#,
    )
    .bind(&new_metric)
    .bind(new_score)
    .bind(new_max)
    .bind(new_review_date)
    .bind(&new_reviewer)
    .bind(&new_notes)
    .bind(&new_metadata)
    .bind(now)
    .bind(scorecard_id)
    .bind(app_id)
    .fetch_one(&mut *tx)
    .await?;

    // Outbox
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

// ============================================================================
// Helpers
// ============================================================================

async fn guard_party_exists_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    app_id: &str,
    party_id: Uuid,
) -> Result<(), PartyError> {
    let exists: Option<(Uuid,)> =
        sqlx::query_as("SELECT id FROM party_parties WHERE id = $1 AND app_id = $2")
            .bind(party_id)
            .bind(app_id)
            .fetch_optional(&mut **tx)
            .await?;

    if exists.is_none() {
        return Err(PartyError::NotFound(party_id));
    }
    Ok(())
}
