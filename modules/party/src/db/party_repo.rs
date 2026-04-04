//! Party repository — all SQL for `party_parties`, `party_companies`,
//! `party_individuals`, and `party_external_refs`.
//!
//! Every function takes `&PgPool` or `&mut Transaction`. No business logic here.

use chrono::{DateTime, Utc};
use sqlx::{PgPool, Postgres, Transaction};
use tokio::try_join;
use uuid::Uuid;

use crate::domain::address::Address;
use crate::domain::contact::Contact;
use crate::domain::party::models::{
    CreateCompanyRequest, CreateIndividualRequest, ExternalRef, Party, PartyCompany, PartyError,
    PartyIndividual, PartyView, SearchQuery,
};

// ── Guard helpers ─────────────────────────────────────────────────────────────

pub async fn guard_party_exists(
    pool: &PgPool,
    app_id: &str,
    party_id: Uuid,
) -> Result<(), PartyError> {
    let exists: Option<(Uuid,)> =
        sqlx::query_as("SELECT id FROM party_parties WHERE id = $1 AND app_id = $2")
            .bind(party_id)
            .bind(app_id)
            .fetch_optional(pool)
            .await?;
    if exists.is_none() {
        return Err(PartyError::NotFound(party_id));
    }
    Ok(())
}

pub async fn guard_party_exists_tx(
    tx: &mut Transaction<'_, Postgres>,
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

// ── Reads ─────────────────────────────────────────────────────────────────────

/// Fetch all related sub-tables for a party in parallel.
pub async fn fetch_party_relations(
    pool: &PgPool,
    app_id: &str,
    party_id: Uuid,
) -> Result<
    (
        Option<PartyCompany>,
        Option<PartyIndividual>,
        Vec<ExternalRef>,
        Vec<Contact>,
        Vec<Address>,
    ),
    PartyError,
> {
    Ok(try_join!(
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
    )?)
}

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
               metadata, tags, created_at, updated_at
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

    let (company, individual, external_refs, contacts, addresses) =
        fetch_party_relations(pool, app_id, party_id).await?;

    Ok(Some(PartyView {
        party,
        company,
        individual,
        external_refs,
        contacts,
        addresses,
    }))
}

pub async fn list_parties(
    pool: &PgPool,
    app_id: &str,
    include_inactive: bool,
    page: i64,
    page_size: i64,
) -> Result<(Vec<Party>, i64), PartyError> {
    let page_size = page_size.clamp(1, 200);
    let offset = (page - 1).max(0) * page_size;

    let (parties, total): (Vec<Party>, i64) = if include_inactive {
        let total: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM party_parties WHERE app_id = $1")
                .bind(app_id)
                .fetch_one(pool)
                .await?;

        let rows = sqlx::query_as(
            r#"
            SELECT id, app_id, party_type::TEXT AS party_type, status::TEXT AS status,
                   display_name, email, phone, website,
                   address_line1, address_line2, city, state, postal_code, country,
                   metadata, tags, created_at, updated_at
            FROM party_parties
            WHERE app_id = $1
            ORDER BY display_name ASC
            LIMIT $2 OFFSET $3
            "#,
        )
        .bind(app_id)
        .bind(page_size)
        .bind(offset)
        .fetch_all(pool)
        .await?;

        (rows, total)
    } else {
        let total: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM party_parties WHERE app_id = $1 AND status = 'active'",
        )
        .bind(app_id)
        .fetch_one(pool)
        .await?;

        let rows = sqlx::query_as(
            r#"
            SELECT id, app_id, party_type::TEXT AS party_type, status::TEXT AS status,
                   display_name, email, phone, website,
                   address_line1, address_line2, city, state, postal_code, country,
                   metadata, tags, created_at, updated_at
            FROM party_parties
            WHERE app_id = $1 AND status = 'active'
            ORDER BY display_name ASC
            LIMIT $2 OFFSET $3
            "#,
        )
        .bind(app_id)
        .bind(page_size)
        .bind(offset)
        .fetch_all(pool)
        .await?;

        (rows, total)
    };

    Ok((parties, total))
}

