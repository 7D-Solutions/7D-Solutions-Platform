//! Outbound webhook management — tenant-scoped webhook subscriptions,
//! delivery audit logging, and signing secret lifecycle.

pub mod guards;
pub mod models;
pub mod repo;
pub mod service;

pub use models::{
    CreateOutboundWebhookRequest, OutboundWebhook, OutboundWebhookDelivery, OutboundWebhookError,
    RecordDeliveryRequest, UpdateOutboundWebhookRequest,
};
pub use service::OutboundWebhookService;
