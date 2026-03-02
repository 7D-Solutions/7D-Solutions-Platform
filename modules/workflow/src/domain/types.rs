//! Core workflow types and status enums.

use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InstanceStatus {
    Active,
    Completed,
    Cancelled,
}

impl fmt::Display for InstanceStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            InstanceStatus::Active => write!(f, "active"),
            InstanceStatus::Completed => write!(f, "completed"),
            InstanceStatus::Cancelled => write!(f, "cancelled"),
        }
    }
}

impl TryFrom<String> for InstanceStatus {
    type Error = String;
    fn try_from(s: String) -> Result<Self, Self::Error> {
        match s.as_str() {
            "active" => Ok(InstanceStatus::Active),
            "completed" => Ok(InstanceStatus::Completed),
            "cancelled" => Ok(InstanceStatus::Cancelled),
            _ => Err(format!("Invalid instance status: {}", s)),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StepDefinition {
    pub step_id: String,
    pub name: String,
    pub step_type: String,
    pub position: i32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn instance_status_roundtrip() {
        let status = InstanceStatus::Active;
        assert_eq!(status.to_string(), "active");
        assert_eq!(
            InstanceStatus::try_from("active".to_string()).unwrap(),
            InstanceStatus::Active
        );
    }

    #[test]
    fn instance_status_invalid() {
        assert!(InstanceStatus::try_from("bogus".to_string()).is_err());
    }
}
