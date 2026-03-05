//! Item master domain model and repository.
//!
//! Invariants enforced here:
//! - SKU is unique per tenant (DB constraint + application guard)
//! - inventory_account_ref, cogs_account_ref, variance_account_ref are required
//! - SKU and name must be non-empty
//! - Deactivate is idempotent (already-inactive items return Ok)
//! - tracking_mode is set on creation and is immutable (none|lot|serial)

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use thiserror::Error;
use uuid::Uuid;

// ============================================================================
// Tracking mode
// ============================================================================

/// How stock movements are tracked for this SKU.
///
/// Set at item creation; immutable thereafter (changing tracking_mode after
/// stock exists would invalidate historical layer associations).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TrackingMode {
    /// No lot/serial tracking. Quantities move freely.
    None,
    /// Stock moves in named lots. lot_code is required on receipt/issue.
    Lot,
    /// Each unit has a unique serial number. serial_codes required on movement.
    /// Serial items must always move in positive integer units.
    Serial,
}

impl TrackingMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Lot => "lot",
            Self::Serial => "serial",
        }
    }
}

impl std::fmt::Display for TrackingMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl TryFrom<String> for TrackingMode {
    type Error = String;

    fn try_from(s: String) -> Result<Self, Self::Error> {
        match s.as_str() {
            "none" => Ok(Self::None),
            "lot" => Ok(Self::Lot),
            "serial" => Ok(Self::Serial),
            other => Err(format!(
                "invalid tracking_mode '{}': expected none|lot|serial",
                other
            )),
        }
    }
}

// ============================================================================
// Domain model
// ============================================================================

/// A unique item (SKU) per tenant with GL account references.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Item {
    pub id: Uuid,
    pub tenant_id: String,
    pub sku: String,
    pub name: String,
    pub description: Option<String>,
    pub inventory_account_ref: String,
    pub cogs_account_ref: String,
    pub variance_account_ref: String,
    pub uom: String,
    /// FK into `uoms` catalog — the item's canonical stock unit.
    /// None until assigned via the UoM catalog.
    pub base_uom_id: Option<Uuid>,
    /// How stock movements are tracked. Immutable after creation.
    #[sqlx(try_from = "String")]
    pub tracking_mode: TrackingMode,
    /// Manufacturing classification: "make" | "buy" (NULL = unset).
    pub make_buy: Option<String>,
    pub active: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

// ============================================================================
// Request types
// ============================================================================

/// Input for POST /api/inventory/items
#[derive(Debug, Deserialize)]
pub struct CreateItemRequest {
    pub tenant_id: String,
    pub sku: String,
    pub name: String,
    pub description: Option<String>,
    /// GL account ref for inventory asset (e.g. "1200")
    pub inventory_account_ref: String,
    /// GL account ref for COGS expense (e.g. "5000")
    pub cogs_account_ref: String,
    /// GL account ref for purchase price variance (e.g. "5010")
    pub variance_account_ref: String,
    /// Unit of measure, defaults to "ea"
    pub uom: Option<String>,
    /// Lot/serial tracking mode. Required; immutable after creation.
    pub tracking_mode: TrackingMode,
    /// Manufacturing classification: "make" | "buy" (optional)
    #[serde(default)]
    pub make_buy: Option<String>,
}

/// Input for PUT /api/inventory/items/:id
#[derive(Debug, Deserialize)]
pub struct UpdateItemRequest {
    pub tenant_id: String,
    pub name: Option<String>,
    pub description: Option<String>,
    pub inventory_account_ref: Option<String>,
    pub cogs_account_ref: Option<String>,
    pub variance_account_ref: Option<String>,
    pub uom: Option<String>,
}

// ============================================================================
// Errors
// ============================================================================

#[derive(Debug, Error)]
pub enum ItemError {
    #[error("SKU '{0}' already exists for tenant '{1}'")]
    DuplicateSku(String, String),

    #[error("Item not found")]
    NotFound,

    #[error("Validation error: {0}")]
    Validation(String),

    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),
}

// ============================================================================
// Validation helpers
// ============================================================================

fn require_non_empty(value: &str, field: &str) -> Result<(), ItemError> {
    if value.trim().is_empty() {
        return Err(ItemError::Validation(format!(
            "{} must not be empty",
            field
        )));
    }
    Ok(())
}

