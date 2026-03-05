//! Structured security audit logging.
//!
//! Provides [`SecurityOutcome`] and [`security_event`] for consistent,
//! machine-parseable security event logging across all authorization paths.

use uuid::Uuid;

/// Outcome of a security-relevant decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecurityOutcome {
    Allowed,
    Denied,
    Error,
}

impl SecurityOutcome {
    pub fn as_str(&self) -> &'static str {
        match self {
            SecurityOutcome::Allowed => "allowed",
            SecurityOutcome::Denied => "denied",
            SecurityOutcome::Error => "error",
        }
    }
}

/// Emit a structured security audit event via `tracing`.
///
/// All fields are stable and suitable for detection/alerting pipelines.
/// Consumers can filter on `target = "security_audit"` and parse the
/// structured fields without relying on message text.
///
/// # Arguments
///
/// * `tenant_id` — Tenant UUID, if known.
/// * `actor_id` — Actor (user/service) UUID, if known.
/// * `action` — The action being attempted (e.g. route path or permission name).
/// * `outcome` — Whether access was allowed, denied, or errored.
/// * `reason` — Human-readable explanation for the outcome.
pub fn security_event(
    tenant_id: Option<Uuid>,
    actor_id: Option<Uuid>,
    action: &str,
    outcome: SecurityOutcome,
    reason: &str,
) {
    let tenant = tenant_id
        .map(|id| id.to_string())
        .unwrap_or_else(|| "unknown".to_string());
    let actor = actor_id
        .map(|id| id.to_string())
        .unwrap_or_else(|| "unknown".to_string());

    match outcome {
        SecurityOutcome::Allowed => {
            tracing::info!(
                target: "security_audit",
                tenant_id = %tenant,
                actor_id = %actor,
                action = %action,
                outcome = "allowed",
                reason = %reason,
                "security event"
            );
        }
        SecurityOutcome::Denied => {
            tracing::warn!(
                target: "security_audit",
                tenant_id = %tenant,
                actor_id = %actor,
                action = %action,
                outcome = "denied",
                reason = %reason,
                "security event"
            );
        }
        SecurityOutcome::Error => {
            tracing::error!(
                target: "security_audit",
                tenant_id = %tenant,
                actor_id = %actor,
                action = %action,
                outcome = "error",
                reason = %reason,
                "security event"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};
    use tracing_subscriber::layer::SubscriberExt;

    /// Collects formatted log lines for assertion.
    #[derive(Clone, Default)]
    struct CapturedLogs(Arc<Mutex<Vec<String>>>);

    impl std::io::Write for CapturedLogs {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            let s = String::from_utf8_lossy(buf).to_string();
            self.0.lock().expect("test mutex").push(s);
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    impl CapturedLogs {
        fn lines(&self) -> Vec<String> {
            self.0.lock().expect("test mutex").clone()
        }
    }

    fn make_subscriber(
        logs: CapturedLogs,
    ) -> impl tracing::Subscriber + Send + Sync {
        let fmt_layer = tracing_subscriber::fmt::layer()
            .with_target(true)
            .with_writer(move || logs.clone())
            .with_ansi(false);
        tracing_subscriber::registry().with(fmt_layer)
    }

    #[test]
    fn security_event_emits_denied_with_stable_fields() {
        let logs = CapturedLogs::default();
        let subscriber = make_subscriber(logs.clone());

        let tenant = Uuid::new_v4();
        let actor = Uuid::new_v4();

        tracing::subscriber::with_default(subscriber, || {
            security_event(
                Some(tenant),
                Some(actor),
                "/api/invoices",
                SecurityOutcome::Denied,
                "missing permission ar.create",
            );
        });

        let output = logs.lines().join("");
        assert!(output.contains("security_audit"), "should target security_audit");
        assert!(output.contains(&tenant.to_string()), "should contain tenant_id");
        assert!(output.contains(&actor.to_string()), "should contain actor_id");
        assert!(output.contains("denied"), "should contain outcome=denied");
        assert!(output.contains("/api/invoices"), "should contain action");
        assert!(
            output.contains("missing permission ar.create"),
            "should contain reason"
        );
    }

    #[test]
    fn security_event_handles_unknown_ids() {
        let logs = CapturedLogs::default();
        let subscriber = make_subscriber(logs.clone());

        tracing::subscriber::with_default(subscriber, || {
            security_event(
                None,
                None,
                "/api/secret",
                SecurityOutcome::Denied,
                "no claims present",
            );
        });

        let output = logs.lines().join("");
        assert!(output.contains("unknown"), "missing IDs should show as unknown");
        assert!(output.contains("denied"));
    }

    #[test]
    fn security_event_emits_allowed() {
        let logs = CapturedLogs::default();
        let subscriber = make_subscriber(logs.clone());

        tracing::subscriber::with_default(subscriber, || {
            security_event(
                Some(Uuid::new_v4()),
                Some(Uuid::new_v4()),
                "/api/items",
                SecurityOutcome::Allowed,
                "all permissions satisfied",
            );
        });

        let output = logs.lines().join("");
        assert!(output.contains("allowed"));
    }

    #[test]
    fn security_event_emits_error() {
        let logs = CapturedLogs::default();
        let subscriber = make_subscriber(logs.clone());

        tracing::subscriber::with_default(subscriber, || {
            security_event(
                Some(Uuid::new_v4()),
                None,
                "/api/crash",
                SecurityOutcome::Error,
                "token decode failure",
            );
        });

        let output = logs.lines().join("");
        assert!(output.contains("error"));
        assert!(output.contains("token decode failure"));
    }

    #[test]
    fn security_outcome_as_str() {
        assert_eq!(SecurityOutcome::Allowed.as_str(), "allowed");
        assert_eq!(SecurityOutcome::Denied.as_str(), "denied");
        assert_eq!(SecurityOutcome::Error.as_str(), "error");
    }
}
