use chrono::Utc;
use sqlx::PgPool;
use uuid::Uuid;

use crate::domain::party::models::{
    CreateCompanyRequest, CreateIndividualRequest, Party, PartyCompany, PartyError,
    PartyIndividual, PartyView,
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

    let party: Party = sqlx::query_as(
        r#"
        INSERT INTO party_parties (
            id, app_id, party_type, status, display_name, email, phone, website,
            address_line1, address_line2, city, state, postal_code, country,
            metadata, created_at, updated_at
        )
        VALUES ($1, $2, 'company', 'active', $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $14)
        RETURNING id, app_id, party_type::TEXT AS party_type, status::TEXT AS status,
                  display_name, email, phone, website,
                  address_line1, address_line2, city, state, postal_code, country,
                  metadata, tags, created_at, updated_at
        "#,
    )
    .bind(party_id)
    .bind(app_id)
    .bind(req.display_name.trim())
    .bind(&req.email)
    .bind(&req.phone)
    .bind(&req.website)
    .bind(&req.address_line1)
    .bind(&req.address_line2)
    .bind(&req.city)
    .bind(&req.state)
    .bind(&req.postal_code)
    .bind(&req.country)
    .bind(&req.metadata)
    .bind(now)
    .fetch_one(&mut *tx)
    .await?;

    let company: PartyCompany = sqlx::query_as(
        r#"
        INSERT INTO party_companies (
            party_id, legal_name, trade_name, registration_number, tax_id,
            country_of_incorporation, industry_code, founded_date, employee_count,
            annual_revenue_cents, currency, metadata, created_at, updated_at
        )
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $13)
        RETURNING party_id, legal_name, trade_name, registration_number, tax_id,
                  country_of_incorporation, industry_code, founded_date, employee_count,
                  annual_revenue_cents, currency, metadata, created_at, updated_at
        "#,
    )
    .bind(party_id)
    .bind(req.legal_name.trim())
    .bind(&req.trade_name)
    .bind(&req.registration_number)
    .bind(&req.tax_id)
    .bind(&req.country_of_incorporation)
    .bind(&req.industry_code)
    .bind(req.founded_date)
    .bind(req.employee_count)
    .bind(req.annual_revenue_cents)
    .bind(req.currency.as_deref().unwrap_or("usd"))
    .bind(&req.metadata)
    .bind(now)
    .fetch_one(&mut *tx)
    .await?;

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

    let party: Party = sqlx::query_as(
        r#"
        INSERT INTO party_parties (
            id, app_id, party_type, status, display_name, email, phone,
            address_line1, address_line2, city, state, postal_code, country,
            metadata, created_at, updated_at
        )
        VALUES ($1, $2, 'individual', 'active', $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $13)
        RETURNING id, app_id, party_type::TEXT AS party_type, status::TEXT AS status,
                  display_name, email, phone, website,
                  address_line1, address_line2, city, state, postal_code, country,
                  metadata, tags, created_at, updated_at
        "#,
    )
    .bind(party_id)
    .bind(app_id)
    .bind(req.display_name.trim())
    .bind(&req.email)
    .bind(&req.phone)
    .bind(&req.address_line1)
    .bind(&req.address_line2)
    .bind(&req.city)
    .bind(&req.state)
    .bind(&req.postal_code)
    .bind(&req.country)
    .bind(&req.metadata)
    .bind(now)
    .fetch_one(&mut *tx)
    .await?;

    let individual: PartyIndividual = sqlx::query_as(
        r#"
        INSERT INTO party_individuals (
            party_id, first_name, last_name, middle_name, date_of_birth, tax_id,
            nationality, job_title, department, metadata, created_at, updated_at
        )
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $11)
        RETURNING party_id, first_name, last_name, middle_name, date_of_birth, tax_id,
                  nationality, job_title, department, metadata, created_at, updated_at
        "#,
    )
    .bind(party_id)
    .bind(req.first_name.trim())
    .bind(req.last_name.trim())
    .bind(&req.middle_name)
    .bind(req.date_of_birth)
    .bind(&req.tax_id)
    .bind(&req.nationality)
    .bind(&req.job_title)
    .bind(&req.department)
    .bind(&req.metadata)
    .bind(now)
    .fetch_one(&mut *tx)
    .await?;

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
