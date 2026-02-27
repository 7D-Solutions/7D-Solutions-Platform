//! Sandbox scenarios: subscription lifecycle — create, update, multi-sub, trial.

#[cfg(test)]
mod tests {
    use crate::tilled_sandbox::helpers::{
        cleanup_customer, cleanup_payment_method, cleanup_subscription,
        try_create_test_payment_method, unique_email, RetryPolicy,
    };
    use crate::tilled_sandbox::try_sandbox_client;
    use ar_rs::tilled::subscription::{SubscriptionOptions, UpdateSubscriptionRequest};
    use std::collections::HashMap;

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
                    c.create_customer(e, Some("Sub Test".to_string()), None)
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

    // -----------------------------------------------------------------------
    // Scenario S1: Create + verify + cancel with metadata and interval checks
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn scenario_s1_create_verify_cancel_with_metadata() {
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

        let mut meta = HashMap::new();
        meta.insert("plan".to_string(), "premium".to_string());
        meta.insert("test_run".to_string(), uuid::Uuid::new_v4().to_string());

        let sub = retry
            .execute(|| {
                let c = client.clone();
                let cid = cust_id.clone();
                let pid = pm_id.clone();
                let m = meta.clone();
                async move {
                    c.create_subscription(
                        cid,
                        pid,
                        1999,
                        Some(SubscriptionOptions {
                            interval_unit: Some("month".to_string()),
                            interval_count: Some(1),
                            metadata: Some(m),
                            ..Default::default()
                        }),
                    )
                    .await
                }
            })
            .await
            .expect("create_subscription failed");

        eprintln!(
            "[scenario-s1] sub={} status={} price={:?}",
            sub.id, sub.status, sub.price
        );
        assert!(!sub.id.is_empty());
        assert!(
            sub.status == "active" || sub.status == "trialing" || sub.status == "pending",
            "expected active/trialing/pending, got: {}",
            sub.status
        );
        assert_eq!(sub.price, Some(1999));
        assert_eq!(sub.customer_id.as_deref(), Some(cust_id.as_str()));
        assert_eq!(sub.interval_unit.as_deref(), Some("month"));
        assert_eq!(sub.interval_count, Some(1));

        // Verify via GET
        let fetched = retry
            .execute(|| {
                let c = client.clone();
                let sid = sub.id.clone();
                async move { c.get_subscription(&sid).await }
            })
            .await
            .expect("get_subscription failed");
        assert_eq!(fetched.id, sub.id);
        assert_eq!(fetched.price, Some(1999));

        // Cancel
        let canceled = retry
            .execute(|| {
                let c = client.clone();
                let sid = sub.id.clone();
                async move { c.cancel_subscription(&sid).await }
            })
            .await
            .expect("cancel_subscription failed");

        eprintln!(
            "[scenario-s1] canceled: {} status={}",
            canceled.id, canceled.status
        );
        assert_eq!(canceled.id, sub.id);
        assert!(
            canceled.status == "canceled"
                || canceled.canceled_at.is_some()
                || canceled.cancel_at_period_end,
            "expected terminal state, got status={}",
            canceled.status
        );

        // Verify terminal via GET
        let final_state = retry
            .execute(|| {
                let c = client.clone();
                let sid = sub.id.clone();
                async move { c.get_subscription(&sid).await }
            })
            .await
            .expect("get_subscription after cancel failed");
        assert!(
            final_state.status == "canceled" || final_state.canceled_at.is_some(),
            "subscription should be terminal, got: {}",
            final_state.status
        );

        cleanup_payment_method(&client, &pm_id).await;
        cleanup_customer(&client, &cust_id).await;
    }

    // -----------------------------------------------------------------------
    // Scenario S2: Update subscription metadata
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn scenario_s2_update_subscription_metadata() {
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

        let sub = retry
            .execute(|| {
                let c = client.clone();
                let cid = cust_id.clone();
                let pid = pm_id.clone();
                async move { c.create_subscription(cid, pid, 999, None).await }
            })
            .await
            .expect("create_subscription failed");

        eprintln!("[scenario-s2] sub={} status={}", sub.id, sub.status);

        // Update metadata
        let mut new_meta = HashMap::new();
        new_meta.insert("tier".to_string(), "gold".to_string());

        let updated = retry
            .execute(|| {
                let c = client.clone();
                let sid = sub.id.clone();
                let m = new_meta.clone();
                async move {
                    c.update_subscription(
                        &sid,
                        UpdateSubscriptionRequest {
                            payment_method_id: None,
                            metadata: Some(m),
                            cancel_at_period_end: None,
                        },
                    )
                    .await
                }
            })
            .await
            .expect("update_subscription failed");

        eprintln!(
            "[scenario-s2] updated sub={} metadata={:?}",
            updated.id, updated.metadata
        );
        assert_eq!(updated.id, sub.id);

        // Verify via GET
        let fetched = retry
            .execute(|| {
                let c = client.clone();
                let sid = sub.id.clone();
                async move { c.get_subscription(&sid).await }
            })
            .await
            .expect("get_subscription failed");

        if let Some(meta) = &fetched.metadata {
            assert_eq!(meta.get("tier").map(String::as_str), Some("gold"));
        }

        cleanup_subscription(&client, &sub.id).await;
        cleanup_payment_method(&client, &pm_id).await;
        cleanup_customer(&client, &cust_id).await;
    }

