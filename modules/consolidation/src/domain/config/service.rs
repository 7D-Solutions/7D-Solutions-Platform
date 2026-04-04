//! Service layer for consolidation config CRUD — groups, entities, COA mappings.
//!
//! All queries are tenant-scoped (via group.tenant_id).
//! Audit fields (created_at, updated_at) are managed by DB defaults.

use sqlx::PgPool;
use uuid::Uuid;

use super::{
    models::*, repo, validate_consolidation_method, validate_currency, validate_fiscal_month,
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

    let row = repo::insert_group(
        pool,
        tenant_id,
        &req.name,
        &req.description,
        &req.reporting_currency,
        month,
    )
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
    Ok(repo::fetch_groups(pool, tenant_id, include_inactive).await?)
}

pub async fn get_group(pool: &PgPool, tenant_id: &str, id: Uuid) -> Result<Group, ConfigError> {
    repo::fetch_group(pool, tenant_id, id)
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

    let row = repo::update_group_row(pool, id, tenant_id, req)
        .await?
        .ok_or(ConfigError::GroupNotFound(id))?;
    Ok(row)
}

pub async fn delete_group(pool: &PgPool, tenant_id: &str, id: Uuid) -> Result<(), ConfigError> {
    let rows_affected = repo::delete_group_row(pool, tenant_id, id).await?;
    if rows_affected == 0 {
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

    let row = repo::insert_entity(
        pool,
        group_id,
        &req.entity_tenant_id,
        &req.entity_name,
        &req.functional_currency,
        bp,
        method,
    )
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
    Ok(repo::fetch_entities(pool, tenant_id, group_id, include_inactive).await?)
}

pub async fn get_entity(pool: &PgPool, id: Uuid) -> Result<GroupEntity, ConfigError> {
    repo::fetch_entity(pool, id)
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

    let row = repo::update_entity_row(pool, id, req).await?;
    Ok(row)
}

pub async fn delete_entity(pool: &PgPool, tenant_id: &str, id: Uuid) -> Result<(), ConfigError> {
    let existing = get_entity(pool, id).await?;
    get_group(pool, tenant_id, existing.group_id).await?;
    repo::delete_entity_row(pool, id).await?;
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

    let row = repo::insert_coa_mapping(
        pool,
        group_id,
        &req.entity_tenant_id,
        &req.source_account_code,
        &req.target_account_code,
        &req.target_account_name,
    )
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
    Ok(repo::fetch_coa_mappings(pool, group_id, entity_tenant_id).await?)
}

pub async fn delete_coa_mapping(
    pool: &PgPool,
    tenant_id: &str,
    id: Uuid,
) -> Result<(), ConfigError> {
    let mapping = repo::fetch_coa_mapping(pool, id)
        .await?
        .ok_or(ConfigError::MappingNotFound(id))?;

    get_group(pool, tenant_id, mapping.group_id).await?;
    repo::delete_coa_mapping_row(pool, id).await?;
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
        let count = repo::count_coa_mappings(pool, group_id, &entity.entity_tenant_id).await?;

        if count == 0 {
            missing_coa.push(entity.entity_tenant_id.clone());
        }

        if entity.functional_currency != group.reporting_currency {
            let fx_count =
                repo::count_fx_policies(pool, group_id, &entity.entity_tenant_id).await?;

            if fx_count == 0 {
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
