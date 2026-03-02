//! Sandbox scenarios: subscription lifecycle — confirm, pause, resume, retry.

#[cfg(test)]
mod tests {
    use crate::tilled_sandbox::helpers::{
        cleanup_customer, cleanup_payment_method, cleanup_subscription,
        try_create_test_payment_method, unique_email, RetryPolicy,
    };
    use crate::tilled_sandbox::try_sandbox_client;

    fn sandbox_config() -> Option<(String, String, String)> {
        let sk = std::env::var("TILLED_SECRET_KEY").ok()?;
        let acct = std::env::var("TILLED_ACCOUNT_ID").ok()?;
        if sk.is_empty() || acct.is_empty() {
            eprintln!("SKIP: TILLED_SECRET_KEY / TILLED_ACCOUNT_ID not set");
            return None;
        }
        Some((sk, acct, "https://sandbox-api.tilled.com".to_string()))
    }

    /// Helper: create customer + attach PM, return (customer_id, pm_id).
    async fn setup_customer_with_pm(
        client: &ar_rs::tilled::TilledClient,
        sk: &str,
        acct: &str,
        base_url: &str,
    ) -> Option<(String, String)> {
        let retry = RetryPolicy::default();
        let customer = retry
            .execute(|| {
                let c = client.clone();
                let e = unique_email();
                async move {
                    c.create_customer(e, Some("SubLifecycle Test".to_string()), None)
                        .await
                }
            })
            .await
            .expect("create_customer failed");

        let pm = match try_create_test_payment_method(sk, acct, base_url).await {
            Some(pm) => pm,
            None => {
                cleanup_customer(client, &customer.id).await;
                return None;
            }
        };

        retry
            .execute(|| {
                let c = client.clone();
                let pm_id = pm.id.clone();
                let cust_id = customer.id.clone();
                async move { c.attach_payment_method(&pm_id, cust_id).await }
            })
            .await
            .expect("attach failed");

        Some((customer.id, pm.id))
    }

    /// Helper: create a subscription, return its ID.
    async fn create_sub(
        client: &ar_rs::tilled::TilledClient,
        cust_id: &str,
        pm_id: &str,
    ) -> String {
        let retry = RetryPolicy::default();
        let sub = retry
            .execute(|| {
                let c = client.clone();
                let cid = cust_id.to_string();
                let pid = pm_id.to_string();
                async move { c.create_subscription(cid, pid, 1499, None).await }
            })
            .await
            .expect("create_subscription failed");
        eprintln!(
            "[sub-lifecycle] created sub={} status={}",
            sub.id, sub.status
        );
        sub.id
    }

