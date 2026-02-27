//! Tilled sandbox test harness.
//!
//! Tests in this module run against the real Tilled sandbox API.
//! They are gated on `TILLED_SECRET_KEY` and `TILLED_ACCOUNT_ID` env vars:
//! - Present → tests run normally
//! - Absent  → tests skip with a clear message (in local dev)
//!             CI sets these as repository secrets so they always run there.
//!
//! Optional platform-level account support:
//! - `TILLED_PARTNER_ACCOUNT_ID` can be used when a test needs partner-scope APIs.

pub mod helpers;
pub mod scenarios;
pub mod scenarios_account_self;
pub mod scenarios_balance;
pub mod scenarios_checkout;
pub mod scenarios_coverage_gaps;
pub mod scenarios_disputes;
pub mod scenarios_documents;
pub mod scenarios_events;
pub mod scenarios_files;
pub mod scenarios_merchants;
pub mod scenarios_misc_gaps;
pub mod scenarios_onboarding;
pub mod scenarios_ops;
pub mod scenarios_payment_intents;
pub mod scenarios_payments;
pub mod scenarios_platform_fees;
pub mod scenarios_pricing;
pub mod scenarios_reports;
pub mod scenarios_subscription_lifecycle;
pub mod scenarios_subscriptions;
pub mod scenarios_user_invitations;
pub mod scenarios_users;
pub mod scenarios_webhooks;

use ar_rs::tilled::{TilledClient, TilledConfig};

fn build_sandbox_client(
    secret_key: Option<String>,
    account_id: Option<String>,
    webhook_secret: Option<String>,
) -> Option<TilledClient> {
    let secret_key = secret_key?;
    let account_id = account_id?;
    let webhook_secret = webhook_secret.unwrap_or_else(|| "not-set".to_string());

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

/// Check whether sandbox credentials are available.
/// Returns `None` (skip) when creds are missing.
pub fn try_sandbox_client() -> Option<TilledClient> {
    build_sandbox_client(
        std::env::var("TILLED_SECRET_KEY").ok(),
        std::env::var("TILLED_ACCOUNT_ID").ok(),
        std::env::var("TILLED_WEBHOOK_SECRET").ok(),
    )
}

/// Build a partner-scope sandbox client when platform-level credentials are available.
/// Returns `None` when `TILLED_PARTNER_ACCOUNT_ID` is not configured.
pub fn try_partner_client() -> Option<TilledClient> {
    build_sandbox_client(
        std::env::var("TILLED_SECRET_KEY").ok(),
        std::env::var("TILLED_PARTNER_ACCOUNT_ID").ok(),
        std::env::var("TILLED_WEBHOOK_SECRET").ok(),
    )
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
        assert!(build_sandbox_client(None, Some("acct_123".to_string()), None).is_none());
        assert!(build_sandbox_client(Some("sk_123".to_string()), None, None).is_none());
    }

    #[test]
    fn try_partner_client_returns_none_without_partner_account() {
        assert!(build_sandbox_client(None, Some("acct_partner".to_string()), None).is_none());
        assert!(build_sandbox_client(Some("sk_123".to_string()), None, None).is_none());
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
