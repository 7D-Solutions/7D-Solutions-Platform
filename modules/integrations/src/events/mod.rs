//! Integrations event contracts v1.
//!
//! All events carry a full EventEnvelope with:
//! - schema_version: "1.0.0"
//! - source_module: "integrations"
//! - mutation_class: DATA_MUTATION or LIFECYCLE
//! - replay_safe: true

pub mod envelope;
pub mod external_ref_created;
pub mod external_ref_deleted;
pub mod external_ref_updated;
pub mod webhook_received;
pub mod webhook_routed;

// ============================================================================
// Shared Constants
// ============================================================================

pub const INTEGRATIONS_EVENT_SCHEMA_VERSION: &str = "1.0.0";
pub const MUTATION_CLASS_DATA_MUTATION: &str = "DATA_MUTATION";
pub const MUTATION_CLASS_LIFECYCLE: &str = "LIFECYCLE";

// ============================================================================
// Re-exports
// ============================================================================

pub use external_ref_created::{
    build_external_ref_created_envelope, ExternalRefCreatedPayload,
    EVENT_TYPE_EXTERNAL_REF_CREATED,
};
pub use external_ref_deleted::{
    build_external_ref_deleted_envelope, ExternalRefDeletedPayload,
    EVENT_TYPE_EXTERNAL_REF_DELETED,
};
pub use external_ref_updated::{
    build_external_ref_updated_envelope, ExternalRefUpdatedPayload,
    EVENT_TYPE_EXTERNAL_REF_UPDATED,
};
pub use webhook_received::{
    build_webhook_received_envelope, WebhookReceivedPayload,
    EVENT_TYPE_WEBHOOK_RECEIVED,
};
pub use webhook_routed::{
    build_webhook_routed_envelope, WebhookRoutedPayload,
    EVENT_TYPE_WEBHOOK_ROUTED,
};
