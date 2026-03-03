use serde::{Deserialize, Serialize};
use std::fmt;

use super::super::shipments::types::ParseEnumError;

/// Carrier request type — what the integration is asking the carrier for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CarrierRequestType {
    Rate,
    Label,
    Track,
}

impl CarrierRequestType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Rate => "rate",
            Self::Label => "label",
            Self::Track => "track",
        }
    }

    pub fn from_str_value(s: &str) -> Result<Self, ParseEnumError> {
        match s {
            "rate" => Ok(Self::Rate),
            "label" => Ok(Self::Label),
            "track" => Ok(Self::Track),
            _ => Err(ParseEnumError {
                type_name: "CarrierRequestType",
                value: s.to_string(),
            }),
        }
    }
}

impl fmt::Display for CarrierRequestType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Carrier request status.
///
/// ```text
/// pending → submitted → completed | failed
/// failed → submitted (retry)
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CarrierRequestStatus {
    Pending,
    Submitted,
    Completed,
    Failed,
}

impl CarrierRequestStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Submitted => "submitted",
            Self::Completed => "completed",
            Self::Failed => "failed",
        }
    }

    pub fn from_str_value(s: &str) -> Result<Self, ParseEnumError> {
        match s {
            "pending" => Ok(Self::Pending),
            "submitted" => Ok(Self::Submitted),
            "completed" => Ok(Self::Completed),
            "failed" => Ok(Self::Failed),
            _ => Err(ParseEnumError {
                type_name: "CarrierRequestStatus",
                value: s.to_string(),
            }),
        }
    }

    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Completed)
    }
}

impl fmt::Display for CarrierRequestStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}
