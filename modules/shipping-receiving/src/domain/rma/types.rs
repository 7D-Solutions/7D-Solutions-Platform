use serde::{Deserialize, Serialize};
use std::fmt;

use super::super::shipments::types::ParseEnumError;

/// RMA disposition status — tracks the lifecycle of returned goods.
///
/// ```text
/// received → inspect → quarantine → return_to_stock | scrap
///                    → return_to_stock | scrap
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DispositionStatus {
    Received,
    Inspect,
    Quarantine,
    ReturnToStock,
    Scrap,
}

impl DispositionStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Received => "received",
            Self::Inspect => "inspect",
            Self::Quarantine => "quarantine",
            Self::ReturnToStock => "return_to_stock",
            Self::Scrap => "scrap",
        }
    }

    pub fn from_str_value(s: &str) -> Result<Self, ParseEnumError> {
        match s {
            "received" => Ok(Self::Received),
            "inspect" => Ok(Self::Inspect),
            "quarantine" => Ok(Self::Quarantine),
            "return_to_stock" => Ok(Self::ReturnToStock),
            "scrap" => Ok(Self::Scrap),
            _ => Err(ParseEnumError {
                type_name: "DispositionStatus",
                value: s.to_string(),
            }),
        }
    }

    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::ReturnToStock | Self::Scrap)
    }

    pub const ALL: [Self; 5] = [
        Self::Received,
        Self::Inspect,
        Self::Quarantine,
        Self::ReturnToStock,
        Self::Scrap,
    ];
}

impl fmt::Display for DispositionStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl TryFrom<String> for DispositionStatus {
    type Error = String;
    fn try_from(s: String) -> Result<Self, Self::Error> {
        Self::from_str_value(&s).map_err(|e| e.to_string())
    }
}
