//! Domain models for Fixed Assets: categories and asset register.
//!
//! All monetary values are i64 minor units + currency text.
//! Tenant-scoped: every query filters by tenant_id.

use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DepreciationMethod {
    StraightLine,
    DecliningBalance,
    UnitsOfProduction,
    None,
}

impl DepreciationMethod {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::StraightLine => "straight_line",
            Self::DecliningBalance => "declining_balance",
            Self::UnitsOfProduction => "units_of_production",
            Self::None => "none",
        }
    }
}

impl TryFrom<String> for DepreciationMethod {
    type Error = String;
    fn try_from(s: String) -> Result<Self, Self::Error> {
        match s.as_str() {
            "straight_line" => Ok(Self::StraightLine),
            "declining_balance" => Ok(Self::DecliningBalance),
            "units_of_production" => Ok(Self::UnitsOfProduction),
            "none" => Ok(Self::None),
            other => Err(format!("invalid depreciation_method '{}'", other)),
        }
    }
}

impl std::fmt::Display for DepreciationMethod {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AssetStatus {
    Draft,
    Active,
    FullyDepreciated,
    Disposed,
    Impaired,
}

impl AssetStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Draft => "draft",
            Self::Active => "active",
            Self::FullyDepreciated => "fully_depreciated",
            Self::Disposed => "disposed",
            Self::Impaired => "impaired",
        }
    }
}

impl TryFrom<String> for AssetStatus {
    type Error = String;
    fn try_from(s: String) -> Result<Self, Self::Error> {
        match s.as_str() {
            "draft" => Ok(Self::Draft),
            "active" => Ok(Self::Active),
            "fully_depreciated" => Ok(Self::FullyDepreciated),
            "disposed" => Ok(Self::Disposed),
            "impaired" => Ok(Self::Impaired),
            other => Err(format!("invalid asset_status '{}'", other)),
        }
    }
}