impl CreateItemRequest {
    pub fn validate(&self) -> Result<(), ItemError> {
        require_non_empty(&self.tenant_id, "tenant_id")?;
        require_non_empty(&self.sku, "sku")?;
        require_non_empty(&self.name, "name")?;
        require_non_empty(&self.inventory_account_ref, "inventory_account_ref")?;
        require_non_empty(&self.cogs_account_ref, "cogs_account_ref")?;
        require_non_empty(&self.variance_account_ref, "variance_account_ref")?;
        if let Some(ref uom) = self.uom {
            require_non_empty(uom, "uom")?;
        }
        super::make_buy::validate_make_buy(&self.make_buy)?;
        Ok(())
    }
}

impl UpdateItemRequest {
    pub fn validate(&self) -> Result<(), ItemError> {
        require_non_empty(&self.tenant_id, "tenant_id")?;
        if let Some(ref name) = self.name {
            require_non_empty(name, "name")?;
        }
        if let Some(ref r) = self.inventory_account_ref {
            require_non_empty(r, "inventory_account_ref")?;
        }
        if let Some(ref r) = self.cogs_account_ref {
            require_non_empty(r, "cogs_account_ref")?;
        }
        if let Some(ref r) = self.variance_account_ref {
            require_non_empty(r, "variance_account_ref")?;
        }
        if let Some(ref uom) = self.uom {
            require_non_empty(uom, "uom")?;
        }
        Ok(())
    }
}

// ============================================================================
// Repository
// ============================================================================

pub struct ItemRepo;

impl ItemRepo {
    /// Create a new item.
    ///
    /// Guard: validates input, then checks SKU uniqueness via DB constraint.
    /// Returns DuplicateSku if the (tenant_id, sku) pair already exists.
    pub async fn create(pool: &PgPool, req: &CreateItemRequest) -> Result<Item, ItemError> {
        req.validate()?;

        let uom = req.uom.as_deref().unwrap_or("ea");
        let id = Uuid::new_v4();
        let now = Utc::now();

        let item = sqlx::query_as::<_, Item>(
            r#"
            INSERT INTO items
                (id, tenant_id, sku, name, description,
                 inventory_account_ref, cogs_account_ref, variance_account_ref,
                 uom, tracking_mode, make_buy, active, created_at, updated_at)
            VALUES
                ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, TRUE, $12, $12)
            RETURNING *
            "#,
        )
        .bind(id)
        .bind(&req.tenant_id)
        .bind(req.sku.trim())
        .bind(req.name.trim())
        .bind(req.description.as_deref())
        .bind(req.inventory_account_ref.trim())
        .bind(req.cogs_account_ref.trim())
        .bind(req.variance_account_ref.trim())
        .bind(uom)
        .bind(req.tracking_mode.as_str())
        .bind(req.make_buy.as_deref())
        .bind(now)
        .fetch_one(pool)
        .await
        .map_err(|e| {
            if let sqlx::Error::Database(ref dbe) = e {
                // PostgreSQL unique violation = code 23505
                if dbe.code().as_deref() == Some("23505") {
                    return ItemError::DuplicateSku(req.sku.clone(), req.tenant_id.clone());
                }
            }
            ItemError::Database(e)
        })?;

        Ok(item)
    }

    /// Update mutable fields of an existing item.
    ///
    /// Only fields present in the request are updated; missing fields are left unchanged.
    /// Scoped to tenant_id to prevent cross-tenant mutation.
    pub async fn update(
        pool: &PgPool,
        id: Uuid,
        req: &UpdateItemRequest,
    ) -> Result<Item, ItemError> {
        req.validate()?;

        let item = sqlx::query_as::<_, Item>(
            r#"
            UPDATE items
            SET
                name                 = COALESCE($3, name),
                description          = CASE WHEN $4::TEXT IS NOT NULL THEN $4 ELSE description END,
                inventory_account_ref = COALESCE($5, inventory_account_ref),
                cogs_account_ref     = COALESCE($6, cogs_account_ref),
                variance_account_ref = COALESCE($7, variance_account_ref),
                uom                  = COALESCE($8, uom),
                updated_at           = NOW()
            WHERE id = $1
              AND tenant_id = $2
            RETURNING *
            "#,
        )
        .bind(id)
        .bind(&req.tenant_id)
        .bind(req.name.as_deref())
        .bind(req.description.as_deref())
        .bind(req.inventory_account_ref.as_deref())
        .bind(req.cogs_account_ref.as_deref())
        .bind(req.variance_account_ref.as_deref())
        .bind(req.uom.as_deref())
        .fetch_optional(pool)
        .await?
        .ok_or(ItemError::NotFound)?;

        Ok(item)
    }

    /// Deactivate an item (soft delete).
    ///
    /// Idempotent: already-inactive items are returned without error.
    /// Scoped to tenant_id to prevent cross-tenant mutation.
    pub async fn deactivate(pool: &PgPool, id: Uuid, tenant_id: &str) -> Result<Item, ItemError> {
        let item = sqlx::query_as::<_, Item>(
            r#"
            UPDATE items
            SET active = FALSE, updated_at = NOW()
            WHERE id = $1 AND tenant_id = $2
            RETURNING *
            "#,
        )
        .bind(id)
        .bind(tenant_id)
        .fetch_optional(pool)
        .await?
        .ok_or(ItemError::NotFound)?;

        Ok(item)
    }

    /// Fetch an item by id, scoped to tenant.
    pub async fn find_by_id(
        pool: &PgPool,
        id: Uuid,
        tenant_id: &str,
    ) -> Result<Option<Item>, ItemError> {
        let item =
            sqlx::query_as::<_, Item>("SELECT * FROM items WHERE id = $1 AND tenant_id = $2")
                .bind(id)
                .bind(tenant_id)
                .fetch_optional(pool)
                .await?;

        Ok(item)
    }

}

