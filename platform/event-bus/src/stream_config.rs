//! JetStream stream configuration with per-class dedup windows.
//!
//! # Design
//!
//! NATS JetStream deduplicates messages using the `Nats-Msg-Id` header within
//! a configurable window (`duplicate_window`). The default is 2 minutes — too
//! short for financial events where a slow consumer lagging more than 2 minutes
//! turns "exactly-once" into "at-least-once", causing double-posted ledger entries.
//!
//! This module declares explicit windows per stream class:
//!
//! | Class        | Window | Streams                                                   |
//! |--------------|--------|-----------------------------------------------------------|
//! | Financial    | 24h    | FINANCIAL_EVENTS (ap, ar, gl, payments, billing, etc.)    |
//! | Operational  | 1h     | OPERATIONAL_EVENTS (production, inventory, shipping, ...) |
//! | Notification | 1h     | NOTIFICATION_EVENTS (notifications)                       |
//! | System       | 24h    | SYSTEM_EVENTS (tenant)                                    |
//!
//! Note: `auth.*` is managed by the identity-auth module (AUTH_EVENTS stream).
//!
//! # Startup
//!
//! Call [`ensure_platform_streams`] once at startup (after NATS connects) to
//! create or update all streams with the correct dedup windows.
//!
//! # Publishers
//!
//! Publishers must set the `Nats-Msg-Id` header to the `EventEnvelope.event_id`
//! so NATS can deduplicate within the window:
//!
//! ```rust,ignore
//! use async_nats::jetstream::{self, context::Publish};
//!
//! js.send_publish(
//!     subject,
//!     Publish::build()
//!         .payload(bytes.into())
//!         .message_id(envelope.event_id.to_string()),
//! ).await?.await?;
//! ```

use std::time::Duration;

/// Classification of a stream by its operational domain.
///
/// Determines the dedup window and retention policy applied to the JetStream stream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StreamClass {
    /// Financial ledger events: AP, AR, GL, payments, billing, treasury, subscriptions.
    /// Dedup window: 24h — financial double-processing is a compliance violation.
    Financial,

    /// Operational domain events: production, inventory, shipping, maintenance, workflow.
    /// Dedup window: 1h — operational duplicates are recoverable via idempotent handlers.
    Operational,

    /// Notification events.
    /// Dedup window: 1h — re-sending a notification is annoying, not catastrophic.
    Notification,

    /// Platform infrastructure events: tenant lifecycle.
    /// Dedup window: 24h — tenant provisioning is non-idempotent; protect aggressively.
    System,
}

impl StreamClass {
    /// The dedup window for this class.
    pub fn dedup_window(&self) -> Duration {
        match self {
            StreamClass::Financial => Duration::from_secs(86_400), // 24h
            StreamClass::Operational => Duration::from_secs(3_600), // 1h
            StreamClass::Notification => Duration::from_secs(3_600), // 1h
            StreamClass::System => Duration::from_secs(86_400),    // 24h
        }
    }

    /// Human-readable name for documentation.
    pub fn label(&self) -> &'static str {
        match self {
            StreamClass::Financial => "financial",
            StreamClass::Operational => "operational",
            StreamClass::Notification => "notification",
            StreamClass::System => "system",
        }
    }
}

/// Full configuration for one JetStream stream.
#[derive(Debug, Clone)]
pub struct StreamDefinition {
    /// JetStream stream name (e.g., `"FINANCIAL_EVENTS"`).
    pub name: &'static str,
    /// Subject patterns captured by this stream (e.g., `["ap.>", "ar.>"]`).
    pub subjects: Vec<String>,
    /// How long NATS tracks `Nats-Msg-Id` for deduplication.
    pub dedup_window: Duration,
    /// Maximum age of messages before they are purged from the stream.
    pub max_age: Duration,
    /// Domain classification driving dedup policy.
    pub class: StreamClass,
}

