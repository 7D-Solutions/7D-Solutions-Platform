use serde::{Deserialize, Serialize};
use std::fmt;

use super::super::shipments::types::ParseEnumError;

/// Shipping document type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DocType {
    PackingSlip,
    BillOfLading,
}

impl DocType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::PackingSlip => "packing_slip",
            Self::BillOfLading => "bill_of_lading",
        }
    }

    pub fn from_str_value(s: &str) -> Result<Self, ParseEnumError> {
        match s {
            "packing_slip" => Ok(Self::PackingSlip),
            "bill_of_lading" => Ok(Self::BillOfLading),
            _ => Err(ParseEnumError {
                type_name: "DocType",
                value: s.to_string(),
            }),
        }
    }
}

impl fmt::Display for DocType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Shipping document request status.
///
/// ```text
/// requested → generating → completed | failed
/// failed → generating (retry)
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DocRequestStatus {
    Requested,
    Generating,
    Completed,
    Failed,
}

impl DocRequestStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Requested => "requested",
            Self::Generating => "generating",
            Self::Completed => "completed",
            Self::Failed => "failed",
        }
    }

    pub fn from_str_value(s: &str) -> Result<Self, ParseEnumError> {
        match s {
            "requested" => Ok(Self::Requested),
            "generating" => Ok(Self::Generating),
            "completed" => Ok(Self::Completed),
            "failed" => Ok(Self::Failed),
            _ => Err(ParseEnumError {
                type_name: "DocRequestStatus",
                value: s.to_string(),
            }),
        }
    }

    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Completed)
    }
}

impl fmt::Display for DocRequestStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}
