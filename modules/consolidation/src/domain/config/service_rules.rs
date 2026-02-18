//! Service layer for elimination rules and FX policies.

use sqlx::PgPool;
use uuid::Uuid;

use super::{
    models::*, service::get_group, validate_not_blank, validate_rate_type, validate_rule_type,
    ConfigError,
};

// ============================================================================
// Elimination rules
// ============================================================================

pub async fn create_elimination_rule(
    pool: &PgPool,
    tenant_id: &str,
    group_id: Uuid,
    req: &CreateEliminationRuleRequest,
) -> Result<EliminationRule, ConfigError> {
    get_group(pool, tenant_id, group_id).await?;
    validate_not_blank(&req.rule_name, "rule_name")?;
    validate_rule_type(&req.rule_type)?;
    validate_not_blank(&req.debit_account_code, "debit_account_code")?;
    validate_not_blank(&req.credit_account_code, "credit_account_code")?;

    let row = sqlx::query_as::<_, EliminationRule>(
        "INSERT INTO csl_elimination_rules
            (group_id, rule_name, rule_type, debit_account_code, credit_account_code, description)
         VALUES ($1, $2, $3, $4, $5, $6)
         RETURNING *",
    )
    .bind(group_id)
    .bind(&req.rule_name)
    .bind(&req.rule_type)
    .bind(&req.debit_account_code)
    .bind(&req.credit_account_code)
    .bind(&req.description)
    .fetch_one(pool)
    .await
    .map_err(|e| match &e {
        sqlx::Error::Database(db)
            if db.constraint() == Some("csl_elimination_rules_unique_name") =>
        {
            ConfigError::Conflict(format!("Rule '{}' already exists", req.rule_name))
        }
        _ => ConfigError::Database(e),
    })?;
    Ok(row)
}

pub async fn list_elimination_rules(
    pool: &PgPool,
    tenant_id: &str,
    group_id: Uuid,
    include_inactive: bool,
) -> Result<Vec<EliminationRule>, ConfigError> {
    get_group(pool, tenant_id, group_id).await?;

    let rows = if include_inactive {
        sqlx::query_as::<_, EliminationRule>(
            "SELECT * FROM csl_elimination_rules WHERE group_id = $1 ORDER BY rule_name",
        )
        .bind(group_id)
        .fetch_all(pool)
        .await?
    } else {
        sqlx::query_as::<_, EliminationRule>(
            "SELECT * FROM csl_elimination_rules WHERE group_id = $1 AND is_active = TRUE ORDER BY rule_name",
        )
        .bind(group_id)
        .fetch_all(pool)
        .await?
    };
    Ok(rows)
}

pub async fn get_elimination_rule(pool: &PgPool, id: Uuid) -> Result<EliminationRule, ConfigError> {
    sqlx::query_as::<_, EliminationRule>("SELECT * FROM csl_elimination_rules WHERE id = $1")
        .bind(id)
        .fetch_optional(pool)
        .await?
        .ok_or(ConfigError::RuleNotFound(id))
}

pub async fn update_elimination_rule(
    pool: &PgPool,
    tenant_id: &str,
    id: Uuid,
    req: &UpdateEliminationRuleRequest,
) -> Result<EliminationRule, ConfigError> {
    let existing = get_elimination_rule(pool, id).await?;
    get_group(pool, tenant_id, existing.group_id).await?;

    if let Some(ref name) = req.rule_name {
        validate_not_blank(name, "rule_name")?;
    }
    if let Some(ref rt) = req.rule_type {
        validate_rule_type(rt)?;
    }
    if let Some(ref code) = req.debit_account_code {
        validate_not_blank(code, "debit_account_code")?;
    }
    if let Some(ref code) = req.credit_account_code {
        validate_not_blank(code, "credit_account_code")?;
    }

    let row = sqlx::query_as::<_, EliminationRule>(
        "UPDATE csl_elimination_rules SET
            rule_name = COALESCE($2, rule_name),
            rule_type = COALESCE($3, rule_type),
            debit_account_code = COALESCE($4, debit_account_code),
            credit_account_code = COALESCE($5, credit_account_code),
            description = COALESCE($6, description),
            is_active = COALESCE($7, is_active),
            updated_at = NOW()
         WHERE id = $1
         RETURNING *",
    )
    .bind(id)
    .bind(&req.rule_name)
    .bind(&req.rule_type)
    .bind(&req.debit_account_code)
    .bind(&req.credit_account_code)
    .bind(&req.description)
    .bind(req.is_active)
    .fetch_one(pool)
    .await?;
    Ok(row)
}

