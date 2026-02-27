//! Tilled sandbox test harness.
//!
//! Tests in this module run against the real Tilled sandbox API.
//! They are gated on `TILLED_SECRET_KEY` and `TILLED_ACCOUNT_ID` env vars:
//! - Present → tests run normally
//! - Absent  → tests skip with a clear message (in local dev)
//!             CI sets these as repository secrets so they always run there.

pub mod helpers;

use ar_rs::tilled::{TilledClient, TilledConfig};

/// Check whether sandbox credentials are available.
/// Returns `None` (skip) when creds are missing.
pub fn try_sandbox_client() -> Option<TilledClient> {
    let secret_key = std::env::var("TILLED_SECRET_KEY").ok()?;
    let account_id = std::env::var("TILLED_ACCOUNT_ID").ok()?;
    let webhook_secret =
        std::env::var("TILLED_WEBHOOK_SECRET").unwrap_or_else(|_| "not-set".to_string());

    if secret_key.is_empty() || account_id.is_empty() {
        return None;
    }

    let config = TilledConfig {
        secret_key,
        account_id,
        webhook_secret,
        sandbox: true,
        base_path: "https://sandbox-api.tilled.com".to_string(),
    };

    TilledClient::new(config).ok()
}

/// Macro to skip a test when sandbox credentials are not available.
/// Usage: `let client = require_sandbox!();`
#[macro_export]
macro_rules! require_sandbox {
    () => {
        match $crate::tilled_sandbox::try_sandbox_client() {
            Some(c) => c,
            None => {
                eprintln!(
                    "SKIP: TILLED_SECRET_KEY / TILLED_ACCOUNT_ID not set — \
                     sandbox test skipped. Set these env vars to run."
                );
                return;
            }
        }
    };
}

// ---------------------------------------------------------------------------
// Bootstrap tests — verify the harness itself works
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use helpers::{unique_email, unique_metadata, RetryPolicy};

    #[test]
    fn unique_email_generates_uuid_based_address() {
        let e1 = unique_email();
        let e2 = unique_email();
        assert_ne!(e1, e2);
        assert!(e1.contains("sandbox-test-"));
        assert!(e1.ends_with("@7d-test.example.com"));
    }

    #[test]
    fn unique_metadata_contains_test_run_key() {
        let meta = unique_metadata();
        assert!(meta["test_run"].is_string());
        assert_eq!(meta["harness"], "tilled_sandbox");
    }

    #[test]
    fn try_sandbox_client_returns_none_without_creds() {
        // Save and clear
        let saved_sk = std::env::var("TILLED_SECRET_KEY").ok();
        let saved_acct = std::env::var("TILLED_ACCOUNT_ID").ok();
        std::env::remove_var("TILLED_SECRET_KEY");
        std::env::remove_var("TILLED_ACCOUNT_ID");

        assert!(try_sandbox_client().is_none());

        // Restore
        if let Some(v) = saved_sk {
            std::env::set_var("TILLED_SECRET_KEY", v);
        }
        if let Some(v) = saved_acct {
            std::env::set_var("TILLED_ACCOUNT_ID", v);
        }
    }

    #[tokio::test]
    async fn retry_policy_returns_ok_on_first_success() {
        let policy = RetryPolicy::default();
        let result: Result<i32, ar_rs::tilled::error::TilledError> =
            policy.execute(|| async { Ok(42) }).await;
        assert_eq!(result.unwrap(), 42);
    }

    #[tokio::test]
    async fn retry_policy_fails_fast_on_client_error() {
        let policy = RetryPolicy::default();
        let mut attempts = 0u32;
        let result: Result<i32, ar_rs::tilled::error::TilledError> = policy
            .execute(|| {
                attempts += 1;
                async {
                    Err(ar_rs::tilled::error::TilledError::ApiError {
                        status_code: 400,
                        message: "bad request".to_string(),
                    })
                }
            })
            .await;
        assert!(result.is_err());
        assert_eq!(attempts, 1, "should not retry 4xx errors");
    }
}
