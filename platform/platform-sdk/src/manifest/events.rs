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

    /// When `true`, the SDK creates the standard outbox table at startup
    /// if it does not already exist. Modules with custom migrations
    /// should leave this `false` (the default).
    #[serde(default)]
    pub auto_create: bool,

    /// Optional prefix prepended to `event_type` before publishing
    /// (e.g. `"trashtech.events"` → subject becomes `"trashtech.events.stop.started"`).
    #[serde(default)]
    pub subject_prefix: Option<String>,

    #[serde(flatten)]
    pub extra: BTreeMap<String, toml::Value>,
}
