//! Credit terms service — Guard→Mutation→Outbox atomicity.

use chrono::Utc;
use sqlx::PgPool;
use uuid::Uuid;

use crate::events::{
    build_credit_terms_created_envelope, build_credit_terms_updated_envelope, CreditTermsPayload,
    EVENT_TYPE_CREDIT_TERMS_CREATED, EVENT_TYPE_CREDIT_TERMS_UPDATED,
};
use crate::outbox::enqueue_event_tx;

use super::credit_terms::{CreateCreditTermsRequest, CreditTerms, UpdateCreditTermsRequest};
use super::party::PartyError;

// ============================================================================
// Reads
// ============================================================================

/// List credit terms for a party, scoped to app_id. Most recent first.
pub async fn list_credit_terms(
    pool: &PgPool,
    app_id: &str,
    party_id: Uuid,
) -> Result<Vec<CreditTerms>, PartyError> {
    let rows: Vec<CreditTerms> = sqlx::query_as(
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
    .await?;

    Ok(rows)
}

/// Get a single credit terms record by ID, scoped to app_id.
pub async fn get_credit_terms(
    pool: &PgPool,
    app_id: &str,
    credit_terms_id: Uuid,
) -> Result<Option<CreditTerms>, PartyError> {
    let row: Option<CreditTerms> = sqlx::query_as(
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
    .await?;

    Ok(row)
}

// ============================================================================
// Writes
// ============================================================================

/// Create credit terms for a party. Idempotent via idempotency_key.
/// Emits `party.credit_terms.created` via the outbox.
pub async fn create_credit_terms(
    pool: &PgPool,
    app_id: &str,
    party_id: Uuid,
    req: &CreateCreditTermsRequest,
    correlation_id: String,
) -> Result<CreditTerms, PartyError> {
    req.validate()?;

    // Idempotency check
    if let Some(ref idem_key) = req.idempotency_key {
        let existing: Option<CreditTerms> = sqlx::query_as(
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
        .await?;

        if let Some(existing) = existing {
            return Ok(existing);
        }
    }

    let ct_id = Uuid::new_v4();
    let event_id = Uuid::new_v4();
    let now = Utc::now();
    let currency = req.currency.as_deref().unwrap_or("USD");

    let mut tx = pool.begin().await?;

    // Guard: party must exist
    guard_party_exists_tx(&mut tx, app_id, party_id).await?;

    // Mutation
    let ct: CreditTerms = sqlx::query_as(
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
    .fetch_one(&mut *tx)
    .await?;

    // Outbox
    let payload = CreditTermsPayload {
        credit_terms_id: ct_id,
        party_id,
        app_id: app_id.to_string(),
        payment_terms: ct.payment_terms.clone(),
        credit_limit_cents: ct.credit_limit_cents,
        effective_from: ct.effective_from,
    };

    let envelope = build_credit_terms_created_envelope(
        event_id,
        app_id.to_string(),
        correlation_id,
        None,
        payload,
    );

    enqueue_event_tx(
        &mut tx,
        event_id,
        EVENT_TYPE_CREDIT_TERMS_CREATED,
        "credit_terms",
        &ct_id.to_string(),
        app_id,
        &envelope,
    )
    .await?;

    tx.commit().await?;
    Ok(ct)
}

/// Update credit terms. Emits `party.credit_terms.updated`.
pub async fn update_credit_terms(
    pool: &PgPool,
    app_id: &str,
    credit_terms_id: Uuid,
    req: &UpdateCreditTermsRequest,
    correlation_id: String,
) -> Result<CreditTerms, PartyError> {
    req.validate()?;

    let event_id = Uuid::new_v4();
    let now = Utc::now();

    let mut tx = pool.begin().await?;

    // Guard
    let existing: Option<CreditTerms> = sqlx::query_as(
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
    .fetch_optional(&mut *tx)
    .await?;

    let current = existing.ok_or(PartyError::NotFound(credit_terms_id))?;

    let new_terms = req
        .payment_terms
        .as_deref()
        .map(|t| t.trim().to_string())
        .unwrap_or(current.payment_terms);
    let new_limit = req.credit_limit_cents.or(current.credit_limit_cents);
    let new_currency = req
        .currency
        .clone()
        .unwrap_or(current.currency);
    let new_from = req.effective_from.unwrap_or(current.effective_from);
    let new_to = if req.effective_to.is_some() {
        req.effective_to
    } else {
        current.effective_to
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
    let updated: CreditTerms = sqlx::query_as(
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
    .bind(&new_terms)
    .bind(new_limit)
    .bind(&new_currency)
    .bind(new_from)
    .bind(new_to)
    .bind(&new_notes)
    .bind(&new_metadata)
    .bind(now)
    .bind(credit_terms_id)
    .bind(app_id)
    .fetch_one(&mut *tx)
    .await?;

    // Outbox
    let payload = CreditTermsPayload {
        credit_terms_id,
        party_id: updated.party_id,
        app_id: app_id.to_string(),
        payment_terms: updated.payment_terms.clone(),
        credit_limit_cents: updated.credit_limit_cents,
        effective_from: updated.effective_from,
    };

    let envelope = build_credit_terms_updated_envelope(
        event_id,
        app_id.to_string(),
        correlation_id,
        None,
        payload,
    );

    enqueue_event_tx(
        &mut tx,
        event_id,
        EVENT_TYPE_CREDIT_TERMS_UPDATED,
        "credit_terms",
        &credit_terms_id.to_string(),
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
