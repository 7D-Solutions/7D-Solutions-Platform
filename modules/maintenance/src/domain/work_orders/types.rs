use serde::{Deserialize, Serialize};
use std::fmt;

/// Work order type — classifies the nature of the maintenance work.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WoType {
    Preventive,
    Corrective,
    Inspection,
}

impl WoType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Preventive => "preventive",
            Self::Corrective => "corrective",
            Self::Inspection => "inspection",
        }
    }

    pub fn from_str_value(s: &str) -> Result<Self, ParseEnumError> {
        match s {
            "preventive" => Ok(Self::Preventive),
            "corrective" => Ok(Self::Corrective),
            "inspection" => Ok(Self::Inspection),
            _ => Err(ParseEnumError {
                type_name: "WoType",
                value: s.to_string(),
            }),
        }
    }
}

impl fmt::Display for WoType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Work order status — tracks lifecycle position within the state machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WoStatus {
    Draft,
    AwaitingApproval,
    Scheduled,
    InProgress,
    OnHold,
    Completed,
    Closed,
    Cancelled,
}

impl WoStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Draft => "draft",
            Self::AwaitingApproval => "awaiting_approval",
            Self::Scheduled => "scheduled",
            Self::InProgress => "in_progress",
            Self::OnHold => "on_hold",
            Self::Completed => "completed",
            Self::Closed => "closed",
            Self::Cancelled => "cancelled",
        }
    }

    pub fn from_str_value(s: &str) -> Result<Self, ParseEnumError> {
        match s {
            "draft" => Ok(Self::Draft),
            "awaiting_approval" => Ok(Self::AwaitingApproval),
            "scheduled" => Ok(Self::Scheduled),
            "in_progress" => Ok(Self::InProgress),
            "on_hold" => Ok(Self::OnHold),
            "completed" => Ok(Self::Completed),
            "closed" => Ok(Self::Closed),
            "cancelled" => Ok(Self::Cancelled),
            _ => Err(ParseEnumError {
                type_name: "WoStatus",
                value: s.to_string(),
            }),
        }
    }

    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Closed | Self::Cancelled)
    }
}

impl fmt::Display for WoStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Priority level for work orders and maintenance plans.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Priority {
    Low,
    Medium,
    High,
    Critical,
}

impl Priority {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
            Self::Critical => "critical",
        }
    }

    pub fn from_str_value(s: &str) -> Result<Self, ParseEnumError> {
        match s {
            "low" => Ok(Self::Low),
            "medium" => Ok(Self::Medium),
            "high" => Ok(Self::High),
            "critical" => Ok(Self::Critical),
            _ => Err(ParseEnumError {
                type_name: "Priority",
                value: s.to_string(),
            }),
        }
    }
}

impl fmt::Display for Priority {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Asset type — classifies what kind of thing is being maintained.
/// Asset-type agnostic: this is for filtering/reporting, not behavior.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AssetType {
    Vehicle,
    Machinery,
    Equipment,
    Facility,
    Other,
}

impl AssetType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Vehicle => "vehicle",
            Self::Machinery => "machinery",
            Self::Equipment => "equipment",
            Self::Facility => "facility",
            Self::Other => "other",
        }
    }

    pub fn from_str_value(s: &str) -> Result<Self, ParseEnumError> {
        match s {
            "vehicle" => Ok(Self::Vehicle),
            "machinery" => Ok(Self::Machinery),
            "equipment" => Ok(Self::Equipment),
            "facility" => Ok(Self::Facility),
            "other" => Ok(Self::Other),
            _ => Err(ParseEnumError {
                type_name: "AssetType",
                value: s.to_string(),
            }),
        }
    }
}

impl fmt::Display for AssetType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Asset operational status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AssetStatus {
    Active,
    Inactive,
    Retired,
}

impl AssetStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Inactive => "inactive",
            Self::Retired => "retired",
        }
    }

    pub fn from_str_value(s: &str) -> Result<Self, ParseEnumError> {
        match s {
            "active" => Ok(Self::Active),
            "inactive" => Ok(Self::Inactive),
            "retired" => Ok(Self::Retired),
            _ => Err(ParseEnumError {
                type_name: "AssetStatus",
                value: s.to_string(),
            }),
        }
    }
}

impl fmt::Display for AssetStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Schedule type for maintenance plans.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScheduleType {
    Calendar,
    Meter,
    Both,
}

impl ScheduleType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Calendar => "calendar",
            Self::Meter => "meter",
            Self::Both => "both",
        }
    }

    pub fn from_str_value(s: &str) -> Result<Self, ParseEnumError> {
        match s {
            "calendar" => Ok(Self::Calendar),
            "meter" => Ok(Self::Meter),
            "both" => Ok(Self::Both),
            _ => Err(ParseEnumError {
                type_name: "ScheduleType",
                value: s.to_string(),
            }),
        }
    }
}

impl fmt::Display for ScheduleType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Error returned when parsing a string into a domain enum fails.
#[derive(Debug, Clone)]
pub struct ParseEnumError {
    pub type_name: &'static str,
    pub value: String,
}

impl fmt::Display for ParseEnumError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "invalid {} value: '{}'",
            self.type_name, self.value
        )
    }
}

impl std::error::Error for ParseEnumError {}
