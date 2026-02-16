/// Platform-level audit primitives
///
/// This crate provides shared audit infrastructure for recording
/// who did what, when, and why across the platform.

pub mod actor;
pub mod policy;
pub mod diff;
pub mod schema;
pub mod writer;