impl std::fmt::Display for AssetStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Category {
    pub id: Uuid,
    pub tenant_id: String,
    pub code: String,
    pub name: String,
    pub description: Option<String>,
    pub default_method: String,
    pub default_useful_life_months: i32,
    pub default_salvage_pct_bp: i32,
    pub asset_account_ref: String,
    pub depreciation_expense_ref: String,
    pub accum_depreciation_ref: String,
    pub gain_loss_account_ref: Option<String>,
    pub is_active: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Asset {
    pub id: Uuid,
    pub tenant_id: String,
    pub category_id: Uuid,
    pub asset_tag: String,
    pub name: String,
    pub description: Option<String>,
    pub status: String,
    pub acquisition_date: NaiveDate,
    pub in_service_date: Option<NaiveDate>,
    pub acquisition_cost_minor: i64,
    pub currency: String,
    pub depreciation_method: String,
    pub useful_life_months: i32,
    pub salvage_value_minor: i64,
    pub accum_depreciation_minor: i64,
    pub net_book_value_minor: i64,
    pub asset_account_ref: Option<String>,
    pub depreciation_expense_ref: Option<String>,
    pub accum_depreciation_ref: Option<String>,
    pub location: Option<String>,
    pub department: Option<String>,
    pub responsible_person: Option<String>,
    pub serial_number: Option<String>,
    pub vendor: Option<String>,
    pub purchase_order_ref: Option<String>,
    pub notes: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
pub struct CreateCategoryRequest {
    pub tenant_id: String,
    pub code: String,
    pub name: String,
    pub description: Option<String>,
    pub default_method: Option<DepreciationMethod>,
    pub default_useful_life_months: Option<i32>,
    pub default_salvage_pct_bp: Option<i32>,
    pub asset_account_ref: String,
    pub depreciation_expense_ref: String,
    pub accum_depreciation_ref: String,
    pub gain_loss_account_ref: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateCategoryRequest {
    pub tenant_id: String,
    pub name: Option<String>,
    pub description: Option<String>,
    pub default_method: Option<DepreciationMethod>,
    pub default_useful_life_months: Option<i32>,
    pub default_salvage_pct_bp: Option<i32>,
    pub asset_account_ref: Option<String>,
    pub depreciation_expense_ref: Option<String>,
    pub accum_depreciation_ref: Option<String>,
    pub gain_loss_account_ref: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct CreateAssetRequest {
    pub tenant_id: String,
    pub category_id: Uuid,
    pub asset_tag: String,
    pub name: String,
    pub description: Option<String>,
    pub acquisition_date: NaiveDate,
    pub in_service_date: Option<NaiveDate>,
    pub acquisition_cost_minor: i64,
    pub currency: Option<String>,
    pub depreciation_method: Option<DepreciationMethod>,
    pub useful_life_months: Option<i32>,
    pub salvage_value_minor: Option<i64>,
    pub location: Option<String>,
    pub department: Option<String>,
    pub responsible_person: Option<String>,
    pub serial_number: Option<String>,
    pub vendor: Option<String>,
    pub purchase_order_ref: Option<String>,
    pub notes: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateAssetRequest {
    pub tenant_id: String,
    pub name: Option<String>,
    pub description: Option<String>,
    pub location: Option<String>,
    pub department: Option<String>,
    pub responsible_person: Option<String>,
    pub notes: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct AssetCreatedEvent {
    pub asset_id: Uuid,
    pub tenant_id: String,
    pub asset_tag: String,
    pub category_id: Uuid,
    pub acquisition_cost_minor: i64,
    pub currency: String,
}

#[derive(Debug, Serialize)]
pub struct AssetUpdatedEvent {
    pub asset_id: Uuid,
    pub tenant_id: String,
}

#[derive(Debug, Serialize)]
pub struct AssetDeactivatedEvent {
    pub asset_id: Uuid,
    pub tenant_id: String,
    pub previous_status: String,
}

#[derive(Debug, Serialize)]
pub struct CategoryCreatedEvent {
    pub category_id: Uuid,
    pub tenant_id: String,
    pub code: String,
}

#[derive(Debug, thiserror::Error)]
pub enum AssetError {
    #[error("Asset tag '{0}' already exists for tenant '{1}'")]
    DuplicateTag(String, String),
    #[error("Category code '{0}' already exists for tenant '{1}'")]
    DuplicateCategoryCode(String, String),
    #[error("Not found")]
    NotFound,
    #[error("Category not found: {0}")]
    CategoryNotFound(Uuid),
    #[error("Validation error: {0}")]
    Validation(String),
    #[error("Invalid state transition: {0}")]
    InvalidTransition(String),
    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),
}

fn require_non_empty(value: &str, field: &str) -> Result<(), AssetError> {
    if value.trim().is_empty() {
        return Err(AssetError::Validation(format!("{} must not be empty", field)));
    }
    Ok(())
}

fn validate_life_months(months: Option<i32>) -> Result<(), AssetError> {
    if let Some(m) = months {
        if m <= 0 {
            return Err(AssetError::Validation("useful_life_months must be positive".into()));
        }
    }
    Ok(())
}

fn validate_salvage_bp(bp: Option<i32>) -> Result<(), AssetError> {
    if let Some(b) = bp {
        if !(0..=10000).contains(&b) {
            return Err(AssetError::Validation("salvage_pct_bp must be 0-10000".into()));
        }
    }
    Ok(())
}

impl CreateCategoryRequest {
    pub fn validate(&self) -> Result<(), AssetError> {
        require_non_empty(&self.tenant_id, "tenant_id")?;
        require_non_empty(&self.code, "code")?;
        require_non_empty(&self.name, "name")?;
        require_non_empty(&self.asset_account_ref, "asset_account_ref")?;
        require_non_empty(&self.depreciation_expense_ref, "depreciation_expense_ref")?;
        require_non_empty(&self.accum_depreciation_ref, "accum_depreciation_ref")?;
        validate_life_months(self.default_useful_life_months)?;
        validate_salvage_bp(self.default_salvage_pct_bp)
    }
}

impl UpdateCategoryRequest {
    pub fn validate(&self) -> Result<(), AssetError> {
        require_non_empty(&self.tenant_id, "tenant_id")?;
        if let Some(ref n) = self.name { require_non_empty(n, "name")?; }
        if let Some(ref r) = self.asset_account_ref { require_non_empty(r, "asset_account_ref")?; }
        if let Some(ref r) = self.depreciation_expense_ref { require_non_empty(r, "depreciation_expense_ref")?; }
        if let Some(ref r) = self.accum_depreciation_ref { require_non_empty(r, "accum_depreciation_ref")?; }
        validate_life_months(self.default_useful_life_months)?;
        validate_salvage_bp(self.default_salvage_pct_bp)
    }
}

impl CreateAssetRequest {
    pub fn validate(&self) -> Result<(), AssetError> {
        require_non_empty(&self.tenant_id, "tenant_id")?;
        require_non_empty(&self.asset_tag, "asset_tag")?;
        require_non_empty(&self.name, "name")?;
        if self.acquisition_cost_minor < 0 {
            return Err(AssetError::Validation("acquisition_cost_minor must be non-negative".into()));
        }
        if let Some(s) = self.salvage_value_minor {
            if s < 0 {
                return Err(AssetError::Validation("salvage_value_minor must be non-negative".into()));
            }
        }
        validate_life_months(self.useful_life_months)
    }
}

impl UpdateAssetRequest {
    pub fn validate(&self) -> Result<(), AssetError> {
        require_non_empty(&self.tenant_id, "tenant_id")?;
        if let Some(ref n) = self.name { require_non_empty(n, "name")?; }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_create_category() -> CreateCategoryRequest {
        CreateCategoryRequest {
            tenant_id: "t1".into(), code: "FURN".into(), name: "Furniture".into(),
            description: None, default_method: None,
            default_useful_life_months: Some(60), default_salvage_pct_bp: Some(500),
            asset_account_ref: "1500".into(), depreciation_expense_ref: "6100".into(),
            accum_depreciation_ref: "1510".into(), gain_loss_account_ref: None,
        }
    }

    fn valid_create_asset() -> CreateAssetRequest {
        CreateAssetRequest {
            tenant_id: "t1".into(), category_id: Uuid::new_v4(),
            asset_tag: "FA-001".into(), name: "Office Desk".into(), description: None,
            acquisition_date: NaiveDate::from_ymd_opt(2026, 1, 15).unwrap(),
            in_service_date: None, acquisition_cost_minor: 50000, currency: None,
            depreciation_method: None, useful_life_months: None, salvage_value_minor: None,
            location: None, department: None, responsible_person: None,
            serial_number: None, vendor: None, purchase_order_ref: None, notes: None,
        }
    }

    #[test]
    fn category_create_valid() { assert!(valid_create_category().validate().is_ok()); }

    #[test]
    fn category_empty_code_rejected() {
        let mut r = valid_create_category(); r.code = "  ".into();
        assert!(matches!(r.validate(), Err(AssetError::Validation(_))));
    }

    #[test]
    fn category_negative_life_rejected() {
        let mut r = valid_create_category(); r.default_useful_life_months = Some(-1);
        assert!(matches!(r.validate(), Err(AssetError::Validation(_))));
    }

    #[test]
    fn category_salvage_bp_overflow_rejected() {
        let mut r = valid_create_category(); r.default_salvage_pct_bp = Some(10001);
        assert!(matches!(r.validate(), Err(AssetError::Validation(_))));
    }

    #[test]
    fn asset_create_valid() { assert!(valid_create_asset().validate().is_ok()); }

    #[test]
    fn asset_negative_cost_rejected() {
        let mut r = valid_create_asset(); r.acquisition_cost_minor = -100;
        assert!(matches!(r.validate(), Err(AssetError::Validation(_))));
    }

    #[test]
    fn asset_empty_tag_rejected() {
        let mut r = valid_create_asset(); r.asset_tag = "".into();
        assert!(matches!(r.validate(), Err(AssetError::Validation(_))));
    }

    #[test]
    fn depreciation_method_roundtrip() {
        assert_eq!(DepreciationMethod::try_from("straight_line".to_string()), Ok(DepreciationMethod::StraightLine));
        assert_eq!(DepreciationMethod::try_from("declining_balance".to_string()), Ok(DepreciationMethod::DecliningBalance));
        assert_eq!(DepreciationMethod::try_from("none".to_string()), Ok(DepreciationMethod::None));
        assert!(DepreciationMethod::try_from("invalid".to_string()).is_err());
    }

    #[test]
    fn asset_status_roundtrip() {
        assert_eq!(AssetStatus::try_from("draft".to_string()), Ok(AssetStatus::Draft));
        assert_eq!(AssetStatus::try_from("active".to_string()), Ok(AssetStatus::Active));
        assert_eq!(AssetStatus::try_from("disposed".to_string()), Ok(AssetStatus::Disposed));
        assert!(AssetStatus::try_from("invalid".to_string()).is_err());
    }

    #[test]
    fn update_category_valid_empty() {
        let r = UpdateCategoryRequest {
            tenant_id: "t1".into(), name: None, description: None, default_method: None,
            default_useful_life_months: None, default_salvage_pct_bp: None,
            asset_account_ref: None, depreciation_expense_ref: None,
            accum_depreciation_ref: None, gain_loss_account_ref: None,
        };
        assert!(r.validate().is_ok());
    }

    #[test]
    fn update_asset_valid_empty() {
        let r = UpdateAssetRequest {
            tenant_id: "t1".into(), name: None, description: None,
            location: None, department: None, responsible_person: None, notes: None,
        };
        assert!(r.validate().is_ok());
    }
}