    // -----------------------------------------------------------------------
    // Scenario S3: Multiple subscriptions per customer
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn scenario_s3_multiple_subscriptions_per_customer() {
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

        // Sub 1: $9.99/month
        let sub1 = retry
            .execute(|| {
                let c = client.clone();
                let cid = cust_id.clone();
                let pid = pm_id.clone();
                async move { c.create_subscription(cid, pid, 999, None).await }
            })
            .await
            .expect("create sub1 failed");

        // Sub 2: $29.99/month
        let sub2 = retry
            .execute(|| {
                let c = client.clone();
                let cid = cust_id.clone();
                let pid = pm_id.clone();
                async move { c.create_subscription(cid, pid, 2999, None).await }
            })
            .await
            .expect("create sub2 failed");

        eprintln!(
            "[scenario-s3] sub1={} sub2={} customer={}",
            sub1.id, sub2.id, cust_id
        );
        assert_ne!(sub1.id, sub2.id);

        // List subscriptions for customer
        let mut filters = HashMap::new();
        filters.insert("customer_id".to_string(), cust_id.clone());
        let list = retry
            .execute(|| {
                let c = client.clone();
                let f = filters.clone();
                async move { c.list_subscriptions(Some(f)).await }
            })
            .await
            .expect("list_subscriptions failed");

        eprintln!(
            "[scenario-s3] found {} subscriptions for customer",
            list.items.len()
        );
        assert!(
            list.items.len() >= 2,
            "expected at least 2 subs, got {}",
            list.items.len()
        );
        let ids: Vec<&str> = list.items.iter().map(|s| s.id.as_str()).collect();
        assert!(ids.contains(&sub1.id.as_str()), "sub1 not found in list");
        assert!(ids.contains(&sub2.id.as_str()), "sub2 not found in list");

        // Cleanup
        cleanup_subscription(&client, &sub1.id).await;
        cleanup_subscription(&client, &sub2.id).await;
        cleanup_payment_method(&client, &pm_id).await;
        cleanup_customer(&client, &cust_id).await;
    }

    // -----------------------------------------------------------------------
    // Scenario S4: Subscription with trial period
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn scenario_s4_subscription_with_trial() {
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

        // Trial end: 7 days from now (unix timestamp)
        let trial_end = chrono::Utc::now().timestamp() + (7 * 24 * 3600);

        let sub = match retry
            .execute(|| {
                let c = client.clone();
                let cid = cust_id.clone();
                let pid = pm_id.clone();
                async move {
                    c.create_subscription(
                        cid,
                        pid,
                        1499,
                        Some(SubscriptionOptions {
                            trial_end: Some(trial_end),
                            ..Default::default()
                        }),
                    )
                    .await
                }
            })
            .await
        {
            Ok(sub) => sub,
            Err(ar_rs::tilled::error::TilledError::ApiError {
                status_code: 400,
                message,
            }) if message.contains("trial") => {
                eprintln!("SKIP: sandbox does not support trial subscriptions: {message}");
                cleanup_payment_method(&client, &pm_id).await;
                cleanup_customer(&client, &cust_id).await;
                return;
            }
            Err(e) => panic!("create_subscription with trial failed: {e}"),
        };

        eprintln!(
            "[scenario-s4] sub={} status={} trial_end={:?}",
            sub.id, sub.status, sub.trial_end
        );

        assert!(!sub.id.is_empty());
        // With trial, status might be trialing, pending, or active
        assert!(
            sub.status == "trialing" || sub.status == "active" || sub.status == "pending",
            "expected trialing/active/pending with trial, got: {}",
            sub.status
        );

        // Verify via GET
        let fetched = retry
            .execute(|| {
                let c = client.clone();
                let sid = sub.id.clone();
                async move { c.get_subscription(&sid).await }
            })
            .await
            .expect("get_subscription failed");
        assert_eq!(fetched.id, sub.id);

        cleanup_subscription(&client, &sub.id).await;
        cleanup_payment_method(&client, &pm_id).await;
        cleanup_customer(&client, &cust_id).await;
    }
}
