//! Sandbox scenarios: dispute evidence, subscription lifecycle, detach guard.
//!
//! Continuation of scenarios.rs — split to stay under 500 LOC.

#[cfg(test)]
mod tests {
    use crate::tilled_sandbox::helpers::{
        cleanup_customer, cleanup_payment_method, cleanup_subscription,
        try_create_test_payment_method, unique_email, RetryPolicy,
    };
    use crate::tilled_sandbox::try_sandbox_client;
    use ar_rs::tilled::subscription::SubscriptionOptions;

    fn sandbox_config() -> Option<(String, String, String)> {
        let sk = std::env::var("TILLED_SECRET_KEY").ok()?;
        let acct = std::env::var("TILLED_ACCOUNT_ID").ok()?;
        if sk.is_empty() || acct.is_empty() {
            eprintln!("SKIP: TILLED_SECRET_KEY / TILLED_ACCOUNT_ID not set");
            return None;
        }
        Some((sk, acct, "https://sandbox-api.tilled.com".to_string()))
    }

    // -----------------------------------------------------------------------
    // Scenario 5: Dispute evidence submission
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn scenario_05_dispute_evidence_submission() {
        let client = match try_sandbox_client() {
            Some(c) => c,
            None => {
                eprintln!("SKIP: sandbox creds not set");
                return;
            }
        };

        // List existing disputes — sandbox cannot create disputes programmatically
        let disputes = client
            .list_disputes(None)
            .await
            .expect("list_disputes failed");

        if disputes.items.is_empty() {
            eprintln!(
                "SKIP: no disputes in sandbox account — \
                 Tilled sandbox does not support programmatic dispute creation."
            );
            return;
        }

        let eligible = disputes
            .items
            .iter()
            .find(|d| d.status == "warning_needs_response" || d.status == "needs_response");

        let dispute = match eligible {
            Some(d) => d,
            None => {
                eprintln!(
                    "SKIP: no evidence-eligible dispute (have {} disputes, statuses: {:?})",
                    disputes.items.len(),
                    disputes.items.iter().map(|d| &d.status).collect::<Vec<_>>()
                );
                return;
            }
        };

        eprintln!(
            "[scenario-05] submitting evidence for dispute: {} (status={})",
            dispute.id, dispute.status
        );

        let evidence = ar_rs::tilled::dispute::SubmitEvidenceRequest {
            description: Some(format!(
                "REVERSAL - automated sandbox test run {}",
                uuid::Uuid::new_v4()
            )),
            files: None,
        };

        let updated = match client.submit_dispute_evidence(&dispute.id, evidence).await {
            Ok(updated) => updated,
            Err(ar_rs::tilled::error::TilledError::ApiError {
                status_code: 400,
                message,
            }) if message.contains("Must provide at least one file") => {
                eprintln!(
                    "SKIP: dispute evidence API requires file uploads in this sandbox account"
                );
                return;
            }
            Err(e) => panic!("submit_dispute_evidence failed: {e}"),
        };

        eprintln!(
            "[scenario-05] evidence submitted, dispute status: {}",
            updated.status
        );
        assert_eq!(updated.id, dispute.id);
    }

    // -----------------------------------------------------------------------
    // Scenario 6: Subscription lifecycle (create -> verify -> cancel -> verify)
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn scenario_06_subscription_lifecycle() {
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

        let pm = match try_create_test_payment_method(&sk, &acct, &base_url).await {
            Some(pm) => pm,
            None => {
                cleanup_customer(&client, &customer.id).await;
                return;
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

        // Create subscription: $9.99/month
        let sub = retry
            .execute(|| {
                let c = client.clone();
                let cust_id = customer.id.clone();
                let pm_id = pm.id.clone();
                async move {
                    c.create_subscription(
                        cust_id,
                        pm_id,
                        999,
                        Some(SubscriptionOptions {
                            interval_unit: Some("month".to_string()),
                            interval_count: Some(1),
                            ..Default::default()
                        }),
                    )
                    .await
                }
            })
            .await
            .expect("create_subscription failed");

        eprintln!(
            "[scenario-06] sub created: {} status={}",
            sub.id, sub.status
        );
        assert!(!sub.id.is_empty());
        assert!(
            sub.status == "active" || sub.status == "trialing" || sub.status == "pending",
            "expected active/trialing/pending, got: {}",
            sub.status
        );
        assert_eq!(sub.customer_id.as_deref(), Some(customer.id.as_str()));
        assert_eq!(sub.price, Some(999));

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
            "[scenario-06] canceled: {} status={}",
            canceled.id, canceled.status
        );
        assert_eq!(canceled.id, sub.id);
        assert!(
            canceled.status == "canceled"
                || canceled.canceled_at.is_some()
                || canceled.cancel_at_period_end,
            "expected terminal state, got status={} cancel_at_period_end={}",
            canceled.status,
            canceled.cancel_at_period_end
        );

        // Verify terminal state via GET
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

        cleanup_payment_method(&client, &pm.id).await;
        cleanup_customer(&client, &customer.id).await;
    }

    // -----------------------------------------------------------------------
    // Scenario 7: Detach guard — PM used by active subscription
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn scenario_07_detach_guard_behavior() {
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

        let customer = retry
            .execute(|| {
                let c = client.clone();
                let e = unique_email();
                async move {
                    c.create_customer(e, Some("Detach Test".to_string()), None)
                        .await
                }
            })
            .await
            .expect("create_customer failed");

        let pm = match try_create_test_payment_method(&sk, &acct, &base_url).await {
            Some(pm) => pm,
            None => {
                cleanup_customer(&client, &customer.id).await;
                return;
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

        let sub = retry
            .execute(|| {
                let c = client.clone();
                let cust_id = customer.id.clone();
                let pm_id = pm.id.clone();
                async move { c.create_subscription(cust_id, pm_id, 500, None).await }
            })
            .await
            .expect("create_subscription failed");

        eprintln!(
            "[scenario-07] sub={} pm={} — attempting detach with active sub",
            sub.id, pm.id
        );

        // Attempt detach — Tilled may reject if PM backs active subscription
        let detach_result = client.detach_payment_method(&pm.id).await;

        match &detach_result {
            Err(ar_rs::tilled::error::TilledError::ApiError {
                status_code,
                message,
            }) => {
                eprintln!("[scenario-07] detach rejected: {status_code} — {message}");
                assert!(
                    (400..500).contains(status_code),
                    "expected 4xx for guarded detach, got {status_code}"
                );
            }
            Ok(_) => {
                // Some providers allow detach even with active subs.
                eprintln!(
                    "[scenario-07] NOTE: Tilled allowed detach with active sub \
                     — app-layer guard needed"
                );
            }
            Err(e) => panic!("unexpected error type on guarded detach: {e}"),
        }

        // Cancel subscription then detach
        cleanup_subscription(&client, &sub.id).await;
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;

        let detach_after = client.detach_payment_method(&pm.id).await;
        match detach_after {
            Ok(d) => {
                eprintln!("[scenario-07] detach after cancel succeeded: {}", d.id);
                assert_eq!(d.id, pm.id);
            }
            Err(e) => {
                // PM may already be detached if first attempt succeeded
                eprintln!("[scenario-07] detach after cancel: {e} (may already be detached)");
            }
        }

        cleanup_customer(&client, &customer.id).await;
    }
}
