//! Party CRUD service — Guard→Mutation→Outbox atomicity.
//!
//! Operations:
//! - create_company: create base party + company extension + outbox event
//! - create_individual: create base party + individual extension + outbox event
//! - get_party: fetch party with extension and external refs
//! - list_parties: list parties for app_id
//! - update_party: update base party fields + outbox event
//! - deactivate_party: soft-delete + outbox event
//! - search_parties: name/type/status/external_ref search

use chrono::Utc;
use sqlx::PgPool;
use uuid::Uuid;

use crate::events::{
    build_party_created_envelope, build_party_deactivated_envelope,
    build_party_updated_envelope, PartyCreatedPayload, PartyDeactivatedPayload,
    PartyUpdatedPayload, EVENT_TYPE_PARTY_CREATED, EVENT_TYPE_PARTY_DEACTIVATED,
    EVENT_TYPE_PARTY_UPDATED,
};
use crate::outbox::enqueue_event_tx;

use super::models::{
    CreateCompanyRequest, CreateIndividualRequest, ExternalRef, Party, PartyCompany,
    PartyError, PartyIndividual, PartyView, SearchQuery, UpdatePartyRequest,
};

// ============================================================================
// Reads
// ============================================================================

/// Fetch a single party with extension and external refs, scoped to app_id.
pub async fn get_party(
    pool: &PgPool,
    app_id: &str,
    party_id: Uuid,
) -> Result<Option<PartyView>, PartyError> {
    let party: Option<Party> = sqlx::query_as(
        r#"
        SELECT id, app_id, party_type::TEXT AS party_type, status::TEXT AS status,
               display_name, email, phone, website,
               address_line1, address_line2, city, state, postal_code, country,
               metadata, created_at, updated_at
        FROM party_parties
        WHERE id = $1 AND app_id = $2
        "#,
    )
    .bind(party_id)
    .bind(app_id)
    .fetch_optional(pool)
    .await?;

    let party = match party {
        Some(p) => p,
        None => return Ok(None),
    };

    let company: Option<PartyCompany> = sqlx::query_as(
        r#"
        SELECT party_id, legal_name, trade_name, registration_number, tax_id,
               country_of_incorporation, industry_code, founded_date, employee_count,
               annual_revenue_cents, currency, metadata, created_at, updated_at
        FROM party_companies WHERE party_id = $1
        "#,
    )
    .bind(party_id)
    .fetch_optional(pool)
    .await?;

    let individual: Option<PartyIndividual> = sqlx::query_as(
        r#"
        SELECT party_id, first_name, last_name, middle_name, date_of_birth, tax_id,
               nationality, job_title, department, metadata, created_at, updated_at
        FROM party_individuals WHERE party_id = $1
        "#,
    )
    .bind(party_id)
    .fetch_optional(pool)
    .await?;

    let external_refs: Vec<ExternalRef> = sqlx::query_as(
        r#"
        SELECT id, party_id, app_id, system, external_id, label, metadata, created_at, updated_at
        FROM party_external_refs
        WHERE party_id = $1 AND app_id = $2
        ORDER BY system, external_id
        "#,
    )
    .bind(party_id)
    .bind(app_id)
    .fetch_all(pool)
    .await?;

    let contacts = crate::domain::contact_service::list_contacts(pool, app_id, party_id)
        .await
        .unwrap_or_default();

    let addresses = crate::domain::address_service::list_addresses(pool, app_id, party_id)
        .await
        .unwrap_or_default();

    Ok(Some(PartyView {
        party,
        company,
        individual,
        external_refs,
        contacts,
        addresses,
    }))
}

/// List parties for an app_id (base records only, no extension detail).
pub async fn list_parties(
    pool: &PgPool,
    app_id: &str,
    include_inactive: bool,
) -> Result<Vec<Party>, PartyError> {
    let parties = if include_inactive {
        sqlx::query_as(
            r#"
            SELECT id, app_id, party_type::TEXT AS party_type, status::TEXT AS status,
                   display_name, email, phone, website,
                   address_line1, address_line2, city, state, postal_code, country,
                   metadata, created_at, updated_at
            FROM party_parties
            WHERE app_id = $1
            ORDER BY display_name ASC
            "#,
        )
        .bind(app_id)
        .fetch_all(pool)
        .await?
    } else {
        sqlx::query_as(
            r#"
            SELECT id, app_id, party_type::TEXT AS party_type, status::TEXT AS status,
                   display_name, email, phone, website,
                   address_line1, address_line2, city, state, postal_code, country,
                   metadata, created_at, updated_at
            FROM party_parties
            WHERE app_id = $1 AND status = 'active'
            ORDER BY display_name ASC
            "#,
        )
        .bind(app_id)
        .fetch_all(pool)
        .await?
    };

    Ok(parties)
}

