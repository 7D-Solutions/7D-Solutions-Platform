//! Bootstrap tests for the Tilled sandbox harness.
//! These verify harness infrastructure (retry policy, data generators, skip logic).
//! The real scenario tests are in a separate file (added by bd-39vr).

mod tilled_sandbox;

// Re-export the macro so it's available in scenario tests that also
// include `mod tilled_sandbox;`.

// Bootstrap tests live inside tilled_sandbox/mod.rs.
// This file exists to ensure they compile and run with:
//   cargo test -p ar-rs --test tilled_sandbox_tests
