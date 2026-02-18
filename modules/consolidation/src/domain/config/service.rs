//! Service layer for consolidation config CRUD — groups, entities, COA mappings.
//!
//! All queries are tenant-scoped (via group.tenant_id).
//! Audit fields (created_at, updated_at) are managed by DB defaults.

use sqlx::PgPool;
use uuid::Uuid;

use super::{
    models::*, validate_consolidation_method, validate_currency, validate_fiscal_month,
    validate_not_blank, validate_ownership_bp, ConfigError,
};

// ============================================================================
// Groups
// ============================================================================

pub async fn create_group(
    pool: &PgPool,
    tenant_id: &str,
    req: &CreateGroupRequest,
) -> Result<Group, ConfigError> {
    validate_not_blank(&req.name, "name")?;
    validate_currency(&req.reporting_currency)?;
    let month = req.fiscal_year_end_month.unwrap_or(12);
    validate_fiscal_month(month)?;

    let row = sqlx::query_as::<_, Group>(
        "INSERT INTO csl_groups (tenant_id, name, description, reporting_currency, fiscal_year_end_month)
         VALUES ($1, $2, $3, $4, $5)
         RETURNING *",
    )
    .bind(tenant_id)
    .bind(&req.name)
    .bind(&req.description)
    .bind(&req.reporting_currency)
    .bind(month)
    .fetch_one(pool)
    .await
    .map_err(|e| match &e {
        sqlx::Error::Database(db) if db.constraint() == Some("csl_groups_unique_name") => {
            ConfigError::Conflict(format!("Group name '{}' already exists", req.name))
        }
        _ => ConfigError::Database(e),
    })?;
    Ok(row)
}

pub async fn list_groups(
    pool: &PgPool,
    tenant_id: &str,
    include_inactive: bool,
) -> Result<Vec<Group>, ConfigError> {
    let rows = if include_inactive {
        sqlx::query_as::<_, Group>(
            "SELECT * FROM csl_groups WHERE tenant_id = $1 ORDER BY name",
        )
        .bind(tenant_id)
        .fetch_all(pool)
        .await?
    } else {
        sqlx::query_as::<_, Group>(
            "SELECT * FROM csl_groups WHERE tenant_id = $1 AND is_active = TRUE ORDER BY name",
        )
        .bind(tenant_id)
        .fetch_all(pool)
        .await?
    };
    Ok(rows)
}

pub async fn get_group(
    pool: &PgPool,
    tenant_id: &str,
    id: Uuid,
) -> Result<Group, ConfigError> {
    sqlx::query_as::<_, Group>(
        "SELECT * FROM csl_groups WHERE id = $1 AND tenant_id = $2",
    )
    .bind(id)
    .bind(tenant_id)
    .fetch_optional(pool)
    .await?
    .ok_or(ConfigError::GroupNotFound(id))
}