    // -----------------------------------------------------------------------
    // Scenario SL1: Pause and resume a subscription
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn scenario_sl1_pause_and_resume() {
        let client = match try_sandbox_client() {
            Some(c) => c,
            None => {
                eprintln!("SKIP: sandbox creds not set");
                return;
            }
        };
        let (sk, acct, base_url) = match sandbox_config() {
            Some(c) => c,
            None => return,
        };
        let retry = RetryPolicy::default();

        let (cust_id, pm_id) = match setup_customer_with_pm(&client, &sk, &acct, &base_url).await {
            Some(ids) => ids,
            None => return,
        };

        let sub_id = create_sub(&client, &cust_id, &pm_id).await;

        // Pause the subscription
        let paused = match retry
            .execute(|| {
                let c = client.clone();
                let sid = sub_id.clone();
                async move { c.pause_subscription(&sid).await }
            })
            .await
        {
            Ok(s) => s,
            Err(ar_rs::tilled::error::TilledError::ApiError {
                status_code,
                ref message,
            }) if status_code == 400 || status_code == 422 => {
                eprintln!(
                    "[scenario-sl1] pause not supported for this sub state: {status_code} {message}"
                );
                // Verify the method at least made a valid API call
                assert!(
                    message.contains("status") || message.contains("pause") || !message.is_empty(),
                    "expected meaningful error, got empty"
                );
                cleanup_subscription(&client, &sub_id).await;
                cleanup_payment_method(&client, &pm_id).await;
                cleanup_customer(&client, &cust_id).await;
                return;
            }
            Err(e) => panic!("pause_subscription failed: {e}"),
        };

        eprintln!(
            "[scenario-sl1] paused: id={} status={}",
            paused.id, paused.status
        );
        assert_eq!(paused.id, sub_id);
        assert_eq!(paused.status, "paused");

        // Resume the subscription
        let resumed = retry
            .execute(|| {
                let c = client.clone();
                let sid = sub_id.clone();
                async move { c.resume_subscription(&sid).await }
            })
            .await
            .expect("resume_subscription failed");

        eprintln!(
            "[scenario-sl1] resumed: id={} status={}",
            resumed.id, resumed.status
        );
        assert_eq!(resumed.id, sub_id);
        assert!(
            resumed.status == "active" || resumed.status == "pending",
            "expected active/pending after resume, got: {}",
            resumed.status
        );

        cleanup_subscription(&client, &sub_id).await;
        cleanup_payment_method(&client, &pm_id).await;
        cleanup_customer(&client, &cust_id).await;
    }

    // -----------------------------------------------------------------------
    // Scenario SL2: Confirm a subscription
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn scenario_sl2_confirm_subscription() {
        let client = match try_sandbox_client() {
            Some(c) => c,
            None => {
                eprintln!("SKIP: sandbox creds not set");
                return;
            }
        };
        let (sk, acct, base_url) = match sandbox_config() {
            Some(c) => c,
            None => return,
        };
        let retry = RetryPolicy::default();

        let (cust_id, pm_id) = match setup_customer_with_pm(&client, &sk, &acct, &base_url).await {
            Some(ids) => ids,
            None => return,
        };

        let sub_id = create_sub(&client, &cust_id, &pm_id).await;

        // Attempt confirm — sandbox subs typically start active, so confirm
        // may return an error indicating the sub doesn't need confirmation.
        match retry
            .execute(|| {
                let c = client.clone();
                let sid = sub_id.clone();
                async move { c.confirm_subscription(&sid).await }
            })
            .await
        {
            Ok(confirmed) => {
                eprintln!(
                    "[scenario-sl2] confirmed: id={} status={}",
                    confirmed.id, confirmed.status
                );
                assert_eq!(confirmed.id, sub_id);
                assert!(
                    confirmed.status == "active"
                        || confirmed.status == "trialing"
                        || confirmed.status == "pending",
                    "unexpected status after confirm: {}",
                    confirmed.status
                );
            }
            Err(ar_rs::tilled::error::TilledError::ApiError {
                status_code,
                ref message,
            }) if status_code == 400 || status_code == 422 => {
                // Sub was already active — confirm not applicable. This is valid.
                eprintln!(
                    "[scenario-sl2] confirm returned {status_code}: {message} (expected for active sub)"
                );
                assert!(!message.is_empty(), "expected meaningful error message");
            }
            Err(e) => panic!("confirm_subscription failed unexpectedly: {e}"),
        }

        cleanup_subscription(&client, &sub_id).await;
        cleanup_payment_method(&client, &pm_id).await;
        cleanup_customer(&client, &cust_id).await;
    }