pub async fn search_parties(
    pool: &PgPool,
    app_id: &str,
    query: &SearchQuery,
) -> Result<(Vec<Party>, i64), PartyError> {
    let limit = query.limit.unwrap_or(50).clamp(1, 200);
    let offset = query.offset.unwrap_or(0);
    let status = query.status.as_deref().unwrap_or("active");

    if query.external_system.is_some() || query.external_id.is_some() {
        let total: i64 = sqlx::query_scalar(
            r#"
            SELECT COUNT(DISTINCT p.id)
            FROM party_parties p
            JOIN party_external_refs r ON r.party_id = p.id AND r.app_id = p.app_id
            WHERE p.app_id = $1
              AND ($2::TEXT IS NULL OR p.status::TEXT = $2)
              AND ($3::TEXT IS NULL OR p.party_type::TEXT = $3)
              AND ($4::TEXT IS NULL OR p.display_name ILIKE '%' || $4 || '%')
              AND ($5::TEXT IS NULL OR r.system = $5)
              AND ($6::TEXT IS NULL OR r.external_id = $6)
            "#,
        )
        .bind(app_id)
        .bind(status)
        .bind(query.party_type.as_deref())
        .bind(query.name.as_deref())
        .bind(query.external_system.as_deref())
        .bind(query.external_id.as_deref())
        .fetch_one(pool)
        .await?;

        let parties: Vec<Party> = sqlx::query_as(
            r#"
            SELECT DISTINCT p.id, p.app_id,
                   p.party_type::TEXT AS party_type, p.status::TEXT AS status,
                   p.display_name, p.email, p.phone, p.website,
                   p.address_line1, p.address_line2, p.city, p.state, p.postal_code, p.country,
                   p.metadata, p.tags, p.created_at, p.updated_at
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

        return Ok((parties, total));
    }

    let total: i64 = sqlx::query_scalar(
        r#"
        SELECT COUNT(*)
        FROM party_parties
        WHERE app_id = $1
          AND ($2::TEXT IS NULL OR status::TEXT = $2)
          AND ($3::TEXT IS NULL OR party_type::TEXT = $3)
          AND ($4::TEXT IS NULL OR display_name ILIKE '%' || $4 || '%')
        "#,
    )
    .bind(app_id)
    .bind(status)
    .bind(query.party_type.as_deref())
    .bind(query.name.as_deref())
    .fetch_one(pool)
    .await?;

    let parties: Vec<Party> = sqlx::query_as(
        r#"
        SELECT id, app_id, party_type::TEXT AS party_type, status::TEXT AS status,
               display_name, email, phone, website,
               address_line1, address_line2, city, state, postal_code, country,
               metadata, tags, created_at, updated_at
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

    Ok((parties, total))
}

// ── Transaction helpers — update ──────────────────────────────────────────────

pub async fn fetch_party_for_update_tx(
    tx: &mut Transaction<'_, Postgres>,
    app_id: &str,
    party_id: Uuid,
) -> Result<Option<Party>, PartyError> {
    Ok(sqlx::query_as(
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
    .fetch_optional(&mut **tx)
    .await?)
}

pub struct UpdatePartyData<'a> {
    pub party_id: Uuid,
    pub app_id: &'a str,
    pub display_name: &'a str,
    pub email: Option<&'a String>,
    pub phone: Option<&'a String>,
    pub website: Option<&'a String>,
    pub address_line1: Option<&'a String>,
    pub address_line2: Option<&'a String>,
    pub city: Option<&'a String>,
    pub state: Option<&'a String>,
    pub postal_code: Option<&'a String>,
    pub country: Option<&'a String>,
    pub metadata: Option<&'a serde_json::Value>,
    pub tags: &'a [String],
    pub updated_at: DateTime<Utc>,
}

pub async fn update_party_row_tx(
    tx: &mut Transaction<'_, Postgres>,
    p: &UpdatePartyData<'_>,
) -> Result<Party, PartyError> {
    Ok(sqlx::query_as(
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
    .bind(p.display_name)
    .bind(p.email)
    .bind(p.phone)
    .bind(p.website)
    .bind(p.address_line1)
    .bind(p.address_line2)
    .bind(p.city)
    .bind(p.state)
    .bind(p.postal_code)
    .bind(p.country)
    .bind(p.metadata)
    .bind(p.tags)
    .bind(p.updated_at)
    .bind(p.party_id)
    .bind(p.app_id)
    .fetch_one(&mut **tx)
    .await?)
}

pub async fn fetch_party_status_for_update_tx(
    tx: &mut Transaction<'_, Postgres>,
    app_id: &str,
    party_id: Uuid,
) -> Result<Option<(String,)>, PartyError> {
    Ok(
        sqlx::query_as("SELECT status::TEXT FROM party_parties WHERE id = $1 AND app_id = $2")
            .bind(party_id)
            .bind(app_id)
            .fetch_optional(&mut **tx)
            .await?,
    )
}

pub async fn set_party_status_tx(
    tx: &mut Transaction<'_, Postgres>,
    app_id: &str,
    party_id: Uuid,
    status: &str,
    updated_at: DateTime<Utc>,
) -> Result<(), PartyError> {
    sqlx::query(
        "UPDATE party_parties SET status = $1, updated_at = $2 WHERE id = $3 AND app_id = $4",
    )
    .bind(status)
    .bind(updated_at)
    .bind(party_id)
    .bind(app_id)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

// ── Transaction helpers — create ──────────────────────────────────────────────

pub async fn insert_party_tx(
    tx: &mut Transaction<'_, Postgres>,
    party_id: Uuid,
    app_id: &str,
    party_type: &str,
    req: &CreateCompanyRequest,
    now: DateTime<Utc>,
) -> Result<Party, PartyError> {
    Ok(sqlx::query_as(
        r#"
        INSERT INTO party_parties (
            id, app_id, party_type, status, display_name, email, phone, website,
            address_line1, address_line2, city, state, postal_code, country,
            metadata, created_at, updated_at
        )
        VALUES ($1, $2, $3, 'active', $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $15)
        RETURNING id, app_id, party_type::TEXT AS party_type, status::TEXT AS status,
                  display_name, email, phone, website,
                  address_line1, address_line2, city, state, postal_code, country,
                  metadata, tags, created_at, updated_at
        "#,
    )
    .bind(party_id)
    .bind(app_id)
    .bind(party_type)
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
    .fetch_one(&mut **tx)
    .await?)
}

pub async fn insert_party_individual_tx(
    tx: &mut Transaction<'_, Postgres>,
    party_id: Uuid,
    app_id: &str,
    req: &CreateIndividualRequest,
    now: DateTime<Utc>,
) -> Result<Party, PartyError> {
    Ok(sqlx::query_as(
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
    .fetch_one(&mut **tx)
    .await?)
}

pub async fn insert_company_tx(
    tx: &mut Transaction<'_, Postgres>,
    party_id: Uuid,
    req: &CreateCompanyRequest,
    now: DateTime<Utc>,
) -> Result<PartyCompany, PartyError> {
    Ok(sqlx::query_as(
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
    .fetch_one(&mut **tx)
    .await?)
}

pub async fn insert_individual_tx(
    tx: &mut Transaction<'_, Postgres>,
    party_id: Uuid,
    req: &CreateIndividualRequest,
    now: DateTime<Utc>,
) -> Result<PartyIndividual, PartyError> {
    Ok(sqlx::query_as(
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
    .fetch_one(&mut **tx)
    .await?)
}
