//! Inbound webhook domain — signature verification, idempotency, routing.

pub mod models;
pub mod routing;
pub mod service;
pub mod verify;

pub use models::{IngestResult, IngestWebhookRequest, WebhookError, WebhookIngest};
pub use service::WebhookService;
