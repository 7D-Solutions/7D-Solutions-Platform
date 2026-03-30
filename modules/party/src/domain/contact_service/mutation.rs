use chrono::Utc;
use sqlx::PgPool;
use uuid::Uuid;

use crate::events::{
    build_contact_created_envelope, build_contact_deactivated_envelope,
    build_contact_primary_set_envelope, build_contact_updated_envelope, ContactDeactivatedPayload,
    ContactPayload, ContactPrimarySetPayload, EVENT_TYPE_CONTACT_CREATED,
    EVENT_TYPE_CONTACT_DEACTIVATED, EVENT_TYPE_CONTACT_PRIMARY_SET, EVENT_TYPE_CONTACT_UPDATED,
};
use crate::outbox::enqueue_event_tx;

use crate::domain::contact::{Contact, CreateContactRequest, UpdateContactRequest};
use crate::domain::party::PartyError;

use super::guards::{clear_primary_for_role, guard_party_exists_tx};

pub async fn create_contact(
    pool: &PgPool,
    app_id: &str,
    party_id: Uuid,
    req: &CreateContactRequest,
    correlation_id: String,
) -> Result<Contact, PartyError> {
    req.validate()?;

    let contact_id = Uuid::new_v4();
    let event_id = Uuid::new_v4();
    let now = Utc::now();
    let is_primary = req.is_primary.unwrap_or(false);

    let mut tx = pool.begin().await?;

    guard_party_exists_tx(&mut tx, app_id, party_id).await?;

    if is_primary {
        clear_primary_for_role(&mut tx, app_id, party_id, req.role.as_deref()).await?;
    }

    let contact: Contact = sqlx::query_as(
        r#"
        INSERT INTO party_contacts (
            id, party_id, app_id, first_name, last_name, email, phone,
            role, is_primary, metadata, created_at, updated_at
        )
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $11)
        RETURNING id, party_id, app_id, first_name, last_name, email, phone,
                  role, is_primary, metadata, created_at, updated_at, deactivated_at
        "#,
    )
    .bind(contact_id)
    .bind(party_id)
    .bind(app_id)
    .bind(req.first_name.trim())
    .bind(req.last_name.trim())
    .bind(&req.email)
    .bind(&req.phone)
    .bind(&req.role)
    .bind(is_primary)
    .bind(&req.metadata)
    .bind(now)
    .fetch_one(&mut *tx)
    .await?;

    let payload = ContactPayload {
        contact_id,
        party_id,
        app_id: app_id.to_string(),
        first_name: contact.first_name.clone(),
        last_name: contact.last_name.clone(),
        email: contact.email.clone(),
        role: contact.role.clone(),
        is_primary: contact.is_primary,
    };

    let envelope =
        build_contact_created_envelope(event_id, app_id.to_string(), correlation_id, None, payload);

    enqueue_event_tx(
        &mut tx,
        event_id,
        EVENT_TYPE_CONTACT_CREATED,
        "contact",
        &contact_id.to_string(),
        app_id,
        &envelope,
    )
    .await?;

    tx.commit().await?;
    Ok(contact)
}

pub async fn update_contact(
    pool: &PgPool,
    app_id: &str,
    contact_id: Uuid,
    req: &UpdateContactRequest,
    correlation_id: String,
) -> Result<Contact, PartyError> {
    req.validate()?;

    let event_id = Uuid::new_v4();
    let now = Utc::now();
    let mut tx = pool.begin().await?;

    let existing: Option<Contact> = sqlx::query_as(
        r#"
        SELECT id, party_id, app_id, first_name, last_name, email, phone,
               role, is_primary, metadata, created_at, updated_at, deactivated_at
        FROM party_contacts
        WHERE id = $1 AND app_id = $2 AND deactivated_at IS NULL
        FOR UPDATE
        "#,
    )
    .bind(contact_id)
    .bind(app_id)
    .fetch_optional(&mut *tx)
    .await?;

    let current = existing.ok_or(PartyError::NotFound(contact_id))?;

    if req.is_primary == Some(true) && !current.is_primary {
        let role = req.role.as_deref().or(current.role.as_deref());
        clear_primary_for_role(&mut tx, app_id, current.party_id, role).await?;
    }

    let new_first = req
        .first_name
        .as_deref()
        .map(|n| n.trim().to_string())
        .unwrap_or(current.first_name);
    let new_last = req
        .last_name
        .as_deref()
        .map(|n| n.trim().to_string())
        .unwrap_or(current.last_name);
    let new_email = if req.email.is_some() {
        req.email.clone()
    } else {
        current.email
    };
    let new_phone = if req.phone.is_some() {
        req.phone.clone()
    } else {
        current.phone
    };
    let new_role = if req.role.is_some() {
        req.role.clone()
    } else {
        current.role
    };
    let new_primary = req.is_primary.unwrap_or(current.is_primary);
    let new_metadata = if req.metadata.is_some() {
        req.metadata.clone()
    } else {
        current.metadata
    };

    let updated: Contact = sqlx::query_as(
        r#"
        UPDATE party_contacts
        SET first_name = $1, last_name = $2, email = $3, phone = $4,
            role = $5, is_primary = $6, metadata = $7, updated_at = $8
        WHERE id = $9 AND app_id = $10
        RETURNING id, party_id, app_id, first_name, last_name, email, phone,
                  role, is_primary, metadata, created_at, updated_at, deactivated_at
        "#,
    )
    .bind(&new_first)
    .bind(&new_last)
    .bind(&new_email)
    .bind(&new_phone)
    .bind(&new_role)
    .bind(new_primary)
    .bind(&new_metadata)
    .bind(now)
    .bind(contact_id)
    .bind(app_id)
    .fetch_one(&mut *tx)
    .await?;

    let payload = ContactPayload {
        contact_id,
        party_id: updated.party_id,
        app_id: app_id.to_string(),
        first_name: updated.first_name.clone(),
        last_name: updated.last_name.clone(),
        email: updated.email.clone(),
        role: updated.role.clone(),
        is_primary: updated.is_primary,
    };

    let envelope =
        build_contact_updated_envelope(event_id, app_id.to_string(), correlation_id, None, payload);

    enqueue_event_tx(
        &mut tx,
        event_id,
        EVENT_TYPE_CONTACT_UPDATED,
        "contact",
        &contact_id.to_string(),
        app_id,
        &envelope,
    )
    .await?;

    tx.commit().await?;
    Ok(updated)
}

