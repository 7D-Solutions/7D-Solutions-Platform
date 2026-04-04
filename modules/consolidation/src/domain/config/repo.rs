//! Repository layer — all SQL access for consolidation config.

use sqlx::PgPool;
use uuid::Uuid;

use super::models::*;

// ── Groups ──────────────────────────────────────────────────────────────────

pub async fn insert_group(
    pool: &PgPool,
    tenant_id: &str,
    name: &str,
    description: &Option<String>,
    reporting_currency: &str,
    fiscal_year_end_month: i16,
) -> Result<Group, sqlx::Error> {
    sqlx::query_as::<_, Group>(
        "INSERT INTO csl_groups (tenant_id, name, description, reporting_currency, fiscal_year_end_month)
         VALUES ($1, $2, $3, $4, $5)
         RETURNING *",
    )
    .bind(tenant_id)
    .bind(name)
    .bind(description)
    .bind(reporting_currency)
    .bind(fiscal_year_end_month)
    .fetch_one(pool)
    .await
}

pub async fn fetch_groups(
    pool: &PgPool,
    tenant_id: &str,
    include_inactive: bool,
) -> Result<Vec<Group>, sqlx::Error> {
    if include_inactive {
        sqlx::query_as::<_, Group>("SELECT * FROM csl_groups WHERE tenant_id = $1 ORDER BY name")
            .bind(tenant_id)
            .fetch_all(pool)
            .await
    } else {
        sqlx::query_as::<_, Group>(
            "SELECT * FROM csl_groups WHERE tenant_id = $1 AND is_active = TRUE ORDER BY name",
        )
        .bind(tenant_id)
        .fetch_all(pool)
        .await
    }
}

pub async fn fetch_group(
    pool: &PgPool,
    tenant_id: &str,
    id: Uuid,
) -> Result<Option<Group>, sqlx::Error> {
    sqlx::query_as::<_, Group>("SELECT * FROM csl_groups WHERE id = $1 AND tenant_id = $2")
        .bind(id)
        .bind(tenant_id)
        .fetch_optional(pool)
        .await
}

pub async fn update_group_row(
    pool: &PgPool,
    id: Uuid,
    tenant_id: &str,
    req: &UpdateGroupRequest,
) -> Result<Option<Group>, sqlx::Error> {
    sqlx::query_as::<_, Group>(
        "UPDATE csl_groups SET
            name = COALESCE($3, name),
            description = COALESCE($4, description),
            reporting_currency = COALESCE($5, reporting_currency),
            fiscal_year_end_month = COALESCE($6, fiscal_year_end_month),
            is_active = COALESCE($7, is_active),
            updated_at = NOW()
         WHERE id = $1 AND tenant_id = $2
         RETURNING *",
    )
    .bind(id)
    .bind(tenant_id)
    .bind(&req.name)
    .bind(&req.description)
    .bind(&req.reporting_currency)
    .bind(req.fiscal_year_end_month)
    .bind(req.is_active)
    .fetch_optional(pool)
    .await
}

pub async fn delete_group_row(
    pool: &PgPool,
    tenant_id: &str,
    id: Uuid,
) -> Result<u64, sqlx::Error> {
    let result = sqlx::query("DELETE FROM csl_groups WHERE id = $1 AND tenant_id = $2")
        .bind(id)
        .bind(tenant_id)
        .execute(pool)
        .await?;
    Ok(result.rows_affected())
}

// ── Entities ────────────────────────────────────────────────────────────────

pub async fn insert_entity(
    pool: &PgPool,
    group_id: Uuid,
    entity_tenant_id: &str,
    entity_name: &str,
    functional_currency: &str,
    ownership_pct_bp: i32,
    consolidation_method: &str,
) -> Result<GroupEntity, sqlx::Error> {
    sqlx::query_as::<_, GroupEntity>(
        "INSERT INTO csl_group_entities
            (group_id, entity_tenant_id, entity_name, functional_currency, ownership_pct_bp, consolidation_method)
         VALUES ($1, $2, $3, $4, $5, $6)
         RETURNING *",
    )
    .bind(group_id)
    .bind(entity_tenant_id)
    .bind(entity_name)
    .bind(functional_currency)
    .bind(ownership_pct_bp)
    .bind(consolidation_method)
    .fetch_one(pool)
    .await
}