/// Returns the canonical set of platform stream definitions.
///
/// Each entry becomes a JetStream stream created (or updated) at startup
/// via [`ensure_platform_streams`].
pub fn all_stream_definitions() -> Vec<StreamDefinition> {
    vec![
        // ── Financial ─────────────────────────────────────────────────────────
        StreamDefinition {
            name: "FINANCIAL_EVENTS",
            subjects: vec![
                "ap.>".into(),
                "ar.>".into(),
                "gl.>".into(),
                "payments.>".into(),
                "billing.>".into(),
                "treasury.>".into(),
                "subscriptions.>".into(),
                "ttp.>".into(),
            ],
            dedup_window: StreamClass::Financial.dedup_window(),
            max_age: Duration::from_secs(86_400 * 14), // 14 days
            class: StreamClass::Financial,
        },
        // ── Operational ───────────────────────────────────────────────────────
        StreamDefinition {
            name: "OPERATIONAL_EVENTS",
            subjects: vec![
                "production.>".into(),
                "inventory.>".into(),
                "shipping.>".into(),
                "maintenance.>".into(),
                "workflow.>".into(),
                "quality.>".into(),
                "fixed-assets.>".into(),
                "timekeeping.>".into(),
                "workforce.>".into(),
                "numbering.>".into(),
            ],
            dedup_window: StreamClass::Operational.dedup_window(),
            max_age: Duration::from_secs(86_400 * 14), // 14 days
            class: StreamClass::Operational,
        },
        // ── Notification ──────────────────────────────────────────────────────
        StreamDefinition {
            name: "NOTIFICATION_EVENTS",
            subjects: vec!["notifications.>".into()],
            dedup_window: StreamClass::Notification.dedup_window(),
            max_age: Duration::from_secs(86_400 * 7), // 7 days
            class: StreamClass::Notification,
        },
        // ── System ────────────────────────────────────────────────────────────
        // Note: `auth.*` is managed separately by identity-auth (AUTH_EVENTS stream).
        StreamDefinition {
            name: "SYSTEM_EVENTS",
            subjects: vec!["tenant.>".into()],
            dedup_window: StreamClass::System.dedup_window(),
            max_age: Duration::from_secs(86_400 * 14), // 14 days
            class: StreamClass::System,
        },
    ]
}

/// Errors returned by [`ensure_platform_streams`].
#[derive(Debug, thiserror::Error)]
#[error("JetStream stream setup failed for '{stream}': {reason}")]
pub struct EnsureStreamsError {
    pub stream: String,
    pub reason: String,
}

/// Create or update all platform JetStream streams with their configured dedup windows.
///
/// This is idempotent: safe to call on every startup. Streams that already exist
/// are updated to reflect any configuration changes (e.g., a dedup window that
/// was added or lengthened). New streams are created.
///
/// # Errors
///
/// Returns the first stream that fails to create or update. Streams are processed
/// in definition order.
///
/// # Example
///
/// ```rust,no_run
/// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
/// let nats = event_bus::connect_nats("nats://localhost:4222").await?;
/// event_bus::stream_config::ensure_platform_streams(nats).await?;
/// # Ok(())
/// # }
/// ```
pub async fn ensure_platform_streams(nats: async_nats::Client) -> Result<(), EnsureStreamsError> {
    use async_nats::jetstream::{self, stream};

    let js = jetstream::new(nats);

    for def in all_stream_definitions() {
        let cfg = stream::Config {
            name: def.name.to_string(),
            subjects: def.subjects.clone(),
            duplicate_window: def.dedup_window,
            max_age: def.max_age,
            ..Default::default()
        };

        // Try to get the stream first. If it exists, update its config so
        // existing deployments pick up the new dedup window. If absent, create.
        let result = match js.get_stream(def.name).await {
            Ok(_) => js
                .update_stream(cfg)
                .await
                .map(|_| ())
                .map_err(|e| e.to_string()),
            Err(_) => js
                .create_stream(cfg)
                .await
                .map(|_| ())
                .map_err(|e| e.to_string()),
        };

        result.map_err(|reason| EnsureStreamsError {
            stream: def.name.to_string(),
            reason,
        })?;

        tracing::info!(
            stream = def.name,
            class = def.class.label(),
            dedup_window_secs = def.dedup_window.as_secs(),
            "JetStream stream configured"
        );
    }

    Ok(())
}
