//! Integrations event contracts v1.
//!
//! All events carry a full EventEnvelope with:
//! - schema_version: "1.0.0"
//! - source_module: "integrations"
//! - mutation_class: DATA_MUTATION or LIFECYCLE
//! - replay_safe: true

pub mod edi_transaction_created;
pub mod edi_transaction_status_changed;
pub mod envelope;
pub mod external_ref_created;
pub mod external_ref_deleted;
pub mod external_ref_updated;
pub mod file_job_created;
pub mod file_job_status_changed;
pub mod order_ingested;
pub mod outbound_webhook_created;
pub mod outbound_webhook_deleted;
pub mod outbound_webhook_updated;
pub mod sync_authority_changed;
pub mod sync_push_failed;
pub mod webhook_received;
pub mod webhook_routed;

// ============================================================================
// Shared Constants
// ============================================================================

pub const INTEGRATIONS_EVENT_SCHEMA_VERSION: &str = "1.0.0";
pub const MUTATION_CLASS_DATA_MUTATION: &str = "DATA_MUTATION";
pub const MUTATION_CLASS_LIFECYCLE: &str = "LIFECYCLE";
pub const MUTATION_CLASS_SIDE_EFFECT: &str = "SIDE_EFFECT";

// ============================================================================
// Re-exports
// ============================================================================

pub use edi_transaction_created::{
    build_edi_transaction_created_envelope, EdiTransactionCreatedPayload,
    EVENT_TYPE_EDI_TRANSACTION_CREATED,
};
pub use edi_transaction_status_changed::{
    build_edi_transaction_status_changed_envelope, EdiTransactionStatusChangedPayload,
    EVENT_TYPE_EDI_TRANSACTION_STATUS_CHANGED,
};
pub use external_ref_created::{
    build_external_ref_created_envelope, ExternalRefCreatedPayload, EVENT_TYPE_EXTERNAL_REF_CREATED,
};
pub use external_ref_deleted::{
    build_external_ref_deleted_envelope, ExternalRefDeletedPayload, EVENT_TYPE_EXTERNAL_REF_DELETED,
};
pub use external_ref_updated::{
    build_external_ref_updated_envelope, ExternalRefUpdatedPayload, EVENT_TYPE_EXTERNAL_REF_UPDATED,
};
pub use file_job_created::{
    build_file_job_created_envelope, FileJobCreatedPayload, EVENT_TYPE_FILE_JOB_CREATED,
};
pub use file_job_status_changed::{
    build_file_job_status_changed_envelope, FileJobStatusChangedPayload,
    EVENT_TYPE_FILE_JOB_STATUS_CHANGED,
};
pub use order_ingested::{
    build_order_ingested_envelope, OrderIngestedPayload, OrderLineItemPayload,
    EVENT_TYPE_ORDER_INGESTED,
};
pub use outbound_webhook_created::{
    build_outbound_webhook_created_envelope, OutboundWebhookCreatedPayload,
    EVENT_TYPE_OUTBOUND_WEBHOOK_CREATED,
};
pub use outbound_webhook_deleted::{
    build_outbound_webhook_deleted_envelope, OutboundWebhookDeletedPayload,
    EVENT_TYPE_OUTBOUND_WEBHOOK_DELETED,
};
pub use outbound_webhook_updated::{
    build_outbound_webhook_updated_envelope, OutboundWebhookUpdatedPayload,
    EVENT_TYPE_OUTBOUND_WEBHOOK_UPDATED,
};
pub use sync_authority_changed::{
    build_sync_authority_changed_envelope, SyncAuthorityChangedPayload,
    EVENT_TYPE_SYNC_AUTHORITY_CHANGED,
};
pub use sync_push_failed::{
    build_sync_push_failed_envelope, SyncPushFailedPayload, EVENT_TYPE_SYNC_PUSH_FAILED,
};
pub use webhook_received::{
    build_webhook_received_envelope, WebhookReceivedPayload, EVENT_TYPE_WEBHOOK_RECEIVED,
};
pub use webhook_routed::{
    build_webhook_routed_envelope, WebhookRoutedPayload, EVENT_TYPE_WEBHOOK_ROUTED,
};
