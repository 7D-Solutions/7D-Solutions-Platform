//! Platform-level audit primitives
//!
//! This crate provides shared audit infrastructure for recording
//! who did what, when, and why across the platform.
//!
//! ## Oracle Integration
//!
//! The E2E test oracle (e2e-tests/tests/oracle.rs) validates that:
//! - Every mutation in module outbox tables has exactly one audit record
//! - Audit records are linked via causation_id to the originating event
//! - No gaps or duplicates exist in the audit trail

pub mod actor;
pub mod policy;
pub mod diff;
pub mod schema;
pub mod writer;
pub mod outbox_bridge;
