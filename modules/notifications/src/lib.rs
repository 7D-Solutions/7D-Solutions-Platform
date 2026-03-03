//! Notifications module — library crate.
//!
//! Exposes public modules so E2E tests and integration consumers can call
//! handler functions directly without going through NATS.

pub mod broadcast;
pub mod config;
pub mod consumers;
pub mod consumer_tasks;
pub mod db;
pub mod dlq;
pub mod envelope_validation;
pub mod escalation;
pub mod event_bus;
pub mod handlers;
pub mod inbox;
pub mod metrics;
pub mod models;
pub mod http;
pub mod scheduled;
pub mod templates;
