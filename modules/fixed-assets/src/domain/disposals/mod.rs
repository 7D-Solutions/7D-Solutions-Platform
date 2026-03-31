//! Domain models for asset disposals and impairments.
//!
//! All monetary values are i64 minor units + TEXT currency.
//! Tenant-scoped: every query filters by tenant_id.

pub mod service;

pub use service::DisposalService;

use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum DisposalType {
    Sale,
    Scrap,
    Impairment,
    WriteOff,
    Transfer,
}

impl DisposalType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Sale => "sale",
            Self::Scrap => "scrap",
            Self::Impairment => "impairment",
            Self::WriteOff => "write_off",
            Self::Transfer => "transfer",
        }
    }

    /// Target asset status after this disposal type.
    pub fn target_status(&self) -> &'static str {
        match self {
            Self::Impairment => "impaired",
            _ => "disposed",
        }
    }
}

impl TryFrom<String> for DisposalType {
    type Error = String;
    fn try_from(s: String) -> Result<Self, Self::Error> {
        match s.as_str() {
            "sale" => Ok(Self::Sale),
            "scrap" => Ok(Self::Scrap),
            "impairment" => Ok(Self::Impairment),
            "write_off" => Ok(Self::WriteOff),
            "transfer" => Ok(Self::Transfer),
            other => Err(format!("invalid disposal_type '{}'", other)),
        }
    }
}

impl std::fmt::Display for DisposalType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Row from fa_disposals (disposal_type cast to TEXT in queries).
#[derive(Debug, Clone, Serialize, sqlx::FromRow, ToSchema)]
pub struct Disposal {
    pub id: Uuid,
    pub tenant_id: String,
    pub asset_id: Uuid,
    pub disposal_type: String,
    pub disposal_date: NaiveDate,
    pub net_book_value_at_disposal_minor: i64,
    pub proceeds_minor: i64,
    pub gain_loss_minor: i64,
    pub currency: String,
    pub reason: Option<String>,
    pub buyer: Option<String>,
    pub reference: Option<String>,
    pub journal_entry_ref: Option<String>,
    pub is_posted: bool,
    pub posted_at: Option<DateTime<Utc>>,
    pub created_by: Option<String>,
    pub approved_by: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Request to dispose of or impair an asset.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct DisposeAssetRequest {
    pub tenant_id: String,
    pub asset_id: Uuid,
    pub disposal_type: DisposalType,
    pub disposal_date: NaiveDate,
    /// Sale proceeds (only meaningful for disposal_type = sale). Defaults to 0.
    pub proceeds_minor: Option<i64>,
    pub reason: Option<String>,
    pub buyer: Option<String>,
    pub reference: Option<String>,
    pub created_by: Option<String>,
}

impl DisposeAssetRequest {
    pub fn validate(&self) -> Result<(), DisposalError> {
        if self.tenant_id.trim().is_empty() {
            return Err(DisposalError::Validation("tenant_id required".into()));
        }
        if let Some(p) = self.proceeds_minor {
            if p < 0 {
                return Err(DisposalError::Validation(
                    "proceeds_minor must be non-negative".into(),
                ));
            }
        }
        Ok(())
    }
}

/// GL entry data carried in the outbox event for the GL consumer.
#[derive(Debug, Serialize)]
pub struct DisposalGlData {
    pub disposal_id: Uuid,
    pub asset_id: Uuid,
    pub disposal_type: String,
    pub disposal_date: NaiveDate,
    pub acquisition_cost_minor: i64,
    pub accum_depreciation_minor: i64,
    pub net_book_value_minor: i64,
    pub proceeds_minor: i64,
    pub gain_loss_minor: i64,
    pub currency: String,
    pub asset_account_ref: String,
    pub accum_depreciation_ref: String,
    pub gain_loss_account_ref: Option<String>,
}

/// Outbox event emitted when an asset is disposed or impaired.
#[derive(Debug, Serialize)]
pub struct AssetDisposedEvent {
    pub disposal_id: Uuid,
    pub asset_id: Uuid,
    pub tenant_id: String,
    pub disposal_type: String,
    pub disposal_date: NaiveDate,
    pub gl_data: DisposalGlData,
}

#[derive(Debug, thiserror::Error)]
pub enum DisposalError {
    #[error("Asset not found: {0}")]
    AssetNotFound(Uuid),
    #[error("Category not found for asset: {0}")]
    CategoryNotFound(Uuid),
    #[error("Invalid state for disposal: {0}")]
    InvalidState(String),
    #[error("Validation error: {0}")]
    Validation(String),
    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),
}

