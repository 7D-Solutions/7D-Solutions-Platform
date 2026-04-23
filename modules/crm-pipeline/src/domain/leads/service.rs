//! Lead service — business logic layer.
//!
//! Owns state machine enforcement, numbering, and outbox event emission.
//! All state mutations use a transaction to atomically update rows + enqueue events.

use chrono::Utc;
use platform_client_party::{CreateCompanyRequest, PartiesClient};
use platform_sdk::PlatformClient;
use sqlx::PgPool;
use uuid::Uuid;

use super::{
    repo, ConvertLeadRequest, ConvertLeadResponse, CreateLeadRequest, DisqualifyLeadRequest, Lead,
    LeadError, LeadStatus, ListLeadsQuery, UpdateLeadRequest,
};
use crate::events::{
    build_lead_converted_envelope, build_lead_created_envelope, build_lead_status_changed_envelope,
    LeadConvertedPayload, LeadCreatedPayload, LeadStatusChangedPayload, EVENT_TYPE_LEAD_CONVERTED,
    EVENT_TYPE_LEAD_CREATED, EVENT_TYPE_LEAD_STATUS_CHANGED,
};
use crate::outbox;

pub async fn create_lead(
    pool: &PgPool,
    tenant_id: &str,
    req: &CreateLeadRequest,
    created_by: String,
) -> Result<Lead, LeadError> {
    if req.company_name.trim().is_empty() {
        return Err(LeadError::Validation("company_name is required".into()));
    }
    if req.source.trim().is_empty() {
        return Err(LeadError::Validation("source is required".into()));
    }

    let mut tx = pool.begin().await?;
    let lead_number = repo::next_lead_number(&mut *tx, tenant_id).await?;

    let lead = Lead {
        id: Uuid::new_v4(),
        tenant_id: tenant_id.to_string(),
        lead_number,
        source: req.source.clone(),
        source_detail: req.source_detail.clone(),
        company_name: req.company_name.clone(),
        contact_name: req.contact_name.clone(),
        contact_email: req.contact_email.clone(),
        contact_phone: req.contact_phone.clone(),
        contact_title: req.contact_title.clone(),
        party_id: None,
        party_contact_id: None,
        status: LeadStatus::New.as_str().to_string(),
        disqualify_reason: None,
        estimated_value_cents: req.estimated_value_cents,
        currency: req.currency.clone().unwrap_or_else(|| "USD".to_string()),
        converted_opportunity_id: None,
        converted_at: None,
        owner_id: req.owner_id.clone(),
        notes: req.notes.clone(),
        created_by,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };

    let created = repo::insert_lead(&mut *tx, &lead).await?;

    let payload = LeadCreatedPayload {
        lead_id: created.id,
        tenant_id: tenant_id.to_string(),
        lead_number: created.lead_number.clone(),
        source: created.source.clone(),
        company_name: created.company_name.clone(),
        estimated_value_cents: created.estimated_value_cents,
        created_at: created.created_at,
    };
    let envelope = build_lead_created_envelope(
        Uuid::new_v4(),
        tenant_id.to_string(),
        Uuid::new_v4().to_string(),
        payload,
    );
    outbox::enqueue_event_tx(
        &mut tx,
        Uuid::new_v4(),
        EVENT_TYPE_LEAD_CREATED,
        "lead",
        &created.id.to_string(),
        &envelope,
    )
    .await?;

    tx.commit().await?;
    Ok(created)
}

pub async fn get_lead(pool: &PgPool, tenant_id: &str, id: Uuid) -> Result<Lead, LeadError> {
    repo::fetch_lead(pool, tenant_id, id)
        .await?
        .ok_or(LeadError::NotFound(id))
}

pub async fn list_leads(
    pool: &PgPool,
    tenant_id: &str,
    query: &ListLeadsQuery,
) -> Result<Vec<Lead>, LeadError> {
    repo::list_leads(
        pool,
        tenant_id,
        query.status.as_deref(),
        query.owner_id.as_deref(),
        query.include_terminal.unwrap_or(false),
    )
    .await
}

pub async fn update_lead(
    pool: &PgPool,
    tenant_id: &str,
    id: Uuid,
    req: &UpdateLeadRequest,
) -> Result<Lead, LeadError> {
    let lead = get_lead(pool, tenant_id, id).await?;
    let status = LeadStatus::from_str(&lead.status)
        .ok_or_else(|| LeadError::Validation(format!("Unknown status: {}", lead.status)))?;
    if status.is_terminal() {
        return Err(LeadError::TerminalState(lead.status.clone()));
    }
    repo::update_lead_fields(pool, tenant_id, id, req).await
}

