//! Scorecard repository — all SQL for `party_scorecards`.

use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

use crate::domain::party::PartyError;
use crate::domain::scorecard::{CreateScorecardRequest, Scorecard, UpdateScorecardRequest};

// ── Reads ─────────────────────────────────────────────────────────────────────

pub async fn list_scorecards(
    pool: &PgPool,
    app_id: &str,
    party_id: Uuid,
) -> Result<Vec<Scorecard>, PartyError> {
    Ok(sqlx::query_as(
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
    .await?)
}

pub async fn get_scorecard(
    pool: &PgPool,
    app_id: &str,
    scorecard_id: Uuid,
) -> Result<Option<Scorecard>, PartyError> {
    Ok(sqlx::query_as(
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
    .await?)
}

pub async fn find_scorecard_by_idempotency_key(
    pool: &PgPool,
    app_id: &str,
    idem_key: &str,
) -> Result<Option<Scorecard>, PartyError> {
    Ok(sqlx::query_as(
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
    .await?)
}

// ── Transaction helpers ───────────────────────────────────────────────────────

pub async fn insert_scorecard_tx(
    tx: &mut Transaction<'_, Postgres>,
    sc_id: Uuid,
    party_id: Uuid,
    app_id: &str,
    req: &CreateScorecardRequest,
    score: Decimal,
    max_score: Decimal,
    now: DateTime<Utc>,
) -> Result<Scorecard, PartyError> {
    Ok(sqlx::query_as(
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
    .fetch_one(&mut **tx)
    .await?)
}

pub async fn fetch_scorecard_for_update_tx(
    tx: &mut Transaction<'_, Postgres>,
    app_id: &str,
    scorecard_id: Uuid,
) -> Result<Option<Scorecard>, PartyError> {
    Ok(sqlx::query_as(
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
    .fetch_optional(&mut **tx)
    .await?)
}

pub struct UpdateScorecardData<'a> {
    pub scorecard_id: Uuid,
    pub app_id: &'a str,
    pub metric_name: String,
    pub score: Decimal,
    pub max_score: Decimal,
    pub review_date: chrono::NaiveDate,
    pub reviewer: Option<String>,
    pub notes: Option<String>,
    pub metadata: Option<serde_json::Value>,
    pub updated_at: DateTime<Utc>,
}

pub async fn update_scorecard_row_tx(
    tx: &mut Transaction<'_, Postgres>,
    p: &UpdateScorecardData<'_>,
) -> Result<Scorecard, PartyError> {
    Ok(sqlx::query_as(
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
    .bind(&p.metric_name)
    .bind(p.score)
    .bind(p.max_score)
    .bind(p.review_date)
    .bind(&p.reviewer)
    .bind(&p.notes)
    .bind(&p.metadata)
    .bind(p.updated_at)
    .bind(p.scorecard_id)
    .bind(p.app_id)
    .fetch_one(&mut **tx)
    .await?)
}
