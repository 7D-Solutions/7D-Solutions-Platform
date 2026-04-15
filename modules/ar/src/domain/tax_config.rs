//! Tax config repository — jurisdictions and rules SQL operations.

use chrono::NaiveDate;
use serde::Serialize;
use sqlx::PgExecutor;
use uuid::Uuid;

// Types live here in domain; http/tax_config re-exports them.

#[derive(Debug, Serialize)]
pub struct JurisdictionResponse {
    pub id: Uuid,
    pub app_id: String,
    pub country_code: String,
    pub state_code: Option<String>,
    pub postal_pattern: Option<String>,
    pub jurisdiction_name: String,
    pub tax_type: String,
    pub is_active: bool,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Serialize)]
pub struct RuleResponse {
    pub id: Uuid,
    pub jurisdiction_id: Uuid,
    pub app_id: String,
    pub tax_code: Option<String>,
    pub rate: f64,
    pub flat_amount_minor: i64,
    pub is_exempt: bool,
    pub effective_from: NaiveDate,
    pub effective_to: Option<NaiveDate>,
    pub priority: i32,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

pub fn row_to_jurisdiction(
    r: (
        Uuid,
        String,
        String,
        Option<String>,
        Option<String>,
        String,
        String,
        bool,
        chrono::DateTime<chrono::Utc>,
        chrono::DateTime<chrono::Utc>,
    ),
) -> JurisdictionResponse {
    JurisdictionResponse {
        id: r.0,
        app_id: r.1,
        country_code: r.2,
        state_code: r.3,
        postal_pattern: r.4,
        jurisdiction_name: r.5,
        tax_type: r.6,
        is_active: r.7,
        created_at: r.8,
        updated_at: r.9,
    }
}

pub fn row_to_rule(
    r: (
        Uuid,
        Uuid,
        String,
        Option<String>,
        f64,
        i64,
        bool,
        NaiveDate,
        Option<NaiveDate>,
        i32,
        chrono::DateTime<chrono::Utc>,
        chrono::DateTime<chrono::Utc>,
    ),
) -> RuleResponse {
    RuleResponse {
        id: r.0,
        jurisdiction_id: r.1,
        app_id: r.2,
        tax_code: r.3,
        rate: r.4,
        flat_amount_minor: r.5,
        is_exempt: r.6,
        effective_from: r.7,
        effective_to: r.8,
        priority: r.9,
        created_at: r.10,
        updated_at: r.11,
    }
}

// ============================================================================
// Jurisdictions
// ============================================================================

/// List jurisdictions with optional filters.
pub async fn list_jurisdictions<'e>(
    executor: impl PgExecutor<'e>,
    app_id: &str,
    is_active: bool,
    country_code: Option<&str>,
    state_code: Option<&str>,
) -> Result<Vec<JurisdictionResponse>, sqlx::Error> {
    let rows = sqlx::query_as::<
        _,
        (
            Uuid,
            String,
            String,
            Option<String>,
            Option<String>,
            String,
            String,
            bool,
            chrono::DateTime<chrono::Utc>,
            chrono::DateTime<chrono::Utc>,
        ),
    >(
        r#"
        SELECT id, app_id, country_code, state_code, postal_pattern,
               jurisdiction_name, tax_type, is_active, created_at, updated_at
        FROM ar_tax_jurisdictions
        WHERE app_id = $1
          AND is_active = $2
          AND ($3::VARCHAR IS NULL OR country_code = $3)
          AND ($4::VARCHAR IS NULL OR state_code = $4)
        ORDER BY country_code, state_code, postal_pattern
        "#,
    )
    .bind(app_id)
    .bind(is_active)
    .bind(country_code)
    .bind(state_code)
    .fetch_all(executor)
    .await?;

    Ok(rows.into_iter().map(row_to_jurisdiction).collect())
}

/// Fetch a jurisdiction by ID and tenant.
pub async fn get_jurisdiction_by_id_and_tenant<'e>(
    executor: impl PgExecutor<'e>,
    id: Uuid,
    app_id: &str,
) -> Result<Option<JurisdictionResponse>, sqlx::Error> {
    let row = sqlx::query_as::<
        _,
        (
            Uuid,
            String,
            String,
            Option<String>,
            Option<String>,
            String,
            String,
            bool,
            chrono::DateTime<chrono::Utc>,
            chrono::DateTime<chrono::Utc>,
        ),
    >(
        r#"
        SELECT id, app_id, country_code, state_code, postal_pattern,
               jurisdiction_name, tax_type, is_active, created_at, updated_at
        FROM ar_tax_jurisdictions
        WHERE id = $1 AND app_id = $2
        "#,
    )
    .bind(id)
    .bind(app_id)
    .fetch_optional(executor)
    .await?;

    Ok(row.map(row_to_jurisdiction))
}

/// Update jurisdiction fields.
pub async fn update_jurisdiction<'e>(
    executor: impl PgExecutor<'e>,
    id: Uuid,
    jurisdiction_name: Option<&str>,
    tax_type: Option<&str>,
    is_active: Option<bool>,
    app_id: &str,
) -> Result<u64, sqlx::Error> {
    let result = sqlx::query(
        r#"
        UPDATE ar_tax_jurisdictions SET
            jurisdiction_name = COALESCE($2, jurisdiction_name),
            tax_type = COALESCE($3, tax_type),
            is_active = COALESCE($4, is_active),
            updated_at = NOW()
        WHERE id = $1 AND app_id = $5
        "#,
    )
    .bind(id)
    .bind(jurisdiction_name)
    .bind(tax_type)
    .bind(is_active)
    .bind(app_id)
    .execute(executor)
    .await?;
    Ok(result.rows_affected())
}