/// Advance lead to "contacted".
pub async fn mark_contacted(
    pool: &PgPool,
    tenant_id: &str,
    id: Uuid,
    actor: String,
) -> Result<Lead, LeadError> {
    transition(
        pool,
        tenant_id,
        id,
        LeadStatus::Contacted,
        None,
        None,
        None,
        None,
        actor,
    )
    .await
}

/// Advance lead to "qualifying".
pub async fn mark_qualifying(
    pool: &PgPool,
    tenant_id: &str,
    id: Uuid,
    actor: String,
) -> Result<Lead, LeadError> {
    transition(
        pool,
        tenant_id,
        id,
        LeadStatus::Qualifying,
        None,
        None,
        None,
        None,
        actor,
    )
    .await
}

/// Advance lead to "qualified".
pub async fn mark_qualified(
    pool: &PgPool,
    tenant_id: &str,
    id: Uuid,
    actor: String,
) -> Result<Lead, LeadError> {
    transition(
        pool,
        tenant_id,
        id,
        LeadStatus::Qualified,
        None,
        None,
        None,
        None,
        actor,
    )
    .await
}

/// Convert a qualified lead.
///
/// When `party_id` is absent from the request (and not pre-set on the lead),
/// `parties_client` is used to auto-create a Party company from `lead.company_name`.
/// Pass `None` to keep the previous behaviour (error if no party_id).
pub async fn convert_lead(
    pool: &PgPool,
    tenant_id: &str,
    id: Uuid,
    req: &ConvertLeadRequest,
    parties_client: Option<&PartiesClient>,
) -> Result<ConvertLeadResponse, LeadError> {
    let lead = get_lead(pool, tenant_id, id).await?;
    let current = LeadStatus::from_str(&lead.status)
        .ok_or_else(|| LeadError::Validation(format!("Unknown status: {}", lead.status)))?;

    if !current.can_transition_to(&LeadStatus::Converted) {
        return Err(LeadError::InvalidTransition(
            lead.status.clone(),
            "converted".to_string(),
        ));
    }

    let effective_party_id = match req.party_id.or(lead.party_id) {
        Some(pid) => pid,
        None => match parties_client {
            Some(client) => {
                let claims = PlatformClient::service_claims_from_str(tenant_id)
                    .map_err(|e| LeadError::PartyApiError(format!("invalid tenant_id: {e}")))?;
                let body = CreateCompanyRequest {
                    display_name: lead.company_name.clone(),
                    legal_name: lead.company_name.clone(),
                    ..Default::default()
                };
                let party_view = client
                    .create_company(&claims, &body)
                    .await
                    .map_err(|e| LeadError::PartyApiError(e.to_string()))?;
                party_view.party.id
            }
            None => return Err(LeadError::ConversionRequiresParty),
        },
    };

    let mut tx = pool.begin().await?;
    let updated = repo::update_lead_status(
        &mut *tx,
        tenant_id,
        id,
        LeadStatus::Converted.as_str(),
        Some(effective_party_id),
        req.party_contact_id.or(lead.party_contact_id),
        None,
        None,
    )
    .await?;

    let payload = LeadConvertedPayload {
        lead_id: updated.id,
        tenant_id: tenant_id.to_string(),
        opportunity_id: updated.converted_opportunity_id,
        party_id: effective_party_id,
        party_contact_id: updated.party_contact_id,
        converted_at: Utc::now(),
    };
    let envelope = build_lead_converted_envelope(
        Uuid::new_v4(),
        tenant_id.to_string(),
        Uuid::new_v4().to_string(),
        payload,
    );
    outbox::enqueue_event_tx(
        &mut tx,
        Uuid::new_v4(),
        EVENT_TYPE_LEAD_CONVERTED,
        "lead",
        &updated.id.to_string(),
        &envelope,
    )
    .await?;

    tx.commit().await?;
    Ok(ConvertLeadResponse {
        opportunity_id: updated.converted_opportunity_id,
        lead: updated,
    })
}

