use crate::domain::address::Address;
use crate::domain::contact::Contact;
use crate::events::{
    build_party_deactivated_envelope, build_party_updated_envelope, build_tags_updated_envelope,
    PartyDeactivatedPayload, PartyUpdatedPayload, TagsUpdatedPayload, EVENT_TYPE_PARTY_DEACTIVATED,
    EVENT_TYPE_PARTY_UPDATED, EVENT_TYPE_TAGS_UPDATED,
};
use crate::outbox::enqueue_event_tx;
use chrono::Utc;
use sqlx::PgPool;
use tokio::try_join;
use uuid::Uuid;

use super::validation;
use crate::domain::party::models::{
    ExternalRef, Party, PartyCompany, PartyError, PartyIndividual, PartyView, UpdatePartyRequest,
};

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

    let existing: Option<Party> = sqlx::query_as(
        r#"
        SELECT id, app_id, party_type::TEXT AS party_type, status::TEXT AS status,
               display_name, email, phone, website,
               address_line1, address_line2, city, state, postal_code, country,
               metadata, tags, created_at, updated_at
        FROM party_parties
        WHERE id = $1 AND app_id = $2
        FOR UPDATE
        "#,
    )
    .bind(party_id)
    .bind(app_id)
    .fetch_optional(&mut *tx)
    .await?;

    let current = existing.ok_or(PartyError::NotFound(party_id))?;

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

    let updated: Party = sqlx::query_as(
        r#"
        UPDATE party_parties
        SET display_name = $1, email = $2, phone = $3, website = $4,
            address_line1 = $5, address_line2 = $6, city = $7, state = $8,
            postal_code = $9, country = $10, metadata = $11, tags = $12,
            updated_at = $13
        WHERE id = $14 AND app_id = $15
        RETURNING id, app_id, party_type::TEXT AS party_type, status::TEXT AS status,
                  display_name, email, phone, website,
                  address_line1, address_line2, city, state, postal_code, country,
                  metadata, tags, created_at, updated_at
        "#,
    )
    .bind(&new_name)
    .bind(new_email)
    .bind(new_phone)
    .bind(new_website)
    .bind(new_addr1)
    .bind(new_addr2)
    .bind(new_city)
    .bind(new_state)
    .bind(new_postal)
    .bind(new_country)
    .bind(new_metadata)
    .bind(&new_tags)
    .bind(now)
    .bind(party_id)
    .bind(app_id)
    .fetch_one(&mut *tx)
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

    let (company, individual, external_refs, contacts, addresses) = try_join!(
        sqlx::query_as::<_, PartyCompany>(
            r#"
            SELECT party_id, legal_name, trade_name, registration_number, tax_id,
                   country_of_incorporation, industry_code, founded_date, employee_count,
                   annual_revenue_cents, currency, metadata, created_at, updated_at
            FROM party_companies WHERE party_id = $1
            "#,
        )
        .bind(party_id)
        .fetch_optional(pool),
        sqlx::query_as::<_, PartyIndividual>(
            r#"
            SELECT party_id, first_name, last_name, middle_name, date_of_birth, tax_id,
                   nationality, job_title, department, metadata, created_at, updated_at
            FROM party_individuals WHERE party_id = $1
            "#,
        )
        .bind(party_id)
        .fetch_optional(pool),
        sqlx::query_as::<_, ExternalRef>(
            r#"
            SELECT id, party_id, app_id, system, external_id, label, metadata, created_at, updated_at
            FROM party_external_refs
            WHERE party_id = $1 AND app_id = $2
            ORDER BY system, external_id
            "#,
        )
        .bind(party_id)
        .bind(app_id)
        .fetch_all(pool),
        sqlx::query_as::<_, Contact>(
            r#"
            SELECT id, party_id, app_id, first_name, last_name, email, phone,
                   role, is_primary, metadata, created_at, updated_at, deactivated_at
            FROM party_contacts
            WHERE party_id = $1 AND app_id = $2 AND deactivated_at IS NULL
            ORDER BY is_primary DESC, last_name ASC, first_name ASC
            "#,
        )
        .bind(party_id)
        .bind(app_id)
        .fetch_all(pool),
        sqlx::query_as::<_, Address>(
            r#"
            SELECT id, party_id, app_id, address_type::TEXT AS address_type,
                   label, line1, line2, city, state, postal_code, country,
                   is_primary, metadata, created_at, updated_at
            FROM party_addresses
            WHERE party_id = $1 AND app_id = $2
            ORDER BY is_primary DESC, address_type ASC
            "#,
        )
        .bind(party_id)
        .bind(app_id)
        .fetch_all(pool),
    )?;

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

    let exists: Option<(String,)> =
        sqlx::query_as("SELECT status::TEXT FROM party_parties WHERE id = $1 AND app_id = $2")
            .bind(party_id)
            .bind(app_id)
            .fetch_optional(&mut *tx)
            .await?;

    if exists.is_none() {
        return Err(PartyError::NotFound(party_id));
    }

    sqlx::query(
        "UPDATE party_parties SET status = 'inactive', updated_at = $1 WHERE id = $2 AND app_id = $3",
    )
    .bind(now)
    .bind(party_id)
    .bind(app_id)
    .execute(&mut *tx)
    .await?;

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