// ============================================================================
// Unit tests (pure validation; DB tests live in integration suite)
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_create() -> CreateItemRequest {
        CreateItemRequest {
            tenant_id: "t1".to_string(),
            sku: "SKU-001".to_string(),
            name: "Widget".to_string(),
            description: None,
            inventory_account_ref: "1200".to_string(),
            cogs_account_ref: "5000".to_string(),
            variance_account_ref: "5010".to_string(),
            uom: None,
            tracking_mode: TrackingMode::None,
            make_buy: None,
        }
    }

    #[test]
    fn create_request_valid() {
        assert!(valid_create().validate().is_ok());
    }

    #[test]
    fn create_request_empty_sku_rejected() {
        let mut r = valid_create();
        r.sku = "  ".to_string();
        assert!(matches!(r.validate(), Err(ItemError::Validation(_))));
    }

    #[test]
    fn create_request_empty_cogs_rejected() {
        let mut r = valid_create();
        r.cogs_account_ref = "".to_string();
        assert!(matches!(r.validate(), Err(ItemError::Validation(_))));
    }

    #[test]
    fn create_request_empty_inventory_account_rejected() {
        let mut r = valid_create();
        r.inventory_account_ref = "".to_string();
        assert!(matches!(r.validate(), Err(ItemError::Validation(_))));
    }

    #[test]
    fn create_request_empty_variance_account_rejected() {
        let mut r = valid_create();
        r.variance_account_ref = "".to_string();
        assert!(matches!(r.validate(), Err(ItemError::Validation(_))));
    }

    #[test]
    fn update_request_empty_name_rejected() {
        let r = UpdateItemRequest {
            tenant_id: "t1".to_string(),
            name: Some("".to_string()),
            description: None,
            inventory_account_ref: None,
            cogs_account_ref: None,
            variance_account_ref: None,
            uom: None,
        };
        assert!(matches!(r.validate(), Err(ItemError::Validation(_))));
    }

    #[test]
    fn update_request_none_fields_valid() {
        let r = UpdateItemRequest {
            tenant_id: "t1".to_string(),
            name: None,
            description: None,
            inventory_account_ref: None,
            cogs_account_ref: None,
            variance_account_ref: None,
            uom: None,
        };
        assert!(r.validate().is_ok());
    }

    #[test]
    fn create_request_invalid_make_buy_rejected() {
        let mut r = valid_create();
        r.make_buy = Some("invalid".to_string());
        assert!(matches!(r.validate(), Err(ItemError::Validation(_))));
    }

    #[test]
    fn create_request_valid_make_buy_accepted() {
        let mut r = valid_create();
        r.make_buy = Some("make".to_string());
        assert!(r.validate().is_ok());
        r.make_buy = Some("buy".to_string());
        assert!(r.validate().is_ok());
    }

    #[test]
    fn tracking_mode_roundtrip() {
        assert_eq!(
            TrackingMode::try_from("none".to_string()),
            Ok(TrackingMode::None)
        );
        assert_eq!(
            TrackingMode::try_from("lot".to_string()),
            Ok(TrackingMode::Lot)
        );
        assert_eq!(
            TrackingMode::try_from("serial".to_string()),
            Ok(TrackingMode::Serial)
        );
        assert!(TrackingMode::try_from("unknown".to_string()).is_err());
    }

    #[test]
    fn tracking_mode_display() {
        assert_eq!(TrackingMode::None.as_str(), "none");
        assert_eq!(TrackingMode::Lot.as_str(), "lot");
        assert_eq!(TrackingMode::Serial.as_str(), "serial");
    }
}