/// Search parties by name, type, status, and/or external reference.
pub async fn search_parties(
    pool: &PgPool,
    app_id: &str,
    query: &SearchQuery,
) -> Result<Vec<Party>, PartyError> {
    let limit = query.limit.unwrap_or(50).min(200).max(1);
    let offset = query.offset.unwrap_or(0).max(0);
    let status = query.status.as_deref().unwrap_or("active");

    // When filtering by external ref, join and then fetch base party
    if query.external_system.is_some() || query.external_id.is_some() {
        let parties: Vec<Party> = sqlx::query_as(
            r#"
            SELECT DISTINCT p.id, p.app_id,
                   p.party_type::TEXT AS party_type, p.status::TEXT AS status,
                   p.display_name, p.email, p.phone, p.website,
                   p.address_line1, p.address_line2, p.city, p.state, p.postal_code, p.country,
                   p.metadata, p.created_at, p.updated_at
            FROM party_parties p
            JOIN party_external_refs r ON r.party_id = p.id AND r.app_id = p.app_id
            WHERE p.app_id = $1
              AND ($2::TEXT IS NULL OR p.status::TEXT = $2)
              AND ($3::TEXT IS NULL OR p.party_type::TEXT = $3)
              AND ($4::TEXT IS NULL OR p.display_name ILIKE '%' || $4 || '%')
              AND ($5::TEXT IS NULL OR r.system = $5)
              AND ($6::TEXT IS NULL OR r.external_id = $6)
            ORDER BY p.display_name ASC
            LIMIT $7 OFFSET $8
            "#,
        )
        .bind(app_id)
        .bind(status)
        .bind(query.party_type.as_deref())
        .bind(query.name.as_deref())
        .bind(query.external_system.as_deref())
        .bind(query.external_id.as_deref())
        .bind(limit)
        .bind(offset)
        .fetch_all(pool)
        .await?;

        return Ok(parties);
    }

    // Name/type/status only search
    let parties: Vec<Party> = sqlx::query_as(
        r#"
        SELECT id, app_id, party_type::TEXT AS party_type, status::TEXT AS status,
               display_name, email, phone, website,
               address_line1, address_line2, city, state, postal_code, country,
               metadata, created_at, updated_at
        FROM party_parties
        WHERE app_id = $1
          AND ($2::TEXT IS NULL OR status::TEXT = $2)
          AND ($3::TEXT IS NULL OR party_type::TEXT = $3)
          AND ($4::TEXT IS NULL OR display_name ILIKE '%' || $4 || '%')
        ORDER BY display_name ASC
        LIMIT $5 OFFSET $6
        "#,
    )
    .bind(app_id)
    .bind(status)
    .bind(query.party_type.as_deref())
    .bind(query.name.as_deref())
    .bind(limit)
    .bind(offset)
    .fetch_all(pool)
    .await?;

    Ok(parties)
}

// ============================================================================
// Writes: Company
// ============================================================================

/// Create a company party. Emits `party.created` via the outbox.
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

    // Mutation: insert base party
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
                  metadata, created_at, updated_at
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

    // Mutation: insert company extension
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

    // Outbox: emit party.created
    let payload = PartyCreatedPayload {
        party_id,
        app_id: app_id.to_string(),
        party_type: "company".to_string(),
        display_name: party.display_name.clone(),
        email: party.email.clone(),
        created_at: party.created_at,
    };

    let envelope = build_party_created_envelope(
        event_id,
        app_id.to_string(),
        correlation_id,
        None,
        payload,
    );

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

// ============================================================================
// Writes: Individual
// ============================================================================

