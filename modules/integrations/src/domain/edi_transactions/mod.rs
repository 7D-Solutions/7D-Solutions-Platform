//! EDI transaction set framework — durable records with validation pipeline tracking.

pub mod guards;
pub mod models;
pub mod repo;
pub mod service;

pub use models::{
    CreateOutboundEdiRequest, EdiTransaction, EdiTransactionError, IngestEdiRequest,
    TransitionEdiRequest,
};
pub use service::EdiTransactionService;