pub async fn deactivate_contact(
    pool: &PgPool,
    app_id: &str,
    contact_id: Uuid,
    correlation_id: String,
) -> Result<(), PartyError> {
    let event_id = Uuid::new_v4();
    let now = Utc::now();
    let mut tx = pool.begin().await?;

    let existing: Option<Contact> = sqlx::query_as(
        r#"
        SELECT id, party_id, app_id, first_name, last_name, email, phone,
               role, is_primary, metadata, created_at, updated_at, deactivated_at
        FROM party_contacts
        WHERE id = $1 AND app_id = $2 AND deactivated_at IS NULL
        FOR UPDATE
        "#,
    )
    .bind(contact_id)
    .bind(app_id)
    .fetch_optional(&mut *tx)
    .await?;

    let current = existing.ok_or(PartyError::NotFound(contact_id))?;

    sqlx::query(
        "UPDATE party_contacts SET deactivated_at = $1, updated_at = $1 WHERE id = $2 AND app_id = $3",
    )
    .bind(now)
    .bind(contact_id)
    .bind(app_id)
    .execute(&mut *tx)
    .await?;

    let payload = ContactDeactivatedPayload {
        contact_id,
        party_id: current.party_id,
        app_id: app_id.to_string(),
        deactivated_at: now,
    };

    let envelope = build_contact_deactivated_envelope(
        event_id,
        app_id.to_string(),
        correlation_id,
        None,
        payload,
    );

    enqueue_event_tx(
        &mut tx,
        event_id,
        EVENT_TYPE_CONTACT_DEACTIVATED,
        "contact",
        &contact_id.to_string(),
        app_id,
        &envelope,
    )
    .await?;

    tx.commit().await?;
    Ok(())
}

pub async fn set_primary_for_role(
    pool: &PgPool,
    app_id: &str,
    party_id: Uuid,
    contact_id: Uuid,
    role: &str,
    correlation_id: String,
) -> Result<Contact, PartyError> {
    let event_id = Uuid::new_v4();
    let now = Utc::now();
    let mut tx = pool.begin().await?;

    sqlx::query_as::<_, Contact>(
        r#"
        SELECT id, party_id, app_id, first_name, last_name, email, phone,
               role, is_primary, metadata, created_at, updated_at, deactivated_at
        FROM party_contacts
        WHERE id = $1 AND app_id = $2 AND party_id = $3 AND deactivated_at IS NULL
        FOR UPDATE
        "#,
    )
    .bind(contact_id)
    .bind(app_id)
    .bind(party_id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(PartyError::NotFound(contact_id))?;

    clear_primary_for_role(&mut tx, app_id, party_id, Some(role)).await?;

    let updated: Contact = sqlx::query_as(
        r#"
        UPDATE party_contacts
        SET is_primary = true, role = $1, updated_at = $2
        WHERE id = $3 AND app_id = $4
        RETURNING id, party_id, app_id, first_name, last_name, email, phone,
                  role, is_primary, metadata, created_at, updated_at, deactivated_at
        "#,
    )
    .bind(role)
    .bind(now)
    .bind(contact_id)
    .bind(app_id)
    .fetch_one(&mut *tx)
    .await?;

    let payload = ContactPrimarySetPayload {
        contact_id,
        party_id,
        app_id: app_id.to_string(),
        role: role.to_string(),
    };

    let envelope = build_contact_primary_set_envelope(
        event_id,
        app_id.to_string(),
        correlation_id,
        None,
        payload,
    );

    enqueue_event_tx(
        &mut tx,
        event_id,
        EVENT_TYPE_CONTACT_PRIMARY_SET,
        "contact",
        &contact_id.to_string(),
        app_id,
        &envelope,
    )
    .await?;

    tx.commit().await?;
    Ok(updated)
}
