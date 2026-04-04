use chrono::Utc;
use sqlx::PgPool;
use uuid::Uuid;

use crate::db::party_repo;
use crate::domain::party::models::{
    CreateCompanyRequest, CreateIndividualRequest, PartyError, PartyView,
};
use crate::{
    events::{build_party_created_envelope, PartyCreatedPayload, EVENT_TYPE_PARTY_CREATED},
    outbox::enqueue_event_tx,
};

pub async fn create_company(
    pool: &PgPool,
    app_id: &str,
    req: &CreateCompanyRequest,
    correlation_id: String,
) -> Result<PartyView, PartyError> {
    req.validate()?;

    let party_id = Uuid::new_v4();
    let event_id = Uuid::new_v4();
    let now = Utc::now();

    let mut tx = pool.begin().await?;

    let party = party_repo::insert_party_tx(&mut tx, party_id, app_id, "company", req, now).await?;
    let company = party_repo::insert_company_tx(&mut tx, party_id, req, now).await?;

    let payload = PartyCreatedPayload {
        party_id,
        app_id: app_id.to_string(),
        party_type: "company".to_string(),
        display_name: party.display_name.clone(),
        email: party.email.clone(),
        created_at: party.created_at,
    };

    let envelope =
        build_party_created_envelope(event_id, app_id.to_string(), correlation_id, None, payload);

    enqueue_event_tx(
        &mut tx,
        event_id,
        EVENT_TYPE_PARTY_CREATED,
        "party",
        &party_id.to_string(),
        app_id,
        &envelope,
    )
    .await?;

    tx.commit().await?;

    Ok(PartyView {
        party,
        company: Some(company),
        individual: None,
        external_refs: vec![],
        contacts: vec![],
        addresses: vec![],
    })
}

pub async fn create_individual(
    pool: &PgPool,
    app_id: &str,
    req: &CreateIndividualRequest,
    correlation_id: String,
) -> Result<PartyView, PartyError> {
    req.validate()?;

    let party_id = Uuid::new_v4();
    let event_id = Uuid::new_v4();
    let now = Utc::now();

    let mut tx = pool.begin().await?;

    let party =
        party_repo::insert_party_individual_tx(&mut tx, party_id, app_id, req, now).await?;
    let individual = party_repo::insert_individual_tx(&mut tx, party_id, req, now).await?;

    let payload = PartyCreatedPayload {
        party_id,
        app_id: app_id.to_string(),
        party_type: "individual".to_string(),
        display_name: party.display_name.clone(),
        email: party.email.clone(),
        created_at: party.created_at,
    };

    let envelope =
        build_party_created_envelope(event_id, app_id.to_string(), correlation_id, None, payload);

    enqueue_event_tx(
        &mut tx,
        event_id,
        EVENT_TYPE_PARTY_CREATED,
        "party",
        &party_id.to_string(),
        app_id,
        &envelope,
    )
    .await?;

    tx.commit().await?;

    Ok(PartyView {
        party,
        company: None,
        individual: Some(individual),
        external_refs: vec![],
        contacts: vec![],
        addresses: vec![],
    })
}
