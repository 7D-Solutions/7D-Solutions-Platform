//! Domain models for consolidation configuration.
//!
//! Maps directly to csl_* tables from the consolidation schema.
//! All config is scoped to (tenant_id, group_id).

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

// ============================================================================
// Row types (read from DB)
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow, ToSchema)]
pub struct Group {
    pub id: Uuid,
    pub tenant_id: String,
    pub name: String,
    pub description: Option<String>,
    pub reporting_currency: String,
    pub fiscal_year_end_month: i16,
    pub is_active: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow, ToSchema)]
pub struct GroupEntity {
    pub id: Uuid,
    pub group_id: Uuid,
    pub entity_tenant_id: String,
    pub entity_name: String,
    pub functional_currency: String,
    pub ownership_pct_bp: i32,
    pub consolidation_method: String,
    pub is_active: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow, ToSchema)]
pub struct CoaMapping {
    pub id: Uuid,
    pub group_id: Uuid,
    pub entity_tenant_id: String,
    pub source_account_code: String,
    pub target_account_code: String,
    pub target_account_name: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow, ToSchema)]
pub struct EliminationRule {
    pub id: Uuid,
    pub group_id: Uuid,
    pub rule_name: String,
    pub rule_type: String,
    pub debit_account_code: String,
    pub credit_account_code: String,
    pub description: Option<String>,
    pub is_active: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow, ToSchema)]
pub struct FxPolicy {
    pub id: Uuid,
    pub group_id: Uuid,
    pub entity_tenant_id: String,
    pub bs_rate_type: String,
    pub pl_rate_type: String,
    pub equity_rate_type: String,
    pub fx_rate_source: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

// ============================================================================
// Create request types
// ============================================================================

#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct CreateGroupRequest {
    pub name: String,
    pub description: Option<String>,
    pub reporting_currency: String,
    pub fiscal_year_end_month: Option<i16>,
}

#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct CreateEntityRequest {
    pub entity_tenant_id: String,
    pub entity_name: String,
    pub functional_currency: String,
    pub ownership_pct_bp: Option<i32>,
    pub consolidation_method: Option<String>,
}

#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct CreateCoaMappingRequest {
    pub entity_tenant_id: String,
    pub source_account_code: String,
    pub target_account_code: String,
    pub target_account_name: Option<String>,
}

#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct CreateEliminationRuleRequest {
    pub rule_name: String,
    pub rule_type: String,
    pub debit_account_code: String,
    pub credit_account_code: String,
    pub description: Option<String>,
}

#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct UpsertFxPolicyRequest {
    pub entity_tenant_id: String,
    pub bs_rate_type: Option<String>,
    pub pl_rate_type: Option<String>,
    pub equity_rate_type: Option<String>,
    pub fx_rate_source: Option<String>,
}

// ============================================================================
// Update request types
// ============================================================================

#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct UpdateGroupRequest {
    pub name: Option<String>,
    pub description: Option<String>,
    pub reporting_currency: Option<String>,
    pub fiscal_year_end_month: Option<i16>,
    pub is_active: Option<bool>,
}

#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct UpdateEntityRequest {
    pub entity_name: Option<String>,
    pub functional_currency: Option<String>,
    pub ownership_pct_bp: Option<i32>,
    pub consolidation_method: Option<String>,
    pub is_active: Option<bool>,
}

#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct UpdateEliminationRuleRequest {
    pub rule_name: Option<String>,
    pub rule_type: Option<String>,
    pub debit_account_code: Option<String>,
    pub credit_account_code: Option<String>,
    pub description: Option<String>,
    pub is_active: Option<bool>,
}

// ============================================================================
// Query params
// ============================================================================

#[derive(Debug, Clone, Deserialize, IntoParams, ToSchema)]
#[into_params(parameter_in = Query)]
pub struct ListGroupsQuery {
    #[serde(default)]
    pub include_inactive: bool,
}

#[derive(Debug, Clone, Deserialize, IntoParams, ToSchema)]
#[into_params(parameter_in = Query)]
pub struct ListEntitiesQuery {
    #[serde(default)]
    pub include_inactive: bool,
}

#[derive(Debug, Clone, Deserialize, IntoParams, ToSchema)]
#[into_params(parameter_in = Query)]
pub struct ListCoaMappingsQuery {
    pub entity_tenant_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize, IntoParams, ToSchema)]
#[into_params(parameter_in = Query)]
pub struct ListEliminationRulesQuery {
    #[serde(default)]
    pub include_inactive: bool,
}

// ============================================================================
// Validation result
// ============================================================================

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ValidationResult {
    pub is_complete: bool,
    pub missing_coa_mappings: Vec<String>,
    pub missing_fx_policies: Vec<String>,
}