pub async fn fetch_entities(
    pool: &PgPool,
    tenant_id: &str,
    group_id: Uuid,
    include_inactive: bool,
) -> Result<Vec<GroupEntity>, sqlx::Error> {
    if include_inactive {
        sqlx::query_as::<_, GroupEntity>(
            "SELECT * FROM csl_group_entities WHERE group_id = $1 \
             AND group_id IN (SELECT id FROM csl_groups WHERE tenant_id = $2) \
             ORDER BY entity_name",
        )
        .bind(group_id)
        .bind(tenant_id)
        .fetch_all(pool)
        .await
    } else {
        sqlx::query_as::<_, GroupEntity>(
            "SELECT * FROM csl_group_entities WHERE group_id = $1 AND is_active = TRUE \
             AND group_id IN (SELECT id FROM csl_groups WHERE tenant_id = $2) \
             ORDER BY entity_name",
        )
        .bind(group_id)
        .bind(tenant_id)
        .fetch_all(pool)
        .await
    }
}

pub async fn fetch_entity(pool: &PgPool, id: Uuid) -> Result<Option<GroupEntity>, sqlx::Error> {
    sqlx::query_as::<_, GroupEntity>("SELECT * FROM csl_group_entities WHERE id = $1")
        .bind(id)
        .fetch_optional(pool)
        .await
}

pub async fn update_entity_row(
    pool: &PgPool,
    id: Uuid,
    req: &UpdateEntityRequest,
) -> Result<GroupEntity, sqlx::Error> {
    sqlx::query_as::<_, GroupEntity>(
        "UPDATE csl_group_entities SET
            entity_name = COALESCE($2, entity_name),
            functional_currency = COALESCE($3, functional_currency),
            ownership_pct_bp = COALESCE($4, ownership_pct_bp),
            consolidation_method = COALESCE($5, consolidation_method),
            is_active = COALESCE($6, is_active),
            updated_at = NOW()
         WHERE id = $1
         RETURNING *",
    )
    .bind(id)
    .bind(&req.entity_name)
    .bind(&req.functional_currency)
    .bind(req.ownership_pct_bp)
    .bind(&req.consolidation_method)
    .bind(req.is_active)
    .fetch_one(pool)
    .await
}

pub async fn delete_entity_row(pool: &PgPool, id: Uuid) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM csl_group_entities WHERE id = $1")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

// ── COA Mappings ────────────────────────────────────────────────────────────

pub async fn insert_coa_mapping(
    pool: &PgPool,
    group_id: Uuid,
    entity_tenant_id: &str,
    source_account_code: &str,
    target_account_code: &str,
    target_account_name: &Option<String>,
) -> Result<CoaMapping, sqlx::Error> {
    sqlx::query_as::<_, CoaMapping>(
        "INSERT INTO csl_coa_mappings
            (group_id, entity_tenant_id, source_account_code, target_account_code, target_account_name)
         VALUES ($1, $2, $3, $4, $5)
         RETURNING *",
    )
    .bind(group_id)
    .bind(entity_tenant_id)
    .bind(source_account_code)
    .bind(target_account_code)
    .bind(target_account_name)
    .fetch_one(pool)
    .await
}

pub async fn fetch_coa_mappings(
    pool: &PgPool,
    group_id: Uuid,
    entity_tenant_id: Option<&str>,
) -> Result<Vec<CoaMapping>, sqlx::Error> {
    if let Some(eid) = entity_tenant_id {
        sqlx::query_as::<_, CoaMapping>(
            "SELECT * FROM csl_coa_mappings WHERE group_id = $1 AND entity_tenant_id = $2
             ORDER BY source_account_code",
        )
        .bind(group_id)
        .bind(eid)
        .fetch_all(pool)
        .await
    } else {
        sqlx::query_as::<_, CoaMapping>(
            "SELECT * FROM csl_coa_mappings WHERE group_id = $1
             ORDER BY entity_tenant_id, source_account_code",
        )
        .bind(group_id)
        .fetch_all(pool)
        .await
    }
}

pub async fn fetch_coa_mapping(pool: &PgPool, id: Uuid) -> Result<Option<CoaMapping>, sqlx::Error> {
    sqlx::query_as::<_, CoaMapping>("SELECT * FROM csl_coa_mappings WHERE id = $1")
        .bind(id)
        .fetch_optional(pool)
        .await
}

pub async fn delete_coa_mapping_row(pool: &PgPool, id: Uuid) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM csl_coa_mappings WHERE id = $1")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn count_coa_mappings(
    pool: &PgPool,
    group_id: Uuid,
    entity_tenant_id: &str,
) -> Result<i64, sqlx::Error> {
    let (count,): (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM csl_coa_mappings
         WHERE group_id = $1 AND entity_tenant_id = $2",
    )
    .bind(group_id)
    .bind(entity_tenant_id)
    .fetch_one(pool)
    .await?;
    Ok(count)
}

// ── Elimination Rules ───────────────────────────────────────────────────────

