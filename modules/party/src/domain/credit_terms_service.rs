//! Credit terms service — Guard→Mutation→Outbox atomicity.

use chrono::Utc;
use sqlx::PgPool;
use uuid::Uuid;

use crate::db::{credit_terms_repo, credit_terms_repo::UpdateCreditTermsData, party_repo};
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
    credit_terms_repo::list_credit_terms(pool, app_id, party_id).await
}

/// Get a single credit terms record by ID, scoped to app_id.
pub async fn get_credit_terms(
    pool: &PgPool,
    app_id: &str,
    credit_terms_id: Uuid,
) -> Result<Option<CreditTerms>, PartyError> {
    credit_terms_repo::get_credit_terms(pool, app_id, credit_terms_id).await
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
        if let Some(existing) =
            credit_terms_repo::find_credit_terms_by_idempotency_key(pool, app_id, idem_key).await?
        {
            return Ok(existing);
        }
    }

    let ct_id = Uuid::new_v4();
    let event_id = Uuid::new_v4();
    let now = Utc::now();
    let currency = req.currency.as_deref().unwrap_or("USD");

    let mut tx = pool.begin().await?;

    party_repo::guard_party_exists_tx(&mut tx, app_id, party_id).await?;

    let ct = credit_terms_repo::insert_credit_terms_tx(
        &mut tx, ct_id, party_id, app_id, req, currency, now,
    )
    .await?;

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

    let current =
        credit_terms_repo::fetch_credit_terms_for_update_tx(&mut tx, app_id, credit_terms_id)
            .await?
            .ok_or(PartyError::NotFound(credit_terms_id))?;

    let new_terms = req
        .payment_terms
        .as_deref()
        .map(|t| t.trim().to_string())
        .unwrap_or(current.payment_terms);
    let new_limit = req.credit_limit_cents.or(current.credit_limit_cents);
    let new_currency = req.currency.clone().unwrap_or(current.currency);
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

    let updated = credit_terms_repo::update_credit_terms_row_tx(
        &mut tx,
        &UpdateCreditTermsData {
            credit_terms_id,
            app_id,
            payment_terms: new_terms,
            credit_limit_cents: new_limit,
            currency: new_currency,
            effective_from: new_from,
            effective_to: new_to,
            notes: new_notes,
            metadata: new_metadata,
            updated_at: now,
        },
    )
    .await?;

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
