//! Service layer for elimination rules and FX policies.

use sqlx::PgPool;
use uuid::Uuid;

use super::{
    models::*, repo, service::get_group, validate_not_blank, validate_rate_type,
    validate_rule_type, ConfigError,
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

    let row = repo::insert_elimination_rule(
        pool,
        group_id,
        &req.rule_name,
        &req.rule_type,
        &req.debit_account_code,
        &req.credit_account_code,
        &req.description,
    )
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
    Ok(repo::fetch_elimination_rules(pool, tenant_id, group_id, include_inactive).await?)
}

pub async fn get_elimination_rule(
    pool: &PgPool,
    tenant_id: &str,
    id: Uuid,
) -> Result<EliminationRule, ConfigError> {
    repo::fetch_elimination_rule(pool, tenant_id, id)
        .await?
        .ok_or(ConfigError::RuleNotFound(id))
}

pub async fn update_elimination_rule(
    pool: &PgPool,
    tenant_id: &str,
    id: Uuid,
    req: &UpdateEliminationRuleRequest,
) -> Result<EliminationRule, ConfigError> {
    let existing = get_elimination_rule(pool, tenant_id, id).await?;
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

    let row = repo::update_elimination_rule_row(pool, id, req).await?;
    Ok(row)
}

pub async fn delete_elimination_rule(
    pool: &PgPool,
    tenant_id: &str,
    id: Uuid,
) -> Result<(), ConfigError> {
    let existing = get_elimination_rule(pool, tenant_id, id).await?;
    get_group(pool, tenant_id, existing.group_id).await?;
    repo::delete_elimination_rule_row(pool, id).await?;
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

    let row =
        repo::upsert_fx_policy_row(pool, group_id, &req.entity_tenant_id, bs, pl, eq, src).await?;
    Ok(row)
}

pub async fn list_fx_policies(
    pool: &PgPool,
    tenant_id: &str,
    group_id: Uuid,
) -> Result<Vec<FxPolicy>, ConfigError> {
    get_group(pool, tenant_id, group_id).await?;
    Ok(repo::fetch_fx_policies(pool, group_id).await?)
}

pub async fn delete_fx_policy(pool: &PgPool, tenant_id: &str, id: Uuid) -> Result<(), ConfigError> {
    let existing = repo::fetch_fx_policy(pool, id)
        .await?
        .ok_or(ConfigError::PolicyNotFound(id))?;

    get_group(pool, tenant_id, existing.group_id).await?;
    repo::delete_fx_policy_row(pool, id, tenant_id).await?;
    Ok(())
}
