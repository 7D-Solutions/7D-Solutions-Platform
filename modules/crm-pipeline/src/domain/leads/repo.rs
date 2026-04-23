//! Lead repository — SQL layer.

use chrono::Utc;
use sqlx::{PgConnection, PgPool};
use uuid::Uuid;

use super::{Lead, LeadError};

pub async fn insert_lead(conn: &mut PgConnection, lead: &Lead) -> Result<Lead, LeadError> {
    let row = sqlx::query_as::<_, Lead>(
        r#"
        INSERT INTO leads (
            id, tenant_id, lead_number, source, source_detail, company_name,
            contact_name, contact_email, contact_phone, contact_title,
            party_id, party_contact_id, status, disqualify_reason,
            estimated_value_cents, currency, converted_opportunity_id, converted_at,
            owner_id, notes, created_by, created_at, updated_at
        ) VALUES (
            $1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$18,$19,$20,$21,$22,$23
        )
        RETURNING *
        "#,
    )
    .bind(lead.id)
    .bind(&lead.tenant_id)
    .bind(&lead.lead_number)
    .bind(&lead.source)
    .bind(&lead.source_detail)
    .bind(&lead.company_name)
    .bind(&lead.contact_name)
    .bind(&lead.contact_email)
    .bind(&lead.contact_phone)
    .bind(&lead.contact_title)
    .bind(lead.party_id)
    .bind(lead.party_contact_id)
    .bind(&lead.status)
    .bind(&lead.disqualify_reason)
    .bind(lead.estimated_value_cents)
    .bind(&lead.currency)
    .bind(lead.converted_opportunity_id)
    .bind(lead.converted_at)
    .bind(&lead.owner_id)
    .bind(&lead.notes)
    .bind(&lead.created_by)
    .bind(lead.created_at)
    .bind(lead.updated_at)
    .fetch_one(conn)
    .await?;
    Ok(row)
}

pub async fn fetch_lead(
    pool: &PgPool,
    tenant_id: &str,
    id: Uuid,
) -> Result<Option<Lead>, LeadError> {
    let row = sqlx::query_as::<_, Lead>("SELECT * FROM leads WHERE id = $1 AND tenant_id = $2")
        .bind(id)
        .bind(tenant_id)
        .fetch_optional(pool)
        .await?;
    Ok(row)
}

pub async fn list_leads(
    pool: &PgPool,
    tenant_id: &str,
    status_filter: Option<&str>,
    owner_filter: Option<&str>,
    include_terminal: bool,
) -> Result<Vec<Lead>, LeadError> {
    // Build query dynamically
    let rows = if let Some(status) = status_filter {
        sqlx::query_as::<_, Lead>(
            "SELECT * FROM leads WHERE tenant_id = $1 AND status = $2 ORDER BY created_at DESC",
        )
        .bind(tenant_id)
        .bind(status)
        .fetch_all(pool)
        .await?
    } else if include_terminal {
        sqlx::query_as::<_, Lead>(
            "SELECT * FROM leads WHERE tenant_id = $1 ORDER BY created_at DESC",
        )
        .bind(tenant_id)
        .fetch_all(pool)
        .await?
    } else {
        sqlx::query_as::<_, Lead>(
            r#"
            SELECT * FROM leads
            WHERE tenant_id = $1 AND status NOT IN ('converted','disqualified','dead')
            ORDER BY created_at DESC
            "#,
        )
        .bind(tenant_id)
        .fetch_all(pool)
        .await?
    };

    if let Some(owner) = owner_filter {
        return Ok(rows
            .into_iter()
            .filter(|l| l.owner_id.as_deref() == Some(owner))
            .collect());
    }
    Ok(rows)
}

pub async fn update_lead_status(
    conn: &mut PgConnection,
    tenant_id: &str,
    id: Uuid,
    new_status: &str,
    party_id: Option<Uuid>,
    party_contact_id: Option<Uuid>,
    converted_opportunity_id: Option<Uuid>,
    disqualify_reason: Option<&str>,
) -> Result<Lead, LeadError> {
    let row = sqlx::query_as::<_, Lead>(
        r#"
        UPDATE leads SET
            status = $3,
            party_id = COALESCE($4, party_id),
            party_contact_id = COALESCE($5, party_contact_id),
            converted_opportunity_id = COALESCE($6, converted_opportunity_id),
            converted_at = CASE WHEN $3 = 'converted' THEN NOW() ELSE converted_at END,
            disqualify_reason = COALESCE($7, disqualify_reason),
            updated_at = NOW()
        WHERE id = $1 AND tenant_id = $2
        RETURNING *
        "#,
    )
    .bind(id)
    .bind(tenant_id)
    .bind(new_status)
    .bind(party_id)
    .bind(party_contact_id)
    .bind(converted_opportunity_id)
    .bind(disqualify_reason)
    .fetch_one(conn)
    .await
    .map_err(|e| match e {
        sqlx::Error::RowNotFound => LeadError::NotFound(id),
        other => LeadError::Database(other),
    })?;
    Ok(row)
}

pub async fn update_lead_fields(
    pool: &PgPool,
    tenant_id: &str,
    id: Uuid,
    req: &super::UpdateLeadRequest,
) -> Result<Lead, LeadError> {
    let row = sqlx::query_as::<_, Lead>(
        r#"
        UPDATE leads SET
            source               = COALESCE($3, source),
            source_detail        = COALESCE($4, source_detail),
            company_name         = COALESCE($5, company_name),
            contact_name         = COALESCE($6, contact_name),
            contact_email        = COALESCE($7, contact_email),
            contact_phone        = COALESCE($8, contact_phone),
            contact_title        = COALESCE($9, contact_title),
            estimated_value_cents = COALESCE($10, estimated_value_cents),
            currency             = COALESCE($11, currency),
            owner_id             = COALESCE($12, owner_id),
            notes                = COALESCE($13, notes),
            updated_at           = NOW()
        WHERE id = $1 AND tenant_id = $2
        RETURNING *
        "#,
    )
    .bind(id)
    .bind(tenant_id)
    .bind(&req.source)
    .bind(&req.source_detail)
    .bind(&req.company_name)
    .bind(&req.contact_name)
    .bind(&req.contact_email)
    .bind(&req.contact_phone)
    .bind(&req.contact_title)
    .bind(req.estimated_value_cents)
    .bind(&req.currency)
    .bind(&req.owner_id)
    .bind(&req.notes)
    .fetch_one(pool)
    .await
    .map_err(|e| match e {
        sqlx::Error::RowNotFound => LeadError::NotFound(id),
        other => LeadError::Database(other),
    })?;
    Ok(row)
}

pub async fn next_lead_number(
    conn: &mut PgConnection,
    tenant_id: &str,
) -> Result<String, LeadError> {
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM leads WHERE tenant_id = $1")
        .bind(tenant_id)
        .fetch_one(&mut *conn)
        .await?;
    Ok(format!("LEAD-{:05}", count + 1))
}