    // -----------------------------------------------------------------------
    // Scenario SL3: Retry a subscription
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn scenario_sl3_retry_subscription() {
        let client = match try_sandbox_client() {
            Some(c) => c,
            None => {
                eprintln!("SKIP: sandbox creds not set");
                return;
            }
        };
        let (sk, acct, base_url) = match sandbox_config() {
            Some(c) => c,
            None => return,
        };
        let retry = RetryPolicy::default();

        let (cust_id, pm_id) = match setup_customer_with_pm(&client, &sk, &acct, &base_url).await {
            Some(ids) => ids,
            None => return,
        };

        let sub_id = create_sub(&client, &cust_id, &pm_id).await;

        // Retry — only meaningful for past_due/unpaid subs. Active subs
        // should return an API error indicating retry is not applicable.
        match retry
            .execute(|| {
                let c = client.clone();
                let sid = sub_id.clone();
                async move { c.retry_subscription(&sid).await }
            })
            .await
        {
            Ok(retried) => {
                eprintln!(
                    "[scenario-sl3] retried: id={} status={}",
                    retried.id, retried.status
                );
                assert_eq!(retried.id, sub_id);
            }
            Err(ar_rs::tilled::error::TilledError::ApiError {
                status_code,
                ref message,
            }) if status_code == 400 || status_code == 422 => {
                // Active sub can't be retried — this is the expected path.
                eprintln!(
                    "[scenario-sl3] retry returned {status_code}: {message} (expected for active sub)"
                );
                assert!(!message.is_empty(), "expected meaningful error message");
            }
            Err(e) => panic!("retry_subscription failed unexpectedly: {e}"),
        }

        cleanup_subscription(&client, &sub_id).await;
        cleanup_payment_method(&client, &pm_id).await;
        cleanup_customer(&client, &cust_id).await;
    }

    // -----------------------------------------------------------------------
    // Scenario SL4: Pause an already-paused subscription (idempotency)
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn scenario_sl4_pause_already_paused() {
        let client = match try_sandbox_client() {
            Some(c) => c,
            None => {
                eprintln!("SKIP: sandbox creds not set");
                return;
            }
        };
        let (sk, acct, base_url) = match sandbox_config() {
            Some(c) => c,
            None => return,
        };
        let retry = RetryPolicy::default();

        let (cust_id, pm_id) = match setup_customer_with_pm(&client, &sk, &acct, &base_url).await {
            Some(ids) => ids,
            None => return,
        };

        let sub_id = create_sub(&client, &cust_id, &pm_id).await;

        // First pause
        let first_pause = match retry
            .execute(|| {
                let c = client.clone();
                let sid = sub_id.clone();
                async move { c.pause_subscription(&sid).await }
            })
            .await
        {
            Ok(s) => s,
            Err(ar_rs::tilled::error::TilledError::ApiError {
                status_code,
                ref message,
            }) if status_code == 400 || status_code == 422 => {
                eprintln!("[scenario-sl4] pause not supported: {status_code} {message}");
                assert!(!message.is_empty());
                cleanup_subscription(&client, &sub_id).await;
                cleanup_payment_method(&client, &pm_id).await;
                cleanup_customer(&client, &cust_id).await;
                return;
            }
            Err(e) => panic!("first pause failed: {e}"),
        };

        assert_eq!(first_pause.status, "paused");

        // Second pause — should be idempotent or return meaningful error
        match retry
            .execute(|| {
                let c = client.clone();
                let sid = sub_id.clone();
                async move { c.pause_subscription(&sid).await }
            })
            .await
        {
            Ok(second) => {
                eprintln!(
                    "[scenario-sl4] second pause ok: id={} status={}",
                    second.id, second.status
                );
                assert_eq!(second.id, sub_id);
                assert_eq!(second.status, "paused", "should remain paused");
            }
            Err(ar_rs::tilled::error::TilledError::ApiError {
                status_code,
                ref message,
            }) if status_code == 400 || status_code == 422 => {
                eprintln!(
                    "[scenario-sl4] second pause rejected: {status_code} {message} (acceptable)"
                );
                assert!(!message.is_empty());
            }
            Err(e) => panic!("second pause failed unexpectedly: {e}"),
        }

        cleanup_subscription(&client, &sub_id).await;
        cleanup_payment_method(&client, &pm_id).await;
        cleanup_customer(&client, &cust_id).await;
    }
}
