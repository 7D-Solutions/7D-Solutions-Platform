use crate::db::party_repo::{self, UpdatePartyData};
use crate::domain::party::models::{PartyError, PartyView, UpdatePartyRequest};
use crate::events::{
    build_party_deactivated_envelope, build_party_reactivated_envelope,
    build_party_updated_envelope, build_tags_updated_envelope, PartyDeactivatedPayload,
    PartyReactivatedPayload, PartyUpdatedPayload, TagsUpdatedPayload, EVENT_TYPE_PARTY_DEACTIVATED,
    EVENT_TYPE_PARTY_REACTIVATED, EVENT_TYPE_PARTY_UPDATED, EVENT_TYPE_TAGS_UPDATED,
};
use crate::outbox::enqueue_event_tx;
use chrono::Utc;
use sqlx::PgPool;
use uuid::Uuid;

use super::validation;

pub async fn update_party(
    pool: &PgPool,
    app_id: &str,
    party_id: Uuid,
    req: &UpdatePartyRequest,
    correlation_id: String,
) -> Result<PartyView, PartyError> {
    req.validate()?;

    let event_id = Uuid::new_v4();
    let now = Utc::now();
    let actor = req.updated_by.as_deref().unwrap_or("system");

    let mut tx = pool.begin().await?;

    let current = party_repo::fetch_party_for_update_tx(&mut tx, app_id, party_id)
        .await?
        .ok_or(PartyError::NotFound(party_id))?;

    let new_name = req
        .display_name
        .as_deref()
        .map(|n| n.trim().to_string())
        .unwrap_or_else(|| current.display_name.clone());
    let new_email = req.email.as_ref().or(current.email.as_ref());
    let new_phone = req.phone.as_ref().or(current.phone.as_ref());
    let new_website = req.website.as_ref().or(current.website.as_ref());
    let new_addr1 = req
        .address_line1
        .as_ref()
        .or(current.address_line1.as_ref());
    let new_addr2 = req
        .address_line2
        .as_ref()
        .or(current.address_line2.as_ref());
    let new_city = req.city.as_ref().or(current.city.as_ref());
    let new_state = req.state.as_ref().or(current.state.as_ref());
    let new_postal = req.postal_code.as_ref().or(current.postal_code.as_ref());
    let new_country = req.country.as_ref().or(current.country.as_ref());
    let new_metadata = req.metadata.as_ref().or(current.metadata.as_ref());
    let new_tags = validation::normalized_tags(req.tags.clone(), &current.tags);

    let updated = party_repo::update_party_row_tx(
        &mut tx,
        &UpdatePartyData {
            party_id,
            app_id,
            display_name: &new_name,
            email: new_email,
            phone: new_phone,
            website: new_website,
            address_line1: new_addr1,
            address_line2: new_addr2,
            city: new_city,
            state: new_state,
            postal_code: new_postal,
            country: new_country,
            metadata: new_metadata,
            tags: &new_tags,
            updated_at: now,
        },
    )
    .await?;

    let payload = PartyUpdatedPayload {
        party_id,
        app_id: app_id.to_string(),
        display_name: req.display_name.clone(),
        email: req.email.clone(),
        updated_by: actor.to_string(),
        updated_at: now,
    };

    let envelope = build_party_updated_envelope(
        event_id,
        app_id.to_string(),
        correlation_id.clone(),
        None,
        payload,
    );

    enqueue_event_tx(
        &mut tx,
        event_id,
        EVENT_TYPE_PARTY_UPDATED,
        "party",
        &party_id.to_string(),
        app_id,
        &envelope,
    )
    .await?;

    if req.tags.is_some() && req.tags.as_ref() != Some(&current.tags) {
        let tags_event_id = Uuid::new_v4();
        let tags_payload = TagsUpdatedPayload {
            party_id,
            app_id: app_id.to_string(),
            tags: new_tags.clone(),
        };

        let tags_envelope = build_tags_updated_envelope(
            tags_event_id,
            app_id.to_string(),
            correlation_id,
            None,
            tags_payload,
        );

        enqueue_event_tx(
            &mut tx,
            tags_event_id,
            EVENT_TYPE_TAGS_UPDATED,
            "party",
            &party_id.to_string(),
            app_id,
            &tags_envelope,
        )
        .await?;
    }

    tx.commit().await?;

    let (company, individual, external_refs, contacts, addresses) =
        party_repo::fetch_party_relations(pool, app_id, party_id).await?;

    Ok(PartyView {
        party: updated,
        company,
        individual,
        external_refs,
        contacts,
        addresses,
    })
}

pub async fn deactivate_party(
    pool: &PgPool,
    app_id: &str,
    party_id: Uuid,
    actor: &str,
    correlation_id: String,
) -> Result<(), PartyError> {
    let event_id = Uuid::new_v4();
    let now = Utc::now();

    let mut tx = pool.begin().await?;

    if party_repo::fetch_party_status_for_update_tx(&mut tx, app_id, party_id)
        .await?
        .is_none()
    {
        return Err(PartyError::NotFound(party_id));
    }

    party_repo::set_party_status_tx(&mut tx, app_id, party_id, "inactive", now).await?;

    let payload = PartyDeactivatedPayload {
        party_id,
        app_id: app_id.to_string(),
        deactivated_by: actor.to_string(),
        deactivated_at: now,
    };

    let envelope = build_party_deactivated_envelope(
        event_id,
        app_id.to_string(),
        correlation_id,
        None,
        payload,
    );

    enqueue_event_tx(
        &mut tx,
        event_id,
        EVENT_TYPE_PARTY_DEACTIVATED,
        "party",
        &party_id.to_string(),
        app_id,
        &envelope,
    )
    .await?;

    tx.commit().await?;

    Ok(())
}

pub async fn reactivate_party(
    pool: &PgPool,
    app_id: &str,
    party_id: Uuid,
    actor: &str,
    correlation_id: String,
) -> Result<(), PartyError> {
    let event_id = Uuid::new_v4();
    let now = Utc::now();

    let mut tx = pool.begin().await?;

    if party_repo::fetch_party_status_for_update_tx(&mut tx, app_id, party_id)
        .await?
        .is_none()
    {
        return Err(PartyError::NotFound(party_id));
    }

    party_repo::set_party_status_tx(&mut tx, app_id, party_id, "active", now).await?;

    let payload = PartyReactivatedPayload {
        party_id,
        app_id: app_id.to_string(),
        reactivated_by: actor.to_string(),
        reactivated_at: now,
    };

    let envelope = build_party_reactivated_envelope(
        event_id,
        app_id.to_string(),
        correlation_id,
        None,
        payload,
    );

    enqueue_event_tx(
        &mut tx,
        event_id,
        EVENT_TYPE_PARTY_REACTIVATED,
        "party",
        &party_id.to_string(),
        app_id,
        &envelope,
    )
    .await?;

    tx.commit().await?;

    Ok(())
}
