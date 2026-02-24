//! PDF Editor module — library crate.
//!
//! Exposes public modules so E2E tests and integration consumers can call
//! handler functions directly.

pub mod config;
pub mod db;
pub mod domain;
pub mod event_bus;
pub mod metrics;
pub mod routes;
