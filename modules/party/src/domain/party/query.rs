use sqlx::PgPool;
use tokio::try_join;
use uuid::Uuid;

use crate::domain::party::models::{
    ExternalRef, Party, PartyCompany, PartyError, PartyIndividual, PartyView, SearchQuery,
};

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
        sqlx::query_as::<_, crate::domain::contact::Contact>(
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
        sqlx::query_as::<_, crate::domain::address::Address>(
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
) -> Result<Vec<Party>, PartyError> {
    let parties = if include_inactive {
        sqlx::query_as(
            r#"
            SELECT id, app_id, party_type::TEXT AS party_type, status::TEXT AS status,
                   display_name, email, phone, website,
                   address_line1, address_line2, city, state, postal_code, country,
                   metadata, tags, created_at, updated_at
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
                   metadata, tags, created_at, updated_at
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

pub async fn search_parties(
    pool: &PgPool,
    app_id: &str,
    query: &SearchQuery,
) -> Result<Vec<Party>, PartyError> {
    let limit = query.limit.unwrap_or(50).clamp(1, 200);
    let offset = query.offset.unwrap_or(0);
    let status = query.status.as_deref().unwrap_or("active");

    if query.external_system.is_some() || query.external_id.is_some() {
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

        return Ok(parties);
    }

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

    Ok(parties)
}
