//! Structured logging helpers.
//!
//! The platform logging standard requires these fields per level:
//! - ERROR: message, error_code, tenant_id, request_id, actor_id, module
//! - WARN:  message, tenant_id, request_id, module
//! - INFO:  message, tenant_id, request_id, module (actor_id optional)
//! - DEBUG: message, any additional context
//!
//! In HTTP handlers, `platform_trace_middleware` injects a span that carries all
//! required fields automatically.  Use [`request_span`] only in consumers,
//! background tasks, or test code where no HTTP middleware is running.

/// Create a tracing span pre-populated with the required contextual fields.
///
/// Enter the span (or pass it to `.instrument(...)`) before logging; child log
/// events then inherit `tenant_id`, `request_id`, `actor_id`, and `module`.
///
/// # Example
///
/// ```rust
/// use platform_sdk::logging::request_span;
/// use tracing::Instrument as _;
///
/// let span = request_span("inventory", "tenant-abc", "req-123", "user-456");
/// async move {
///     tracing::info!("processing event");
/// }.instrument(span);
/// ```
pub fn request_span(
    module: &str,
    tenant_id: &str,
    request_id: &str,
    actor_id: &str,
) -> tracing::Span {
    tracing::info_span!(
        "request",
        module    = %module,
        tenant_id = %tenant_id,
        request_id = %request_id,
        actor_id  = %actor_id,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_span_is_non_disabled() {
        // Initialise a minimal subscriber so spans can be created.
        let _ = tracing_subscriber::fmt()
            .with_max_level(tracing::Level::DEBUG)
            .try_init();

        let span = request_span("inventory", "tenant-abc", "req-123", "user-456");
        // A non-disabled span was created.
        assert!(
            !span.is_disabled(),
            "request_span must create an enabled span"
        );
    }

    #[test]
    fn request_span_metadata_name() {
        let _ = tracing_subscriber::fmt()
            .with_max_level(tracing::Level::DEBUG)
            .try_init();

        let span = request_span("my-module", "t1", "r1", "a1");
        // The span is named "request" per the platform standard.
        assert_eq!(span.metadata().map(|m| m.name()), Some("request"));
    }

    /// Verify the logging standard fields are present in the span's metadata.
    ///
    /// This is the canonical test referenced by the bead verify command:
    ///   `cargo test -p platform-sdk logging_standard -- --nocapture`
    #[test]
    fn logging_standard_span_fields() {
        let _ = tracing_subscriber::fmt()
            .with_max_level(tracing::Level::DEBUG)
            .try_init();

        let span = request_span("inventory", "tenant-abc", "req-xyz", "user-789");

        let meta = span.metadata().expect("span must have metadata");

        // Verify all required contextual fields are recorded in the span.
        let field_names: Vec<&str> = meta.fields().iter().map(|f| f.name()).collect();
        assert!(field_names.contains(&"module"), "span must record 'module'");
        assert!(
            field_names.contains(&"tenant_id"),
            "span must record 'tenant_id'"
        );
        assert!(
            field_names.contains(&"request_id"),
            "span must record 'request_id'"
        );
        assert!(
            field_names.contains(&"actor_id"),
            "span must record 'actor_id'"
        );
    }

    #[test]
    fn logging_standard_empty_fields_produce_valid_span() {
        let _ = tracing_subscriber::fmt()
            .with_max_level(tracing::Level::DEBUG)
            .try_init();

        // Empty strings are valid (unauthenticated / background tasks).
        let span = request_span("scheduler", "", "", "");
        assert!(!span.is_disabled());
    }
}
