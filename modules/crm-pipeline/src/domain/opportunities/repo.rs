//! Opportunity repository.

use chrono::Utc;
use sqlx::{PgConnection, PgPool};
use uuid::Uuid;

use super::{
    ListOpportunitiesQuery, Opportunity, OpportunityError, OpportunityStageHistory,
    PipelineSummaryItem,
};

pub async fn insert_opportunity(
    conn: &mut PgConnection,
    opp: &Opportunity,
) -> Result<Opportunity, OpportunityError> {
    let row = sqlx::query_as::<_, Opportunity>(
        r#"
        INSERT INTO opportunities (
            id, tenant_id, opp_number, title, party_id, primary_party_contact_id, lead_id,
            stage_code, probability_pct, estimated_value_cents, currency,
            expected_close_date, actual_close_date, close_reason, competitor,
            opp_type, priority, description, requirements, external_quote_ref,
            sales_order_id, owner_id, created_by, created_at, updated_at
        ) VALUES (
            $1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$18,$19,$20,$21,$22,$23,$24,$25
        )
        RETURNING *
        "#,
    )
    .bind(opp.id)
    .bind(&opp.tenant_id)
    .bind(&opp.opp_number)
    .bind(&opp.title)
    .bind(opp.party_id)
    .bind(opp.primary_party_contact_id)
    .bind(opp.lead_id)
    .bind(&opp.stage_code)
    .bind(opp.probability_pct)
    .bind(opp.estimated_value_cents)
    .bind(&opp.currency)
    .bind(opp.expected_close_date)
    .bind(opp.actual_close_date)
    .bind(&opp.close_reason)
    .bind(&opp.competitor)
    .bind(&opp.opp_type)
    .bind(&opp.priority)
    .bind(&opp.description)
    .bind(&opp.requirements)
    .bind(&opp.external_quote_ref)
    .bind(opp.sales_order_id)
    .bind(&opp.owner_id)
    .bind(&opp.created_by)
    .bind(opp.created_at)
    .bind(opp.updated_at)
    .fetch_one(conn)
    .await?;
    Ok(row)
}

pub async fn fetch_opportunity(
    pool: &PgPool,
    tenant_id: &str,
    id: Uuid,
) -> Result<Option<Opportunity>, OpportunityError> {
    let row = sqlx::query_as::<_, Opportunity>(
        "SELECT * FROM opportunities WHERE id = $1 AND tenant_id = $2",
    )
    .bind(id)
    .bind(tenant_id)
    .fetch_optional(pool)
    .await?;
    Ok(row)
}