/// Disqualify a lead. Requires a reason.
pub async fn disqualify_lead(
    pool: &PgPool,
    tenant_id: &str,
    id: Uuid,
    req: &DisqualifyLeadRequest,
) -> Result<Lead, LeadError> {
    if req.reason.trim().is_empty() {
        return Err(LeadError::DisqualifyRequiresReason);
    }
    let lead = get_lead(pool, tenant_id, id).await?;
    let current = LeadStatus::from_str(&lead.status)
        .ok_or_else(|| LeadError::Validation(format!("Unknown status: {}", lead.status)))?;
    if current.is_terminal() {
        return Err(LeadError::TerminalState(lead.status.clone()));
    }

    let mut tx = pool.begin().await?;
    let updated = repo::update_lead_status(
        &mut *tx,
        tenant_id,
        id,
        LeadStatus::Disqualified.as_str(),
        None,
        None,
        None,
        Some(&req.reason),
    )
    .await?;

    let payload = LeadStatusChangedPayload {
        lead_id: updated.id,
        tenant_id: tenant_id.to_string(),
        lead_number: updated.lead_number.clone(),
        from_status: lead.status.clone(),
        to_status: LeadStatus::Disqualified.as_str().to_string(),
        changed_by: "system".to_string(),
        changed_at: Utc::now(),
    };
    let envelope = build_lead_status_changed_envelope(
        Uuid::new_v4(),
        tenant_id.to_string(),
        Uuid::new_v4().to_string(),
        payload,
    );
    outbox::enqueue_event_tx(
        &mut tx,
        Uuid::new_v4(),
        EVENT_TYPE_LEAD_STATUS_CHANGED,
        "lead",
        &updated.id.to_string(),
        &envelope,
    )
    .await?;

    tx.commit().await?;
    Ok(updated)
}

/// Mark a lead as dead (no further activity expected).
pub async fn mark_dead(pool: &PgPool, tenant_id: &str, id: Uuid) -> Result<Lead, LeadError> {
    let lead = get_lead(pool, tenant_id, id).await?;
    let current = LeadStatus::from_str(&lead.status)
        .ok_or_else(|| LeadError::Validation(format!("Unknown status: {}", lead.status)))?;
    if current.is_terminal() {
        return Err(LeadError::TerminalState(lead.status.clone()));
    }

    let mut tx = pool.begin().await?;
    let updated = repo::update_lead_status(
        &mut *tx,
        tenant_id,
        id,
        LeadStatus::Dead.as_str(),
        None,
        None,
        None,
        None,
    )
    .await?;

    let payload = LeadStatusChangedPayload {
        lead_id: updated.id,
        tenant_id: tenant_id.to_string(),
        lead_number: updated.lead_number.clone(),
        from_status: lead.status.clone(),
        to_status: LeadStatus::Dead.as_str().to_string(),
        changed_by: "system".to_string(),
        changed_at: Utc::now(),
    };
    let envelope = build_lead_status_changed_envelope(
        Uuid::new_v4(),
        tenant_id.to_string(),
        Uuid::new_v4().to_string(),
        payload,
    );
    outbox::enqueue_event_tx(
        &mut tx,
        Uuid::new_v4(),
        EVENT_TYPE_LEAD_STATUS_CHANGED,
        "lead",
        &updated.id.to_string(),
        &envelope,
    )
    .await?;

    tx.commit().await?;
    Ok(updated)
}

// ============================================================================
// Internal helpers
// ============================================================================

async fn transition(
    pool: &PgPool,
    tenant_id: &str,
    id: Uuid,
    target: LeadStatus,
    party_id: Option<Uuid>,
    party_contact_id: Option<Uuid>,
    converted_opp_id: Option<Uuid>,
    disqualify_reason: Option<&str>,
    actor: String,
) -> Result<Lead, LeadError> {
    let lead = get_lead(pool, tenant_id, id).await?;
    let current = LeadStatus::from_str(&lead.status)
        .ok_or_else(|| LeadError::Validation(format!("Unknown status: {}", lead.status)))?;

    if !current.can_transition_to(&target) {
        return Err(LeadError::InvalidTransition(
            lead.status.clone(),
            target.as_str().to_string(),
        ));
    }

    let mut tx = pool.begin().await?;
    let updated = repo::update_lead_status(
        &mut *tx,
        tenant_id,
        id,
        target.as_str(),
        party_id,
        party_contact_id,
        converted_opp_id,
        disqualify_reason,
    )
    .await?;

    let payload = LeadStatusChangedPayload {
        lead_id: updated.id,
        tenant_id: tenant_id.to_string(),
        lead_number: updated.lead_number.clone(),
        from_status: lead.status.clone(),
        to_status: target.as_str().to_string(),
        changed_by: actor,
        changed_at: Utc::now(),
    };
    let envelope = build_lead_status_changed_envelope(
        Uuid::new_v4(),
        tenant_id.to_string(),
        Uuid::new_v4().to_string(),
        payload,
    );
    outbox::enqueue_event_tx(
        &mut tx,
        Uuid::new_v4(),
        EVENT_TYPE_LEAD_STATUS_CHANGED,
        "lead",
        &updated.id.to_string(),
        &envelope,
    )
    .await?;

    tx.commit().await?;
    Ok(updated)
}