// ============================================================================
// Rules
// ============================================================================

/// Fetch a tax rule by ID and tenant.
pub async fn get_rule_by_id_and_tenant<'e>(
    executor: impl PgExecutor<'e>,
    id: Uuid,
    app_id: &str,
) -> Result<Option<RuleResponse>, sqlx::Error> {
    let row = sqlx::query_as::<
        _,
        (
            Uuid,
            Uuid,
            String,
            Option<String>,
            f64,
            i64,
            bool,
            NaiveDate,
            Option<NaiveDate>,
            i32,
            chrono::DateTime<chrono::Utc>,
            chrono::DateTime<chrono::Utc>,
        ),
    >(
        r#"
        SELECT id, jurisdiction_id, app_id, tax_code, rate::FLOAT8,
               flat_amount_minor, is_exempt, effective_from, effective_to, priority,
               created_at, updated_at
        FROM ar_tax_rules
        WHERE id = $1 AND app_id = $2
        "#,
    )
    .bind(id)
    .bind(app_id)
    .fetch_optional(executor)
    .await?;

    Ok(row.map(row_to_rule))
}

/// List tax rules with optional filters.
pub async fn list_rules<'e>(
    executor: impl PgExecutor<'e>,
    app_id: &str,
    jurisdiction_id: Option<Uuid>,
    as_of: Option<NaiveDate>,
) -> Result<Vec<RuleResponse>, sqlx::Error> {
    let rows = sqlx::query_as::<_, (
        Uuid, Uuid, String, Option<String>, f64,
        i64, bool, NaiveDate, Option<NaiveDate>, i32,
        chrono::DateTime<chrono::Utc>, chrono::DateTime<chrono::Utc>,
    )>(
        r#"
        SELECT id, jurisdiction_id, app_id, tax_code, rate::FLOAT8,
               flat_amount_minor, is_exempt, effective_from, effective_to, priority,
               created_at, updated_at
        FROM ar_tax_rules
        WHERE app_id = $1
          AND ($2::UUID IS NULL OR jurisdiction_id = $2)
          AND ($3::DATE IS NULL OR (effective_from <= $3 AND (effective_to IS NULL OR effective_to > $3)))
        ORDER BY jurisdiction_id, priority DESC, effective_from DESC
        "#,
    )
    .bind(app_id)
    .bind(jurisdiction_id)
    .bind(as_of)
    .fetch_all(executor)
    .await?;

    Ok(rows.into_iter().map(row_to_rule).collect())
}

/// Update tax rule fields.
#[allow(clippy::too_many_arguments)]
pub async fn update_rule<'e>(
    executor: impl PgExecutor<'e>,
    id: Uuid,
    rate: Option<f64>,
    flat_amount_minor: Option<i64>,
    is_exempt: Option<bool>,
    effective_to: Option<NaiveDate>,
    priority: Option<i32>,
    app_id: &str,
) -> Result<u64, sqlx::Error> {
    let result = sqlx::query(
        r#"
        UPDATE ar_tax_rules SET
            rate             = COALESCE($2, rate),
            flat_amount_minor = COALESCE($3, flat_amount_minor),
            is_exempt        = COALESCE($4, is_exempt),
            effective_to     = COALESCE($5, effective_to),
            priority         = COALESCE($6, priority),
            updated_at       = NOW()
        WHERE id = $1 AND app_id = $7
        "#,
    )
    .bind(id)
    .bind(rate)
    .bind(flat_amount_minor)
    .bind(is_exempt)
    .bind(effective_to)
    .bind(priority)
    .bind(app_id)
    .execute(executor)
    .await?;
    Ok(result.rows_affected())
}

// ============================================================================
// Tax quotes
// ============================================================================

/// Look up the most recent cached tax quote for a tenant + invoice_id.
pub async fn lookup_cached_quote<'e>(
    executor: impl PgExecutor<'e>,
    tenant_id: &str,
    invoice_id: &str,
) -> Result<
    Option<(
        uuid::Uuid,
        String,
        String,
        String,
        String,
        i64,
        serde_json::Value,
        chrono::DateTime<chrono::Utc>,
    )>,
    sqlx::Error,
> {
    sqlx::query_as::<
        _,
        (
            uuid::Uuid,
            String,
            String,
            String,
            String,
            i64,
            serde_json::Value,
            chrono::DateTime<chrono::Utc>,
        ),
    >(
        r#"
        SELECT id, provider, provider_quote_ref, request_hash, idempotency_key,
               total_tax_minor, tax_by_line, quoted_at
        FROM ar_tax_quote_cache
        WHERE app_id = $1 AND invoice_id = $2
        ORDER BY created_at DESC
        LIMIT 1
        "#,
    )
    .bind(tenant_id)
    .bind(invoice_id)
    .fetch_optional(executor)
    .await
}