pub async fn insert_elimination_rule(
    pool: &PgPool,
    group_id: Uuid,
    rule_name: &str,
    rule_type: &str,
    debit_account_code: &str,
    credit_account_code: &str,
    description: &Option<String>,
) -> Result<EliminationRule, sqlx::Error> {
    sqlx::query_as::<_, EliminationRule>(
        "INSERT INTO csl_elimination_rules
            (group_id, rule_name, rule_type, debit_account_code, credit_account_code, description)
         VALUES ($1, $2, $3, $4, $5, $6)
         RETURNING *",
    )
    .bind(group_id)
    .bind(rule_name)
    .bind(rule_type)
    .bind(debit_account_code)
    .bind(credit_account_code)
    .bind(description)
    .fetch_one(pool)
    .await
}

pub async fn fetch_elimination_rules(
    pool: &PgPool,
    tenant_id: &str,
    group_id: Uuid,
    include_inactive: bool,
) -> Result<Vec<EliminationRule>, sqlx::Error> {
    if include_inactive {
        sqlx::query_as::<_, EliminationRule>(
            "SELECT * FROM csl_elimination_rules WHERE group_id = $1 \
             AND group_id IN (SELECT id FROM csl_groups WHERE tenant_id = $2) \
             ORDER BY rule_name",
        )
        .bind(group_id)
        .bind(tenant_id)
        .fetch_all(pool)
        .await
    } else {
        sqlx::query_as::<_, EliminationRule>(
            "SELECT * FROM csl_elimination_rules WHERE group_id = $1 AND is_active = TRUE \
             AND group_id IN (SELECT id FROM csl_groups WHERE tenant_id = $2) \
             ORDER BY rule_name",
        )
        .bind(group_id)
        .bind(tenant_id)
        .fetch_all(pool)
        .await
    }
}

pub async fn fetch_elimination_rule(
    pool: &PgPool,
    id: Uuid,
) -> Result<Option<EliminationRule>, sqlx::Error> {
    sqlx::query_as::<_, EliminationRule>("SELECT * FROM csl_elimination_rules WHERE id = $1")
        .bind(id)
        .fetch_optional(pool)
        .await
}

pub async fn update_elimination_rule_row(
    pool: &PgPool,
    id: Uuid,
    req: &UpdateEliminationRuleRequest,
) -> Result<EliminationRule, sqlx::Error> {
    sqlx::query_as::<_, EliminationRule>(
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
    .await
}

pub async fn delete_elimination_rule_row(pool: &PgPool, id: Uuid) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM csl_elimination_rules WHERE id = $1")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

// ── FX Policies ─────────────────────────────────────────────────────────────

pub async fn upsert_fx_policy_row(
    pool: &PgPool,
    group_id: Uuid,
    entity_tenant_id: &str,
    bs_rate_type: &str,
    pl_rate_type: &str,
    equity_rate_type: &str,
    fx_rate_source: &str,
) -> Result<FxPolicy, sqlx::Error> {
    sqlx::query_as::<_, FxPolicy>(
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
    .bind(entity_tenant_id)
    .bind(bs_rate_type)
    .bind(pl_rate_type)
    .bind(equity_rate_type)
    .bind(fx_rate_source)
    .fetch_one(pool)
    .await
}

pub async fn fetch_fx_policies(
    pool: &PgPool,
    group_id: Uuid,
) -> Result<Vec<FxPolicy>, sqlx::Error> {
    sqlx::query_as::<_, FxPolicy>(
        "SELECT * FROM csl_fx_policies WHERE group_id = $1 ORDER BY entity_tenant_id",
    )
    .bind(group_id)
    .fetch_all(pool)
    .await
}

pub async fn fetch_fx_policy(pool: &PgPool, id: Uuid) -> Result<Option<FxPolicy>, sqlx::Error> {
    sqlx::query_as::<_, FxPolicy>("SELECT * FROM csl_fx_policies WHERE id = $1")
        .bind(id)
        .fetch_optional(pool)
        .await
}

pub async fn delete_fx_policy_row(
    pool: &PgPool,
    id: Uuid,
    tenant_id: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "DELETE FROM csl_fx_policies WHERE id = $1 \
         AND group_id IN (SELECT id FROM csl_groups WHERE tenant_id = $2)",
    )
    .bind(id)
    .bind(tenant_id)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn count_fx_policies(
    pool: &PgPool,
    group_id: Uuid,
    entity_tenant_id: &str,
) -> Result<i64, sqlx::Error> {
    let (count,): (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM csl_fx_policies
         WHERE group_id = $1 AND entity_tenant_id = $2",
    )
    .bind(group_id)
    .bind(entity_tenant_id)
    .fetch_one(pool)
    .await?;
    Ok(count)
}
