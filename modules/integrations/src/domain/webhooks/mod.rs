//! Inbound webhook domain — signature verification, idempotency, routing.

pub mod models;
pub mod qbo_normalizer;
pub mod routing;
pub mod service;
pub mod verify;

pub use models::{IngestResult, IngestWebhookRequest, WebhookError, WebhookIngest};
pub use qbo_normalizer::QboNormalizer;
pub use service::WebhookService;
