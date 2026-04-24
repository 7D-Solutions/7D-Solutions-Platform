//! Notification event and channel enums + validators (bd-kv15d).
//!
//! Stable wire values are the serde snake_case representations.

use platform_http_contracts::FieldError;
use serde::{Deserialize, Serialize};

/// Shipment lifecycle events a customer can subscribe to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NotificationEvent {
    Shipped,
    OutForDelivery,
    Delivered,
    Exception,
}

impl NotificationEvent {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Shipped => "shipped",
            Self::OutForDelivery => "out_for_delivery",
            Self::Delivered => "delivered",
            Self::Exception => "exception",
        }
    }

    fn from_str(s: &str) -> Option<Self> {
        match s {
            "shipped" => Some(Self::Shipped),
            "out_for_delivery" => Some(Self::OutForDelivery),
            "delivered" => Some(Self::Delivered),
            "exception" => Some(Self::Exception),
            _ => None,
        }
    }
}

/// Delivery channels for notifications.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NotificationChannel {
    Email,
    Sms,
}

impl NotificationChannel {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Email => "email",
            Self::Sms => "sms",
        }
    }

    fn from_str(s: &str) -> Option<Self> {
        match s {
            "email" => Some(Self::Email),
            "sms" => Some(Self::Sms),
            _ => None,
        }
    }
}

/// Parse and validate a slice of event strings. Returns FieldError on unknown value.
pub fn parse_notification_events(vals: &[String]) -> Result<Vec<NotificationEvent>, FieldError> {
    vals.iter()
        .map(|s| {
            NotificationEvent::from_str(s.as_str()).ok_or_else(|| FieldError {
                field: "notification_events".to_string(),
                message: format!(
                    "Unknown notification event '{}'. Valid: shipped, out_for_delivery, delivered, exception",
                    s
                ),
            })
        })
        .collect()
}

/// Parse and validate a slice of channel strings. Returns FieldError on unknown value.
pub fn parse_notification_channels(vals: &[String]) -> Result<Vec<NotificationChannel>, FieldError> {
    vals.iter()
        .map(|s| {
            NotificationChannel::from_str(s.as_str()).ok_or_else(|| FieldError {
                field: "notification_channels".to_string(),
                message: format!(
                    "Unknown notification channel '{}'. Valid: email, sms",
                    s
                ),
            })
        })
        .collect()
}

/// Resolve effective events and channels for a ship-to contact.
///
/// Each column resolves independently:
/// - If the contact's column is non-NULL, it wins.
/// - Otherwise, falls back to the party-level column.
pub fn resolve_for_contact(
    party_events: Vec<String>,
    party_channels: Vec<String>,
    contact_events: Option<Vec<String>>,
    contact_channels: Option<Vec<String>>,
) -> (Vec<String>, Vec<String>) {
    let effective_events = contact_events.unwrap_or(party_events);
    let effective_channels = contact_channels.unwrap_or(party_channels);
    (effective_events, effective_channels)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_known_events_succeeds() -> Result<(), Box<dyn std::error::Error>> {
        let vals = vec![
            "shipped".to_string(),
            "delivered".to_string(),
            "exception".to_string(),
        ];
        let result = parse_notification_events(&vals).map_err(|e| e.message)?;
        assert_eq!(result.len(), 3);
        Ok(())
    }

    #[test]
    fn parse_unknown_event_returns_field_error() {
        let vals = vec!["shipped".to_string(), "unknown_event".to_string()];
        let err = parse_notification_events(&vals).unwrap_err();
        assert_eq!(err.field, "notification_events");
        assert!(err.message.contains("unknown_event"));
    }

    #[test]
    fn parse_known_channels_succeeds() -> Result<(), Box<dyn std::error::Error>> {
        let vals = vec!["email".to_string(), "sms".to_string()];
        let result = parse_notification_channels(&vals).map_err(|e| e.message)?;
        assert_eq!(result.len(), 2);
        Ok(())
    }

    #[test]
    fn parse_unknown_channel_returns_field_error() {
        let vals = vec!["carrier_pigeon".to_string()];
        let err = parse_notification_channels(&vals).unwrap_err();
        assert_eq!(err.field, "notification_channels");
    }

    #[test]
    fn resolve_contact_overrides_events_only() {
        let (events, channels) = resolve_for_contact(
            vec!["shipped".into(), "delivered".into()],
            vec!["email".into()],
            Some(vec!["exception".into()]),
            None,
        );
        // Contact overrides events; channels fall back to party
        assert_eq!(events, vec!["exception".to_string()]);
        assert_eq!(channels, vec!["email".to_string()]);
    }

    #[test]
    fn resolve_both_null_returns_party_values() {
        let (events, channels) = resolve_for_contact(
            vec!["shipped".into()],
            vec!["sms".into()],
            None,
            None,
        );
        assert_eq!(events, vec!["shipped".to_string()]);
        assert_eq!(channels, vec!["sms".to_string()]);
    }

    #[test]
    fn resolve_both_non_null_contact_wins() {
        let (events, channels) = resolve_for_contact(
            vec!["shipped".into()],
            vec!["email".into()],
            Some(vec!["delivered".into()]),
            Some(vec!["sms".into()]),
        );
        assert_eq!(events, vec!["delivered".to_string()]);
        assert_eq!(channels, vec!["sms".to_string()]);
    }
}