impl From<DisposalError> for platform_http_contracts::ApiError {
    fn from(err: DisposalError) -> Self {
        match &err {
            DisposalError::AssetNotFound(id) => {
                Self::not_found(format!("Asset {} not found", id))
            }
            DisposalError::CategoryNotFound(id) => {
                Self::not_found(format!("Category not found for asset {}", id))
            }
            DisposalError::InvalidState(msg) => Self::conflict(msg.clone()),
            DisposalError::Validation(msg) => Self::new(422, "validation_error", msg.clone()),
            DisposalError::Database(e) => {
                tracing::error!("Fixed-assets disposal DB error: {}", e);
                Self::internal("Internal database error")
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disposal_type_roundtrip() {
        assert_eq!(
            DisposalType::try_from("sale".to_string()),
            Ok(DisposalType::Sale)
        );
        assert_eq!(
            DisposalType::try_from("scrap".to_string()),
            Ok(DisposalType::Scrap)
        );
        assert_eq!(
            DisposalType::try_from("impairment".to_string()),
            Ok(DisposalType::Impairment)
        );
        assert_eq!(
            DisposalType::try_from("write_off".to_string()),
            Ok(DisposalType::WriteOff)
        );
        assert_eq!(
            DisposalType::try_from("transfer".to_string()),
            Ok(DisposalType::Transfer)
        );
        assert!(DisposalType::try_from("invalid".to_string()).is_err());
    }

    #[test]
    fn target_status_correct() {
        assert_eq!(DisposalType::Sale.target_status(), "disposed");
        assert_eq!(DisposalType::Scrap.target_status(), "disposed");
        assert_eq!(DisposalType::Impairment.target_status(), "impaired");
        assert_eq!(DisposalType::WriteOff.target_status(), "disposed");
        assert_eq!(DisposalType::Transfer.target_status(), "disposed");
    }

    #[test]
    fn validation_rejects_empty_tenant() {
        let req = DisposeAssetRequest {
            tenant_id: " ".into(),
            asset_id: Uuid::new_v4(),
            disposal_type: DisposalType::Sale,
            disposal_date: NaiveDate::from_ymd_opt(2026, 6, 30).expect("valid date"),
            proceeds_minor: None,
            reason: None,
            buyer: None,
            reference: None,
            created_by: None,
        };
        assert!(matches!(req.validate(), Err(DisposalError::Validation(_))));
    }

    #[test]
    fn validation_rejects_negative_proceeds() {
        let req = DisposeAssetRequest {
            tenant_id: "t1".into(),
            asset_id: Uuid::new_v4(),
            disposal_type: DisposalType::Sale,
            disposal_date: NaiveDate::from_ymd_opt(2026, 6, 30).expect("valid date"),
            proceeds_minor: Some(-100),
            reason: None,
            buyer: None,
            reference: None,
            created_by: None,
        };
        assert!(matches!(req.validate(), Err(DisposalError::Validation(_))));
    }

    #[test]
    fn validation_accepts_valid() {
        let req = DisposeAssetRequest {
            tenant_id: "t1".into(),
            asset_id: Uuid::new_v4(),
            disposal_type: DisposalType::Sale,
            disposal_date: NaiveDate::from_ymd_opt(2026, 6, 30).expect("valid date"),
            proceeds_minor: Some(5000),
            reason: Some("Obsolete".into()),
            buyer: Some("Acme".into()),
            reference: None,
            created_by: None,
        };
        assert!(req.validate().is_ok());
    }
}