pub async fn update_group(
    pool: &PgPool,
    tenant_id: &str,
    id: Uuid,
    req: &UpdateGroupRequest,
) -> Result<Group, ConfigError> {
    if let Some(ref name) = req.name {
        validate_not_blank(name, "name")?;
    }
    if let Some(ref cur) = req.reporting_currency {
        validate_currency(cur)?;
    }
    if let Some(m) = req.fiscal_year_end_month {
        validate_fiscal_month(m)?;
    }

    let row = sqlx::query_as::<_, Group>(
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
    .await?
    .ok_or(ConfigError::GroupNotFound(id))?;
    Ok(row)
}

pub async fn delete_group(
    pool: &PgPool,
    tenant_id: &str,
    id: Uuid,
) -> Result<(), ConfigError> {
    let result = sqlx::query(
        "DELETE FROM csl_groups WHERE id = $1 AND tenant_id = $2",
    )
    .bind(id)
    .bind(tenant_id)
    .execute(pool)
    .await?;
    if result.rows_affected() == 0 {
        return Err(ConfigError::GroupNotFound(id));
    }
    Ok(())
}

// ============================================================================
// Group entities
// ============================================================================

pub async fn create_entity(
    pool: &PgPool,
    tenant_id: &str,
    group_id: Uuid,
    req: &CreateEntityRequest,
) -> Result<GroupEntity, ConfigError> {
    get_group(pool, tenant_id, group_id).await?;
    validate_not_blank(&req.entity_tenant_id, "entity_tenant_id")?;
    validate_not_blank(&req.entity_name, "entity_name")?;
    validate_currency(&req.functional_currency)?;
    let bp = req.ownership_pct_bp.unwrap_or(10000);
    validate_ownership_bp(bp)?;
    let method = req.consolidation_method.as_deref().unwrap_or("full");
    validate_consolidation_method(method)?;

    let row = sqlx::query_as::<_, GroupEntity>(
        "INSERT INTO csl_group_entities
            (group_id, entity_tenant_id, entity_name, functional_currency, ownership_pct_bp, consolidation_method)
         VALUES ($1, $2, $3, $4, $5, $6)
         RETURNING *",
    )
    .bind(group_id)
    .bind(&req.entity_tenant_id)
    .bind(&req.entity_name)
    .bind(&req.functional_currency)
    .bind(bp)
    .bind(method)
    .fetch_one(pool)
    .await
    .map_err(|e| match &e {
        sqlx::Error::Database(db) if db.constraint() == Some("csl_group_entities_unique") => {
            ConfigError::Conflict(format!(
                "Entity '{}' already in group",
                req.entity_tenant_id
            ))
        }
        _ => ConfigError::Database(e),
    })?;
    Ok(row)
}

pub async fn list_entities(
    pool: &PgPool,
    tenant_id: &str,
    group_id: Uuid,
    include_inactive: bool,
) -> Result<Vec<GroupEntity>, ConfigError> {
    get_group(pool, tenant_id, group_id).await?;

    let rows = if include_inactive {
        sqlx::query_as::<_, GroupEntity>(
            "SELECT * FROM csl_group_entities WHERE group_id = $1 ORDER BY entity_name",
        )
        .bind(group_id)
        .fetch_all(pool)
        .await?
    } else {
        sqlx::query_as::<_, GroupEntity>(
            "SELECT * FROM csl_group_entities WHERE group_id = $1 AND is_active = TRUE ORDER BY entity_name",
        )
        .bind(group_id)
        .fetch_all(pool)
        .await?
    };
    Ok(rows)
}

pub async fn get_entity(pool: &PgPool, id: Uuid) -> Result<GroupEntity, ConfigError> {
    sqlx::query_as::<_, GroupEntity>("SELECT * FROM csl_group_entities WHERE id = $1")
        .bind(id)
        .fetch_optional(pool)
        .await?
        .ok_or(ConfigError::EntityNotFound(id))
}

pub async fn update_entity(
    pool: &PgPool,
    tenant_id: &str,
    id: Uuid,
    req: &UpdateEntityRequest,
) -> Result<GroupEntity, ConfigError> {
    let existing = get_entity(pool, id).await?;
    get_group(pool, tenant_id, existing.group_id).await?;

    if let Some(ref name) = req.entity_name {
        validate_not_blank(name, "entity_name")?;
    }
    if let Some(ref cur) = req.functional_currency {
        validate_currency(cur)?;
    }
    if let Some(bp) = req.ownership_pct_bp {
        validate_ownership_bp(bp)?;
    }
    if let Some(ref m) = req.consolidation_method {
        validate_consolidation_method(m)?;
    }

    let row = sqlx::query_as::<_, GroupEntity>(
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
    .await?;
    Ok(row)
}

pub async fn delete_entity(
    pool: &PgPool,
    tenant_id: &str,
    id: Uuid,
) -> Result<(), ConfigError> {
    let existing = get_entity(pool, id).await?;
    get_group(pool, tenant_id, existing.group_id).await?;

    sqlx::query("DELETE FROM csl_group_entities WHERE id = $1")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

// ============================================================================
// COA mappings
// ============================================================================

pub async fn create_coa_mapping(
    pool: &PgPool,
    tenant_id: &str,
    group_id: Uuid,
    req: &CreateCoaMappingRequest,
) -> Result<CoaMapping, ConfigError> {
    get_group(pool, tenant_id, group_id).await?;
    validate_not_blank(&req.entity_tenant_id, "entity_tenant_id")?;
    validate_not_blank(&req.source_account_code, "source_account_code")?;
    validate_not_blank(&req.target_account_code, "target_account_code")?;

    let row = sqlx::query_as::<_, CoaMapping>(
        "INSERT INTO csl_coa_mappings
            (group_id, entity_tenant_id, source_account_code, target_account_code, target_account_name)
         VALUES ($1, $2, $3, $4, $5)
         RETURNING *",
    )
    .bind(group_id)
    .bind(&req.entity_tenant_id)
    .bind(&req.source_account_code)
    .bind(&req.target_account_code)
    .bind(&req.target_account_name)
    .fetch_one(pool)
    .await
    .map_err(|e| match &e {
        sqlx::Error::Database(db) if db.constraint() == Some("csl_coa_mappings_unique") => {
            ConfigError::Conflict(format!(
                "Mapping for {}/{} already exists",
                req.entity_tenant_id, req.source_account_code
            ))
        }
        _ => ConfigError::Database(e),
    })?;
    Ok(row)
}

pub async fn list_coa_mappings(
    pool: &PgPool,
    tenant_id: &str,
    group_id: Uuid,
    entity_tenant_id: Option<&str>,
) -> Result<Vec<CoaMapping>, ConfigError> {
    get_group(pool, tenant_id, group_id).await?;

    let rows = if let Some(eid) = entity_tenant_id {
        sqlx::query_as::<_, CoaMapping>(
            "SELECT * FROM csl_coa_mappings WHERE group_id = $1 AND entity_tenant_id = $2
             ORDER BY source_account_code",
        )
        .bind(group_id)
        .bind(eid)
        .fetch_all(pool)
        .await?
    } else {
        sqlx::query_as::<_, CoaMapping>(
            "SELECT * FROM csl_coa_mappings WHERE group_id = $1
             ORDER BY entity_tenant_id, source_account_code",
        )
        .bind(group_id)
        .fetch_all(pool)
        .await?
    };
    Ok(rows)
}

pub async fn delete_coa_mapping(
    pool: &PgPool,
    tenant_id: &str,
    id: Uuid,
) -> Result<(), ConfigError> {
    let mapping = sqlx::query_as::<_, CoaMapping>(
        "SELECT * FROM csl_coa_mappings WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(pool)
    .await?
    .ok_or(ConfigError::MappingNotFound(id))?;

    get_group(pool, tenant_id, mapping.group_id).await?;

    sqlx::query("DELETE FROM csl_coa_mappings WHERE id = $1")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

// ============================================================================
// Completeness validation
// ============================================================================

pub async fn validate_group_completeness(
    pool: &PgPool,
    tenant_id: &str,
    group_id: Uuid,
) -> Result<ValidationResult, ConfigError> {
    let group = get_group(pool, tenant_id, group_id).await?;
    let entities = list_entities(pool, tenant_id, group_id, false).await?;

    let mut missing_coa = Vec::new();
    let mut missing_fx = Vec::new();

    for entity in &entities {
        let count: (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM csl_coa_mappings
             WHERE group_id = $1 AND entity_tenant_id = $2",
        )
        .bind(group_id)
        .bind(&entity.entity_tenant_id)
        .fetch_one(pool)
        .await?;

        if count.0 == 0 {
            missing_coa.push(entity.entity_tenant_id.clone());
        }

        if entity.functional_currency != group.reporting_currency {
            let fx_count: (i64,) = sqlx::query_as(
                "SELECT COUNT(*) FROM csl_fx_policies
                 WHERE group_id = $1 AND entity_tenant_id = $2",
            )
            .bind(group_id)
            .bind(&entity.entity_tenant_id)
            .fetch_one(pool)
            .await?;

            if fx_count.0 == 0 {
                missing_fx.push(entity.entity_tenant_id.clone());
            }
        }
    }

    Ok(ValidationResult {
        is_complete: missing_coa.is_empty() && missing_fx.is_empty(),
        missing_coa_mappings: missing_coa,
        missing_fx_policies: missing_fx,
    })
}
