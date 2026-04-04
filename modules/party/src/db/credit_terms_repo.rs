//! Credit terms repository — all SQL for `party_credit_terms`.

use chrono::{DateTime, Utc};
use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

use crate::domain::credit_terms::{CreateCreditTermsRequest, CreditTerms, UpdateCreditTermsRequest};
use crate::domain::party::PartyError;

// ── Reads ─────────────────────────────────────────────────────────────────────

pub async fn list_credit_terms(
    pool: &PgPool,
    app_id: &str,
    party_id: Uuid,
) -> Result<Vec<CreditTerms>, PartyError> {
    Ok(sqlx::query_as(
        r#"
        SELECT id, party_id, app_id, payment_terms, credit_limit_cents,
               currency, effective_from, effective_to, notes, idempotency_key,
               metadata, created_at, updated_at
        FROM party_credit_terms
        WHERE party_id = $1 AND app_id = $2
        ORDER BY effective_from DESC
        "#,
    )
    .bind(party_id)
    .bind(app_id)
    .fetch_all(pool)
    .await?)
}

pub async fn get_credit_terms(
    pool: &PgPool,
    app_id: &str,
    credit_terms_id: Uuid,
) -> Result<Option<CreditTerms>, PartyError> {
    Ok(sqlx::query_as(
        r#"
        SELECT id, party_id, app_id, payment_terms, credit_limit_cents,
               currency, effective_from, effective_to, notes, idempotency_key,
               metadata, created_at, updated_at
        FROM party_credit_terms
        WHERE id = $1 AND app_id = $2
        "#,
    )
    .bind(credit_terms_id)
    .bind(app_id)
    .fetch_optional(pool)
    .await?)
}

pub async fn find_credit_terms_by_idempotency_key(
    pool: &PgPool,
    app_id: &str,
    idem_key: &str,
) -> Result<Option<CreditTerms>, PartyError> {
    Ok(sqlx::query_as(
        r#"
        SELECT id, party_id, app_id, payment_terms, credit_limit_cents,
               currency, effective_from, effective_to, notes, idempotency_key,
               metadata, created_at, updated_at
        FROM party_credit_terms
        WHERE app_id = $1 AND idempotency_key = $2
        "#,
    )
    .bind(app_id)
    .bind(idem_key)
    .fetch_optional(pool)
    .await?)
}

// ── Transaction helpers ───────────────────────────────────────────────────────

pub async fn insert_credit_terms_tx(
    tx: &mut Transaction<'_, Postgres>,
    ct_id: Uuid,
    party_id: Uuid,
    app_id: &str,
    req: &CreateCreditTermsRequest,
    currency: &str,
    now: DateTime<Utc>,
) -> Result<CreditTerms, PartyError> {
    Ok(sqlx::query_as(
        r#"
        INSERT INTO party_credit_terms (
            id, party_id, app_id, payment_terms, credit_limit_cents,
            currency, effective_from, effective_to, notes, idempotency_key,
            metadata, created_at, updated_at
        )
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $12)
        RETURNING id, party_id, app_id, payment_terms, credit_limit_cents,
                  currency, effective_from, effective_to, notes, idempotency_key,
                  metadata, created_at, updated_at
        "#,
    )
    .bind(ct_id)
    .bind(party_id)
    .bind(app_id)
    .bind(req.payment_terms.trim())
    .bind(req.credit_limit_cents)
    .bind(currency)
    .bind(req.effective_from)
    .bind(req.effective_to)
    .bind(&req.notes)
    .bind(&req.idempotency_key)
    .bind(&req.metadata)
    .bind(now)
    .fetch_one(&mut **tx)
    .await?)
}

pub async fn fetch_credit_terms_for_update_tx(
    tx: &mut Transaction<'_, Postgres>,
    app_id: &str,
    credit_terms_id: Uuid,
) -> Result<Option<CreditTerms>, PartyError> {
    Ok(sqlx::query_as(
        r#"
        SELECT id, party_id, app_id, payment_terms, credit_limit_cents,
               currency, effective_from, effective_to, notes, idempotency_key,
               metadata, created_at, updated_at
        FROM party_credit_terms
        WHERE id = $1 AND app_id = $2
        FOR UPDATE
        "#,
    )
    .bind(credit_terms_id)
    .bind(app_id)
    .fetch_optional(&mut **tx)
    .await?)
}

pub struct UpdateCreditTermsData<'a> {
    pub credit_terms_id: Uuid,
    pub app_id: &'a str,
    pub payment_terms: String,
    pub credit_limit_cents: Option<i64>,
    pub currency: String,
    pub effective_from: chrono::NaiveDate,
    pub effective_to: Option<chrono::NaiveDate>,
    pub notes: Option<String>,
    pub metadata: Option<serde_json::Value>,
    pub updated_at: DateTime<Utc>,
}

pub async fn update_credit_terms_row_tx(
    tx: &mut Transaction<'_, Postgres>,
    p: &UpdateCreditTermsData<'_>,
) -> Result<CreditTerms, PartyError> {
    Ok(sqlx::query_as(
        r#"
        UPDATE party_credit_terms
        SET payment_terms = $1, credit_limit_cents = $2, currency = $3,
            effective_from = $4, effective_to = $5, notes = $6,
            metadata = $7, updated_at = $8
        WHERE id = $9 AND app_id = $10
        RETURNING id, party_id, app_id, payment_terms, credit_limit_cents,
                  currency, effective_from, effective_to, notes, idempotency_key,
                  metadata, created_at, updated_at
        "#,
    )
    .bind(&p.payment_terms)
    .bind(p.credit_limit_cents)
    .bind(&p.currency)
    .bind(p.effective_from)
    .bind(p.effective_to)
    .bind(&p.notes)
    .bind(&p.metadata)
    .bind(p.updated_at)
    .bind(p.credit_terms_id)
    .bind(p.app_id)
    .fetch_one(&mut **tx)
    .await?)
}
