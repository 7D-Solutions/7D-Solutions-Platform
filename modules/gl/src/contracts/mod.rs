//! Contract types for GL module events
//!
//! This module contains types that match the JSON schema definitions
//! for events consumed by the GL module.

pub mod gl_posting_request_v1;
pub mod gl_entry_reverse_request_v1;

pub use gl_posting_request_v1::*;
pub use gl_entry_reverse_request_v1::*;
