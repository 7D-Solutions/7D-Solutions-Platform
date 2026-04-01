use std::collections::BTreeMap;

use serde::Deserialize;

/// `[events]` — event publishing and consuming configuration.
#[derive(Debug, Clone, Deserialize, Default)]
pub struct EventsSection {
    /// `[events.publish]` — outbox publisher settings.
    #[serde(default)]
    pub publish: Option<EventsPublishSection>,

    #[serde(flatten)]
    pub extra: BTreeMap<String, toml::Value>,
}

/// `[events.publish]` — outbox publisher configuration.
#[derive(Debug, Clone, Deserialize)]
pub struct EventsPublishSection {
    /// Name of the outbox table to poll (e.g. `"events_outbox"`).
    pub outbox_table: String,

    #[serde(flatten)]
    pub extra: BTreeMap<String, toml::Value>,
}
