//! Make/Buy classification for items.
//!
//! Classifies items as MAKE (manufactured in-house) or BUY (purchased).
//! Tenant-scoped, evented on change.

use chrono::Utc;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use thiserror::Error;
use uuid::Uuid;

use crate::domain::items::{Item, ItemError};
use crate::events::{
    build_make_buy_changed_envelope, MakeBuyChangedPayload, EVENT_TYPE_MAKE_BUY_CHANGED,
};

// ============================================================================
// MakeBuy enum
// ============================================================================

/// Manufacturing classification: MAKE (manufactured in-house) or BUY (purchased).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MakeBuy {
    Make,
    Buy,
}

impl MakeBuy {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Make => "make",
            Self::Buy => "buy",
        }
    }
}

impl std::fmt::Display for MakeBuy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl TryFrom<String> for MakeBuy {
    type Error = String;

    fn try_from(s: String) -> Result<Self, Self::Error> {
        match s.as_str() {
            "make" => Ok(Self::Make),
            "buy" => Ok(Self::Buy),
            other => Err(format!("invalid make_buy '{}': expected make|buy", other)),
        }
    }
}

// ============================================================================
// Validation
// ============================================================================

pub fn validate_make_buy(value: &Option<String>) -> Result<(), ItemError> {
    if let Some(ref v) = value {
        match v.as_str() {
            "make" | "buy" => {}
            _ => {
                return Err(ItemError::Validation(format!(
                    "make_buy must be 'make' or 'buy' (got '{}')",
                    v
                )));
            }
        }
    }
    Ok(())
}

// ============================================================================
// Errors
// ============================================================================

#[derive(Debug, Error)]
pub enum MakeBuyError {
    #[error("Item not found")]
    NotFound,

    #[error("Validation error: {0}")]
    Validation(String),

    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),
}

// ============================================================================
// Service: set make/buy with event emission
// ============================================================================

/// Input for PUT /api/inventory/items/:id/make-buy
#[derive(Debug, Deserialize)]
pub struct SetMakeBuyRequest {
    pub tenant_id: String,
    /// "make" | "buy"
    pub make_buy: String,
    pub correlation_id: Option<String>,
    pub causation_id: Option<String>,
}

/// Result of setting make/buy
#[derive(Debug, Serialize)]
pub struct SetMakeBuyResult {
    pub item: Item,
    pub previous_value: Option<String>,
}

/// Set the make/buy classification on an item, emitting an event.
///
/// Pattern: Guard → Mutation → Outbox (single transaction).
pub async fn set_make_buy(
    pool: &PgPool,
    item_id: Uuid,
    req: &SetMakeBuyRequest,
) -> Result<SetMakeBuyResult, MakeBuyError> {
    // Validate
    if req.tenant_id.trim().is_empty() {
        return Err(MakeBuyError::Validation("tenant_id is required".into()));
    }
    validate_make_buy(&Some(req.make_buy.clone()))
        .map_err(|e| MakeBuyError::Validation(e.to_string()))?;

    let event_id = Uuid::new_v4();
    let now = Utc::now();
    let correlation_id = req
        .correlation_id
        .clone()
        .unwrap_or_else(|| Uuid::new_v4().to_string());

    let mut tx = pool.begin().await?;

    // Read old value + guard existence
    let old_value: Option<String> = sqlx::query_scalar(
        "SELECT make_buy FROM items WHERE id = $1 AND tenant_id = $2 FOR UPDATE",
    )
    .bind(item_id)
    .bind(&req.tenant_id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(MakeBuyError::NotFound)?;

    // Update
    let item = sqlx::query_as::<_, Item>(
        r#"
        UPDATE items
        SET make_buy = $3, updated_at = NOW()
        WHERE id = $1 AND tenant_id = $2
        RETURNING *
        "#,
    )
    .bind(item_id)
    .bind(&req.tenant_id)
    .bind(&req.make_buy)
    .fetch_one(&mut *tx)
    .await?;

    // Outbox event
    let payload = MakeBuyChangedPayload {
        tenant_id: req.tenant_id.clone(),
        item_id,
        previous_value: old_value.clone(),
        new_value: req.make_buy.clone(),
        changed_at: now,
    };
    let envelope = build_make_buy_changed_envelope(
        event_id,
        req.tenant_id.clone(),
        correlation_id.clone(),
        req.causation_id.clone(),
        payload,
    );
    let envelope_json = serde_json::to_string(&envelope)?;

    sqlx::query(
        r#"
        INSERT INTO inv_outbox
            (event_id, event_type, aggregate_type, aggregate_id, tenant_id,
             payload, correlation_id, causation_id, schema_version)
        VALUES ($1, $2, 'inventory_item', $3, $4, $5::JSONB, $6, $7, '1.0.0')
        "#,
    )
    .bind(event_id)
    .bind(EVENT_TYPE_MAKE_BUY_CHANGED)
    .bind(item_id.to_string())
    .bind(&req.tenant_id)
    .bind(&envelope_json)
    .bind(&correlation_id)
    .bind(&req.causation_id)
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;

    Ok(SetMakeBuyResult {
        item,
        previous_value: old_value,
    })
}

// ============================================================================
// Unit tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn make_buy_roundtrip() {
        assert_eq!(MakeBuy::try_from("make".to_string()), Ok(MakeBuy::Make));
        assert_eq!(MakeBuy::try_from("buy".to_string()), Ok(MakeBuy::Buy));
        assert!(MakeBuy::try_from("unknown".to_string()).is_err());
    }

    #[test]
    fn validate_accepts_valid_values() {
        assert!(validate_make_buy(&Some("make".to_string())).is_ok());
        assert!(validate_make_buy(&Some("buy".to_string())).is_ok());
        assert!(validate_make_buy(&None).is_ok());
    }

    #[test]
    fn validate_rejects_invalid_value() {
        assert!(validate_make_buy(&Some("invalid".to_string())).is_err());
    }
}