pub async fn delete_elimination_rule(
    pool: &PgPool,
    tenant_id: &str,
    id: Uuid,
) -> Result<(), ConfigError> {
    let existing = get_elimination_rule(pool, id).await?;
    get_group(pool, tenant_id, existing.group_id).await?;

    sqlx::query("DELETE FROM csl_elimination_rules WHERE id = $1")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

// ============================================================================
// FX policies
// ============================================================================

pub async fn upsert_fx_policy(
    pool: &PgPool,
    tenant_id: &str,
    group_id: Uuid,
    req: &UpsertFxPolicyRequest,
) -> Result<FxPolicy, ConfigError> {
    get_group(pool, tenant_id, group_id).await?;
    validate_not_blank(&req.entity_tenant_id, "entity_tenant_id")?;

    let bs = req.bs_rate_type.as_deref().unwrap_or("closing");
    let pl = req.pl_rate_type.as_deref().unwrap_or("average");
    let eq = req.equity_rate_type.as_deref().unwrap_or("historical");
    let src = req.fx_rate_source.as_deref().unwrap_or("gl");

    validate_rate_type(bs, "bs_rate_type")?;
    validate_rate_type(pl, "pl_rate_type")?;
    validate_rate_type(eq, "equity_rate_type")?;

    let row = sqlx::query_as::<_, FxPolicy>(
        "INSERT INTO csl_fx_policies
            (group_id, entity_tenant_id, bs_rate_type, pl_rate_type, equity_rate_type, fx_rate_source)
         VALUES ($1, $2, $3, $4, $5, $6)
         ON CONFLICT (group_id, entity_tenant_id) DO UPDATE SET
            bs_rate_type = EXCLUDED.bs_rate_type,
            pl_rate_type = EXCLUDED.pl_rate_type,
            equity_rate_type = EXCLUDED.equity_rate_type,
            fx_rate_source = EXCLUDED.fx_rate_source,
            updated_at = NOW()
         RETURNING *",
    )
    .bind(group_id)
    .bind(&req.entity_tenant_id)
    .bind(bs)
    .bind(pl)
    .bind(eq)
    .bind(src)
    .fetch_one(pool)
    .await?;
    Ok(row)
}

pub async fn list_fx_policies(
    pool: &PgPool,
    tenant_id: &str,
    group_id: Uuid,
) -> Result<Vec<FxPolicy>, ConfigError> {
    get_group(pool, tenant_id, group_id).await?;

    let rows = sqlx::query_as::<_, FxPolicy>(
        "SELECT * FROM csl_fx_policies WHERE group_id = $1 ORDER BY entity_tenant_id",
    )
    .bind(group_id)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

pub async fn delete_fx_policy(
    pool: &PgPool,
    tenant_id: &str,
    id: Uuid,
) -> Result<(), ConfigError> {
    let existing = sqlx::query_as::<_, FxPolicy>(
        "SELECT * FROM csl_fx_policies WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(pool)
    .await?
    .ok_or(ConfigError::PolicyNotFound(id))?;

    get_group(pool, tenant_id, existing.group_id).await?;

    sqlx::query("DELETE FROM csl_fx_policies WHERE id = $1")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}
