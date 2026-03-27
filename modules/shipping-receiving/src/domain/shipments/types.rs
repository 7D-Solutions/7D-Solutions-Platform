use serde::{Deserialize, Serialize};
use std::fmt;

/// Error returned when parsing a string into a domain enum fails.
#[derive(Debug, Clone)]
pub struct ParseEnumError {
    pub type_name: &'static str,
    pub value: String,
}

impl fmt::Display for ParseEnumError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "invalid {} value: '{}'", self.type_name, self.value)
    }
}

impl std::error::Error for ParseEnumError {}

/// Shipment direction — determines which state machine applies.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Direction {
    Inbound,
    Outbound,
}

impl Direction {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Inbound => "inbound",
            Self::Outbound => "outbound",
        }
    }

    pub fn from_str_value(s: &str) -> Result<Self, ParseEnumError> {
        match s {
            "inbound" => Ok(Self::Inbound),
            "outbound" => Ok(Self::Outbound),
            _ => Err(ParseEnumError {
                type_name: "Direction",
                value: s.to_string(),
            }),
        }
    }
}

impl fmt::Display for Direction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl TryFrom<String> for Direction {
    type Error = String;
    fn try_from(s: String) -> Result<Self, Self::Error> {
        Self::from_str_value(&s).map_err(|e| e.to_string())
    }
}

/// Inbound shipment status.
///
/// ```text
/// draft → confirmed → in_transit → arrived → receiving → closed
///                                                         cancelled (from any non-terminal)
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InboundStatus {
    Draft,
    Confirmed,
    InTransit,
    Arrived,
    Receiving,
    Closed,
    Cancelled,
}

impl InboundStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Draft => "draft",
            Self::Confirmed => "confirmed",
            Self::InTransit => "in_transit",
            Self::Arrived => "arrived",
            Self::Receiving => "receiving",
            Self::Closed => "closed",
            Self::Cancelled => "cancelled",
        }
    }

    pub fn from_str_value(s: &str) -> Result<Self, ParseEnumError> {
        match s {
            "draft" => Ok(Self::Draft),
            "confirmed" => Ok(Self::Confirmed),
            "in_transit" => Ok(Self::InTransit),
            "arrived" => Ok(Self::Arrived),
            "receiving" => Ok(Self::Receiving),
            "closed" => Ok(Self::Closed),
            "cancelled" => Ok(Self::Cancelled),
            _ => Err(ParseEnumError {
                type_name: "InboundStatus",
                value: s.to_string(),
            }),
        }
    }

    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Closed | Self::Cancelled)
    }

    pub const ALL: [Self; 7] = [
        Self::Draft,
        Self::Confirmed,
        Self::InTransit,
        Self::Arrived,
        Self::Receiving,
        Self::Closed,
        Self::Cancelled,
    ];
}

impl fmt::Display for InboundStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl TryFrom<String> for InboundStatus {
    type Error = String;
    fn try_from(s: String) -> Result<Self, Self::Error> {
        Self::from_str_value(&s).map_err(|e| e.to_string())
    }
}

/// Outbound shipment status.
///
/// ```text
/// draft → confirmed → picking → packed → shipped → delivered → closed
///                                                                cancelled (from any non-terminal)
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutboundStatus {
    Draft,
    Confirmed,
    Picking,
    Packed,
    Shipped,
    Delivered,
    Closed,
    Cancelled,
}

impl OutboundStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Draft => "draft",
            Self::Confirmed => "confirmed",
            Self::Picking => "picking",
            Self::Packed => "packed",
            Self::Shipped => "shipped",
            Self::Delivered => "delivered",
            Self::Closed => "closed",
            Self::Cancelled => "cancelled",
        }
    }

    pub fn from_str_value(s: &str) -> Result<Self, ParseEnumError> {
        match s {
            "draft" => Ok(Self::Draft),
            "confirmed" => Ok(Self::Confirmed),
            "picking" => Ok(Self::Picking),
            "packed" => Ok(Self::Packed),
            "shipped" => Ok(Self::Shipped),
            "delivered" => Ok(Self::Delivered),
            "closed" => Ok(Self::Closed),
            "cancelled" => Ok(Self::Cancelled),
            _ => Err(ParseEnumError {
                type_name: "OutboundStatus",
                value: s.to_string(),
            }),
        }
    }

    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Closed | Self::Cancelled)
    }

    pub const ALL: [Self; 8] = [
        Self::Draft,
        Self::Confirmed,
        Self::Picking,
        Self::Packed,
        Self::Shipped,
        Self::Delivered,
        Self::Closed,
        Self::Cancelled,
    ];
}

impl fmt::Display for OutboundStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl TryFrom<String> for OutboundStatus {
    type Error = String;
    fn try_from(s: String) -> Result<Self, Self::Error> {
        Self::from_str_value(&s).map_err(|e| e.to_string())
    }
}

/// Shipment line quantity snapshot for invariant checking and event payloads.
/// Extracted from DB rows before guard evaluation.
#[derive(Debug, Clone)]
pub struct LineQty {
    pub line_id: uuid::Uuid,
    pub sku: String,
    pub qty_expected: i64,
    pub qty_shipped: i64,
    pub qty_received: i64,
    pub qty_accepted: i64,
    pub qty_rejected: i64,
}
