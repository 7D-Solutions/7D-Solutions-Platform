use serde::{Deserialize, Serialize};
use std::fmt;

use super::super::shipments::types::ParseEnumError;

/// Inspection routing decision for an inbound shipment line.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RouteDecision {
    DirectToStock,
    SendToInspection,
}

impl RouteDecision {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::DirectToStock => "direct_to_stock",
            Self::SendToInspection => "send_to_inspection",
        }
    }

    pub fn from_str_value(s: &str) -> Result<Self, ParseEnumError> {
        match s {
            "direct_to_stock" => Ok(Self::DirectToStock),
            "send_to_inspection" => Ok(Self::SendToInspection),
            _ => Err(ParseEnumError {
                type_name: "RouteDecision",
                value: s.to_string(),
            }),
        }
    }

    pub fn event_type(&self) -> &'static str {
        match self {
            Self::DirectToStock => "sr.receipt_routed_to_stock.v1",
            Self::SendToInspection => "sr.receipt_routed_to_inspection.v1",
        }
    }
}

impl fmt::Display for RouteDecision {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl TryFrom<String> for RouteDecision {
    type Error = String;
    fn try_from(s: String) -> Result<Self, Self::Error> {
        Self::from_str_value(&s).map_err(|e| e.to_string())
    }
}