pub async fn list_opportunities(
    pool: &PgPool,
    tenant_id: &str,
    query: &ListOpportunitiesQuery,
) -> Result<Vec<Opportunity>, OpportunityError> {
    // Build query with optional filters — use a broad fetch then filter
    // For production, parameterised dynamic query builder would be used.
    let include_closed = query.include_closed.unwrap_or(false);

    let rows = sqlx::query_as::<_, Opportunity>(
        r#"
        SELECT o.* FROM opportunities o
        JOIN pipeline_stages ps ON ps.tenant_id = o.tenant_id AND ps.stage_code = o.stage_code
        WHERE o.tenant_id = $1
          AND ($2::text IS NULL OR o.owner_id = $2)
          AND ($3::text IS NULL OR o.stage_code = $3)
          AND ($4::uuid IS NULL OR o.party_id = $4)
          AND ($5::boolean = TRUE OR ps.is_terminal = FALSE)
        ORDER BY o.created_at DESC
        "#,
    )
    .bind(tenant_id)
    .bind(&query.owner_id)
    .bind(&query.stage_code)
    .bind(query.party_id)
    .bind(include_closed)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

pub async fn update_opportunity_stage(
    conn: &mut PgConnection,
    tenant_id: &str,
    id: Uuid,
    stage_code: &str,
    probability_pct: i32,
    actual_close_date: Option<chrono::NaiveDate>,
    close_reason: Option<&str>,
    competitor: Option<&str>,
    sales_order_id: Option<Uuid>,
) -> Result<Opportunity, OpportunityError> {
    let row = sqlx::query_as::<_, Opportunity>(
        r#"
        UPDATE opportunities SET
            stage_code         = $3,
            probability_pct    = $4,
            actual_close_date  = COALESCE($5, actual_close_date),
            close_reason       = COALESCE($6, close_reason),
            competitor         = COALESCE($7, competitor),
            sales_order_id     = COALESCE($8, sales_order_id),
            updated_at         = NOW()
        WHERE id = $1 AND tenant_id = $2
        RETURNING *
        "#,
    )
    .bind(id)
    .bind(tenant_id)
    .bind(stage_code)
    .bind(probability_pct)
    .bind(actual_close_date)
    .bind(close_reason)
    .bind(competitor)
    .bind(sales_order_id)
    .fetch_one(conn)
    .await
    .map_err(|e| match e {
        sqlx::Error::RowNotFound => OpportunityError::NotFound(id),
        other => OpportunityError::Database(other),
    })?;
    Ok(row)
}

pub async fn update_opportunity_fields(
    pool: &PgPool,
    tenant_id: &str,
    id: Uuid,
    req: &super::UpdateOpportunityRequest,
) -> Result<Opportunity, OpportunityError> {
    let row = sqlx::query_as::<_, Opportunity>(
        r#"
        UPDATE opportunities SET
            title                    = COALESCE($3, title),
            primary_party_contact_id = COALESCE($4, primary_party_contact_id),
            probability_pct          = COALESCE($5, probability_pct),
            estimated_value_cents    = COALESCE($6, estimated_value_cents),
            currency                 = COALESCE($7, currency),
            expected_close_date      = COALESCE($8, expected_close_date),
            opp_type                 = COALESCE($9, opp_type),
            priority                 = COALESCE($10, priority),
            description              = COALESCE($11, description),
            requirements             = COALESCE($12, requirements),
            external_quote_ref       = COALESCE($13, external_quote_ref),
            owner_id                 = COALESCE($14, owner_id),
            competitor               = COALESCE($15, competitor),
            updated_at               = NOW()
        WHERE id = $1 AND tenant_id = $2
        RETURNING *
        "#,
    )
    .bind(id)
    .bind(tenant_id)
    .bind(&req.title)
    .bind(req.primary_party_contact_id)
    .bind(req.probability_pct)
    .bind(req.estimated_value_cents)
    .bind(&req.currency)
    .bind(req.expected_close_date)
    .bind(&req.opp_type)
    .bind(&req.priority)
    .bind(&req.description)
    .bind(&req.requirements)
    .bind(&req.external_quote_ref)
    .bind(&req.owner_id)
    .bind(&req.competitor)
    .fetch_one(pool)
    .await
    .map_err(|e| match e {
        sqlx::Error::RowNotFound => OpportunityError::NotFound(id),
        other => OpportunityError::Database(other),
    })?;
    Ok(row)
}

pub async fn insert_stage_history(
    conn: &mut PgConnection,
    entry: &OpportunityStageHistory,
) -> Result<OpportunityStageHistory, OpportunityError> {
    let row = sqlx::query_as::<_, OpportunityStageHistory>(
        r#"
        INSERT INTO opportunity_stage_history (
            id, tenant_id, opportunity_id, from_stage_code, to_stage_code,
            probability_pct_at_change, days_in_previous_stage, reason, notes,
            changed_by, changed_at
        ) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11)
        RETURNING *
        "#,
    )
    .bind(entry.id)
    .bind(&entry.tenant_id)
    .bind(entry.opportunity_id)
    .bind(&entry.from_stage_code)
    .bind(&entry.to_stage_code)
    .bind(entry.probability_pct_at_change)
    .bind(entry.days_in_previous_stage)
    .bind(&entry.reason)
    .bind(&entry.notes)
    .bind(&entry.changed_by)
    .bind(entry.changed_at)
    .fetch_one(conn)
    .await?;
    Ok(row)
}

pub async fn list_stage_history(
    pool: &PgPool,
    tenant_id: &str,
    opportunity_id: Uuid,
) -> Result<Vec<OpportunityStageHistory>, OpportunityError> {
    let rows = sqlx::query_as::<_, OpportunityStageHistory>(
        "SELECT * FROM opportunity_stage_history WHERE tenant_id = $1 AND opportunity_id = $2 ORDER BY changed_at ASC",
    )
    .bind(tenant_id)
    .bind(opportunity_id)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

pub async fn pipeline_summary(
    pool: &PgPool,
    tenant_id: &str,
    owner_id: Option<&str>,
) -> Result<Vec<PipelineSummaryItem>, OpportunityError> {
    let rows = sqlx::query_as::<_, (String, String, i32, i64, i64)>(
        r#"
        SELECT
            ps.stage_code,
            ps.display_label,
            ps.order_rank,
            COUNT(o.id) AS count,
            COALESCE(SUM(o.estimated_value_cents), 0) AS total_value_cents
        FROM pipeline_stages ps
        LEFT JOIN opportunities o
            ON o.tenant_id = ps.tenant_id
            AND o.stage_code = ps.stage_code
            AND ($2::text IS NULL OR o.owner_id = $2)
        WHERE ps.tenant_id = $1 AND ps.active = TRUE AND ps.is_terminal = FALSE
        GROUP BY ps.stage_code, ps.display_label, ps.order_rank
        ORDER BY ps.order_rank ASC
        "#,
    )
    .bind(tenant_id)
    .bind(owner_id)
    .fetch_all(pool)
    .await?;

    let items = rows
        .into_iter()
        .map(
            |(stage_code, display_label, order_rank, count, total_value_cents)| {
                PipelineSummaryItem {
                    stage_code,
                    display_label,
                    order_rank,
                    count,
                    total_value_cents,
                    weighted_value_cents: 0, // caller can compute from probability_pct if needed
                }
            },
        )
        .collect();
    Ok(items)
}

pub async fn next_opp_number(
    conn: &mut PgConnection,
    tenant_id: &str,
) -> Result<String, OpportunityError> {
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM opportunities WHERE tenant_id = $1")
        .bind(tenant_id)
        .fetch_one(&mut *conn)
        .await?;
    Ok(format!("OPP-{:05}", count + 1))
}