/// Create an individual party. Emits `party.created` via the outbox.
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
                  metadata, created_at, updated_at
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

    let envelope = build_party_created_envelope(
        event_id,
        app_id.to_string(),
        correlation_id,
        None,
        payload,
    );

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

// ============================================================================
// Writes: Update
// ============================================================================

/// Update base party fields. Emits `party.updated` via the outbox.
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
    let actor = req.updated_by.clone().unwrap_or_else(|| "system".to_string());

    let mut tx = pool.begin().await?;

    // Guard: party must exist for this app
    let existing: Option<Party> = sqlx::query_as(
        r#"
        SELECT id, app_id, party_type::TEXT AS party_type, status::TEXT AS status,
               display_name, email, phone, website,
               address_line1, address_line2, city, state, postal_code, country,
               metadata, created_at, updated_at
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

    // Resolve updated values
    let new_name = req
        .display_name
        .as_deref()
        .map(|n| n.trim().to_string())
        .unwrap_or_else(|| current.display_name.clone());
    let new_email = if req.email.is_some() { req.email.clone() } else { current.email.clone() };
    let new_phone = if req.phone.is_some() { req.phone.clone() } else { current.phone.clone() };
    let new_website =
        if req.website.is_some() { req.website.clone() } else { current.website.clone() };
    let new_addr1 = if req.address_line1.is_some() {
        req.address_line1.clone()
    } else {
        current.address_line1.clone()
    };
    let new_addr2 = if req.address_line2.is_some() {
        req.address_line2.clone()
    } else {
        current.address_line2.clone()
    };
    let new_city = if req.city.is_some() { req.city.clone() } else { current.city.clone() };
    let new_state = if req.state.is_some() { req.state.clone() } else { current.state.clone() };
    let new_postal =
        if req.postal_code.is_some() { req.postal_code.clone() } else { current.postal_code.clone() };
    let new_country =
        if req.country.is_some() { req.country.clone() } else { current.country.clone() };
    let new_metadata =
        if req.metadata.is_some() { req.metadata.clone() } else { current.metadata.clone() };

    // Mutation
    let _updated: Party = sqlx::query_as(
        r#"
        UPDATE party_parties
        SET display_name = $1, email = $2, phone = $3, website = $4,
            address_line1 = $5, address_line2 = $6, city = $7, state = $8,
            postal_code = $9, country = $10, metadata = $11, updated_at = $12
        WHERE id = $13 AND app_id = $14
        RETURNING id, app_id, party_type::TEXT AS party_type, status::TEXT AS status,
                  display_name, email, phone, website,
                  address_line1, address_line2, city, state, postal_code, country,
                  metadata, created_at, updated_at
        "#,
    )
    .bind(&new_name)
    .bind(&new_email)
    .bind(&new_phone)
    .bind(&new_website)
    .bind(&new_addr1)
    .bind(&new_addr2)
    .bind(&new_city)
    .bind(&new_state)
    .bind(&new_postal)
    .bind(&new_country)
    .bind(&new_metadata)
    .bind(now)
    .bind(party_id)
    .bind(app_id)
    .fetch_one(&mut *tx)
    .await?;

    // Outbox: party.updated
    let payload = PartyUpdatedPayload {
        party_id,
        app_id: app_id.to_string(),
        display_name: req.display_name.clone(),
        email: req.email.clone(),
        updated_by: actor,
        updated_at: now,
    };

    let envelope = build_party_updated_envelope(
        event_id,
        app_id.to_string(),
        correlation_id,
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

    tx.commit().await?;

    // Fetch full view (extension + refs) after commit
    let view = get_party(pool, app_id, party_id).await?.ok_or(PartyError::NotFound(party_id))?;
    Ok(view)
}

// ============================================================================
// Writes: Deactivate
// ============================================================================

/// Deactivate a party (soft delete). Emits `party.deactivated` via the outbox.
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

    // Guard
    let exists: Option<(String,)> =
        sqlx::query_as("SELECT status::TEXT FROM party_parties WHERE id = $1 AND app_id = $2")
            .bind(party_id)
            .bind(app_id)
            .fetch_optional(&mut *tx)
            .await?;

    if exists.is_none() {
        return Err(PartyError::NotFound(party_id));
    }

    // Mutation
    sqlx::query(
        "UPDATE party_parties SET status = 'inactive', updated_at = $1 WHERE id = $2 AND app_id = $3",
    )
    .bind(now)
    .bind(party_id)
    .bind(app_id)
    .execute(&mut *tx)
    .await?;

    // Outbox: party.deactivated
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

// ============================================================================
// Integrated Tests (real DB)
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;

    const TEST_APP: &str = "test-party-crud";

    fn test_db_url() -> String {
        std::env::var("DATABASE_URL").unwrap_or_else(|_| {
            "postgres://party_user:party_pass@localhost:5448/party_db".to_string()
        })
    }

    async fn test_pool() -> PgPool {
        let pool = sqlx::PgPool::connect(&test_db_url())
            .await
            .expect("Failed to connect to party test database");
        sqlx::migrate!("./db/migrations")
            .run(&pool)
            .await
            .expect("Migrations failed");
        pool
    }

    async fn cleanup(pool: &PgPool) {
        sqlx::query(
            "DELETE FROM party_outbox WHERE app_id = $1"
        )
        .bind(TEST_APP)
        .execute(pool)
        .await
        .ok();
        sqlx::query(
            "DELETE FROM party_parties WHERE app_id = $1"
        )
        .bind(TEST_APP)
        .execute(pool)
        .await
        .ok();
    }

    fn sample_company_req(name: &str) -> CreateCompanyRequest {
        CreateCompanyRequest {
            display_name: name.to_string(),
            legal_name: format!("{} LLC", name),
            trade_name: None,
            registration_number: Some("REG-001".to_string()),
            tax_id: Some("12-3456789".to_string()),
            country_of_incorporation: Some("US".to_string()),
            industry_code: None,
            founded_date: None,
            employee_count: Some(50),
            annual_revenue_cents: None,
            currency: Some("usd".to_string()),
            email: Some("info@example.com".to_string()),
            phone: None,
            website: None,
            address_line1: None,
            address_line2: None,
            city: None,
            state: None,
            postal_code: None,
            country: Some("US".to_string()),
            metadata: None,
        }
    }

    fn sample_individual_req(first: &str, last: &str) -> CreateIndividualRequest {
        CreateIndividualRequest {
            display_name: format!("{} {}", first, last),
            first_name: first.to_string(),
            last_name: last.to_string(),
            middle_name: None,
            date_of_birth: None,
            tax_id: None,
            nationality: Some("US".to_string()),
            job_title: Some("Engineer".to_string()),
            department: None,
            email: Some(format!("{}.{}@example.com", first.to_lowercase(), last.to_lowercase())),
            phone: None,
            address_line1: None,
            address_line2: None,
            city: None,
            state: None,
            postal_code: None,
            country: None,
            metadata: None,
        }
    }

    #[tokio::test]
    #[serial]
    async fn test_create_and_get_company() {
        let pool = test_pool().await;
        cleanup(&pool).await;

        let view = create_company(&pool, TEST_APP, &sample_company_req("Acme Corp"), "corr-1".to_string())
            .await
            .expect("create_company failed");

        assert_eq!(view.party.display_name, "Acme Corp");
        assert_eq!(view.party.party_type, "company");
        assert_eq!(view.party.status, "active");
        assert!(view.company.is_some());
        assert_eq!(view.company.unwrap().legal_name, "Acme Corp LLC");

        let fetched = get_party(&pool, TEST_APP, view.party.id)
            .await
            .expect("get_party failed");
        assert!(fetched.is_some());
        assert_eq!(fetched.unwrap().party.id, view.party.id);

        cleanup(&pool).await;
    }

    #[tokio::test]
    #[serial]
    async fn test_create_and_get_individual() {
        let pool = test_pool().await;
        cleanup(&pool).await;

        let view = create_individual(
            &pool, TEST_APP, &sample_individual_req("Alice", "Smith"), "corr-1".to_string(),
        )
        .await
        .expect("create_individual failed");

        assert_eq!(view.party.party_type, "individual");
        let ind = view.individual.expect("individual extension missing");
        assert_eq!(ind.first_name, "Alice");
        assert_eq!(ind.last_name, "Smith");

        cleanup(&pool).await;
    }

    #[tokio::test]
    #[serial]
    async fn test_update_party() {
        let pool = test_pool().await;
        cleanup(&pool).await;

        let created = create_company(
            &pool, TEST_APP, &sample_company_req("Beta Corp"), "corr-1".to_string(),
        )
        .await
        .expect("create failed");

        let req = UpdatePartyRequest {
            display_name: Some("Beta Corporation".to_string()),
            email: Some("new@beta.com".to_string()),
            phone: None,
            website: None,
            address_line1: None,
            address_line2: None,
            city: None,
            state: None,
            postal_code: None,
            country: None,
            metadata: None,
            updated_by: Some("user-42".to_string()),
        };

        let updated = update_party(&pool, TEST_APP, created.party.id, &req, "corr-2".to_string())
            .await
            .expect("update failed");

        assert_eq!(updated.party.display_name, "Beta Corporation");
        assert_eq!(updated.party.email.as_deref(), Some("new@beta.com"));

        cleanup(&pool).await;
    }

    #[tokio::test]
    #[serial]
    async fn test_deactivate_party() {
        let pool = test_pool().await;
        cleanup(&pool).await;

        let created = create_company(
            &pool, TEST_APP, &sample_company_req("Gamma Inc"), "corr-1".to_string(),
        )
        .await
        .expect("create failed");

        deactivate_party(&pool, TEST_APP, created.party.id, "user-1", "corr-3".to_string())
            .await
            .expect("deactivate failed");

        // Active list excludes it
        let active = list_parties(&pool, TEST_APP, false).await.expect("list failed");
        assert!(active.iter().all(|p| p.id != created.party.id));

        // Full list includes it as inactive
        let all = list_parties(&pool, TEST_APP, true).await.expect("list all failed");
        let found = all.iter().find(|p| p.id == created.party.id);
        assert!(found.is_some());
        assert_eq!(found.unwrap().status, "inactive");

        cleanup(&pool).await;
    }

    #[tokio::test]
    #[serial]
    async fn test_search_by_name() {
        let pool = test_pool().await;
        cleanup(&pool).await;

        create_company(&pool, TEST_APP, &sample_company_req("Delta Supplies"), "c1".to_string())
            .await
            .expect("create 1 failed");
        create_company(&pool, TEST_APP, &sample_company_req("Delta Analytics"), "c2".to_string())
            .await
            .expect("create 2 failed");
        create_company(&pool, TEST_APP, &sample_company_req("Epsilon LLC"), "c3".to_string())
            .await
            .expect("create 3 failed");

        let results = search_parties(
            &pool,
            TEST_APP,
            &SearchQuery {
                name: Some("Delta".to_string()),
                party_type: None,
                status: None,
                external_system: None,
                external_id: None,
                limit: None,
                offset: None,
            },
        )
        .await
        .expect("search failed");

        assert_eq!(results.len(), 2, "expected 2 Delta parties, got {}", results.len());
        assert!(results.iter().all(|p| p.display_name.contains("Delta")));

        cleanup(&pool).await;
    }

    #[tokio::test]
    #[serial]
    async fn test_party_events_in_outbox() {
        let pool = test_pool().await;
        cleanup(&pool).await;

        let created = create_company(
            &pool, TEST_APP, &sample_company_req("Zeta Corp"), "corr-outbox".to_string(),
        )
        .await
        .expect("create failed");

        let count: (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM party_outbox WHERE aggregate_type = 'party' AND aggregate_id = $1",
        )
        .bind(created.party.id.to_string())
        .fetch_one(&pool)
        .await
        .expect("outbox query failed");

        assert!(count.0 >= 1, "expected >=1 outbox event");

        cleanup(&pool).await;
    }

    #[tokio::test]
    #[serial]
    async fn test_get_party_wrong_app_returns_none() {
        let pool = test_pool().await;
        cleanup(&pool).await;

        let created = create_company(
            &pool, TEST_APP, &sample_company_req("Eta Corp"), "corr-1".to_string(),
        )
        .await
        .expect("create failed");

        let result = get_party(&pool, "other-app", created.party.id)
            .await
            .expect("get_party failed");
        assert!(result.is_none());

        cleanup(&pool).await;
    }
}
