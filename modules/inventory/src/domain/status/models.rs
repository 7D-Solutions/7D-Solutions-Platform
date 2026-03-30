//! Status bucket domain model.
//!
//! Stock units belong to exactly one status bucket per (item, warehouse):
//!   available  — normal, reservable stock
//!   quarantine — held for QA inspection; not reservable
//!   damaged    — write-off candidate; not reservable
//!
//! Only 'available' counts toward reservable quantity.
//! Status changes are future movements (adjustments), never in-place updates.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// The three mutually-exclusive status buckets for stock units.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum InvItemStatus {
    Available,
    Quarantine,
    Damaged,
}

impl InvItemStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Available => "available",
            Self::Quarantine => "quarantine",
            Self::Damaged => "damaged",
        }
    }
}

impl std::fmt::Display for InvItemStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl TryFrom<String> for InvItemStatus {
    type Error = String;

    fn try_from(s: String) -> Result<Self, Self::Error> {
        match s.as_str() {
            "available" => Ok(Self::Available),
            "quarantine" => Ok(Self::Quarantine),
            "damaged" => Ok(Self::Damaged),
            other => Err(format!(
                "invalid inv_item_status '{}': expected available|quarantine|damaged",
                other
            )),
        }
    }
}

/// A row from `item_on_hand_by_status`.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct OnHandByStatusRow {
    pub tenant_id: String,
    pub item_id: Uuid,
    pub warehouse_id: Uuid,
    pub status: String,
    pub quantity_on_hand: i64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_roundtrip() {
        assert_eq!(
            InvItemStatus::try_from("available".to_string()),
            Ok(InvItemStatus::Available)
        );
        assert_eq!(
            InvItemStatus::try_from("quarantine".to_string()),
            Ok(InvItemStatus::Quarantine)
        );
        assert_eq!(
            InvItemStatus::try_from("damaged".to_string()),
            Ok(InvItemStatus::Damaged)
        );
        assert!(InvItemStatus::try_from("unknown".to_string()).is_err());
    }

    #[test]
    fn status_display() {
        assert_eq!(InvItemStatus::Available.as_str(), "available");
        assert_eq!(InvItemStatus::Quarantine.as_str(), "quarantine");
        assert_eq!(InvItemStatus::Damaged.as_str(), "damaged");
    }
}
