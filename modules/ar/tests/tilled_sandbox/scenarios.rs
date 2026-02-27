//! Sandbox scenarios: customer, payment method, charge capture, and refund.
//!
//! Each scenario tests a complete round-trip: local intent -> provider action
//! -> provider state verification via follow-up GET.

#[cfg(test)]
mod tests {
    use crate::tilled_sandbox::helpers::{
        cleanup_customer, cleanup_payment_method, try_create_test_payment_method,
        unique_email, unique_metadata, RetryPolicy,
    };
    use crate::tilled_sandbox::try_sandbox_client;
    use ar_rs::tilled::payment_intent::CreatePaymentIntentRequest;

    /// Extract sandbox config values needed for raw API helpers.
    fn sandbox_config() -> Option<(String, String, String)> {
        let sk = std::env::var("TILLED_SECRET_KEY").ok()?;
        let acct = std::env::var("TILLED_ACCOUNT_ID").ok()?;
        if sk.is_empty() || acct.is_empty() {
            return None;
        }
        Some((sk, acct, "https://sandbox-api.tilled.com".to_string()))
    }

    // -----------------------------------------------------------------------
    // Scenario 1: Customer sync path
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn scenario_01_customer_create_and_get() {
        let client = match try_sandbox_client() {
            Some(c) => c,
            None => {
                eprintln!("SKIP: sandbox creds not set");
                return;
            }
        };
        let retry = RetryPolicy::default();

        let email = unique_email();
        let meta = unique_metadata();
        let meta_map: std::collections::HashMap<String, String> =
            serde_json::from_value(meta.clone()).unwrap_or_default();

        let customer = retry
            .execute(|| {
                let e = email.clone();
                let m = meta_map.clone();
                let c = client.clone();
                async move {
                    c.create_customer(e, Some("Sandbox Test".to_string()), Some(m))
                        .await
                }
            })
            .await
            .expect("create_customer failed");

        eprintln!(
            "[scenario-01] created customer: {} (email={})",
            customer.id, email
        );
        assert!(!customer.id.is_empty(), "customer ID must be non-empty");
        assert_eq!(customer.email.as_deref(), Some(email.as_str()));

        // Verify via GET — confirms persisted state
        let fetched = retry
            .execute(|| {
                let c = client.clone();
                let id = customer.id.clone();
                async move { c.get_customer(&id).await }
            })
            .await
            .expect("get_customer failed");

        assert_eq!(fetched.id, customer.id);
        assert_eq!(fetched.email.as_deref(), Some(email.as_str()));
        assert_eq!(fetched.first_name.as_deref(), Some("Sandbox Test"));

        cleanup_customer(&client, &customer.id).await;
    }

    // -----------------------------------------------------------------------
    // Scenario 2: Payment method list with customer_id + type
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn scenario_02_payment_method_create_attach_list() {
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
                async move { c.create_customer(e, Some("PM Test".to_string()), None).await }
            })
            .await
            .expect("create_customer failed");

        // Create payment method (raw API — sandbox allows card details)
        let pm = match try_create_test_payment_method(&sk, &acct, &base_url).await {
            Some(pm) => pm,
            None => {
                cleanup_customer(&client, &customer.id).await;
                return;
            }
        };

        eprintln!(
            "[scenario-02] created PM: {} for customer: {}",
            pm.id, customer.id
        );
        assert_eq!(pm.payment_type, "card");
        assert!(pm.card.is_some(), "card details should be present");
        assert_eq!(pm.card.as_ref().unwrap().last4, "1111");

        // Attach PM to customer
        let attached = retry
            .execute(|| {
                let c = client.clone();
                let pm_id = pm.id.clone();
                let cust_id = customer.id.clone();
                async move { c.attach_payment_method(&pm_id, cust_id).await }
            })
            .await
            .expect("attach_payment_method failed");
        assert_eq!(attached.customer_id.as_deref(), Some(customer.id.as_str()));

        // List payment methods with customer_id + type=card
        let list = retry
            .execute(|| {
                let c = client.clone();
                let cust_id = customer.id.clone();
                async move { c.list_payment_methods(&cust_id, "card").await }
            })
            .await
            .expect("list_payment_methods failed");

        assert!(!list.items.is_empty(), "should have at least 1 PM");
        let found = list.items.iter().any(|p| p.id == pm.id);
        assert!(found, "attached PM should appear in list");

        cleanup_payment_method(&client, &pm.id).await;
        cleanup_customer(&client, &customer.id).await;
    }

    // -----------------------------------------------------------------------
    // Scenario 3: Charge capture success + double-capture failure
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn scenario_03_capture_success_and_double_capture_failure() {
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
                    c.create_customer(e, Some("Capture Test".to_string()), None)
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

        // Create payment intent with manual capture
        let pi = retry
            .execute(|| {
                let c = client.clone();
                let req = CreatePaymentIntentRequest {
                    amount: 2500,
                    currency: "usd".to_string(),
                    payment_method_types: vec!["card".to_string()],
                    customer_id: Some(customer.id.clone()),
                    payment_method_id: Some(pm.id.clone()),
                    description: Some("scenario-03 manual capture".to_string()),
                    metadata: None,
                    confirm: Some(true),
                    capture_method: Some("manual".to_string()),
                };
                async move { c.create_payment_intent(req).await }
            })
            .await
            .expect("create_payment_intent failed");

        eprintln!("[scenario-03] PI: {} status={}", pi.id, pi.status);
        assert_eq!(pi.status, "requires_capture");
        assert_eq!(pi.amount, 2500);

        // Capture
        let captured = retry
            .execute(|| {
                let c = client.clone();
                let pi_id = pi.id.clone();
                async move { c.capture_payment_intent(&pi_id, None).await }
            })
            .await
            .expect("capture_payment_intent failed");

        eprintln!("[scenario-03] captured: {} status={}", captured.id, captured.status);
        assert_eq!(captured.status, "succeeded");

        // Double capture → 4xx
        let double = client.capture_payment_intent(&pi.id, None).await;
        match double {
            Err(ar_rs::tilled::error::TilledError::ApiError { status_code, .. }) => {
                assert!(
                    (400..500).contains(&status_code),
                    "double capture should be 4xx, got {status_code}"
                );
                eprintln!("[scenario-03] double capture rejected: {status_code}");
            }
            Err(e) => panic!("expected ApiError for double capture, got: {e}"),
            Ok(pi) => panic!("double capture should fail, got status={}", pi.status),
        }

        cleanup_payment_method(&client, &pm.id).await;
        cleanup_customer(&client, &customer.id).await;
    }

    // -----------------------------------------------------------------------
    // Scenario 4: Refund success + over-refund failure
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn scenario_04_refund_success_and_over_refund_failure() {
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
                    c.create_customer(e, Some("Refund Test".to_string()), None)
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

        // Auto-captured charge: $50.00
        let charge = retry
            .execute(|| {
                let c = client.clone();
                let cust_id = customer.id.clone();
                let pm_id = pm.id.clone();
                async move { c.create_charge(cust_id, pm_id, 5000, None, None, None).await }
            })
            .await
            .expect("create_charge failed");

        eprintln!("[scenario-04] charge: {} status={}", charge.id, charge.status);
        assert_eq!(charge.status, "succeeded");

        // Partial refund: $20 of $50
        let refund = retry
            .execute(|| {
                let c = client.clone();
                let pi_id = charge.id.clone();
                async move {
                    c.create_refund(pi_id, 2000, None, Some("partial test".into()), None)
                        .await
                }
            })
            .await
            .expect("create_refund failed");

        eprintln!(
            "[scenario-04] refund: {} amount={} status={}",
            refund.id, refund.amount, refund.status
        );
        assert!(!refund.id.is_empty());
        assert_eq!(refund.amount, 2000);

        // Verify refund via GET
        let fetched = retry
            .execute(|| {
                let c = client.clone();
                let rid = refund.id.clone();
                async move { c.get_refund(&rid).await }
            })
            .await
            .expect("get_refund failed");
        assert_eq!(fetched.id, refund.id);

        // Over-refund: $40 on remaining $30 → 4xx
        let over = client.create_refund(charge.id.clone(), 4000, None, None, None).await;
        match over {
            Err(ar_rs::tilled::error::TilledError::ApiError { status_code, .. }) => {
                assert!(
                    (400..500).contains(&status_code),
                    "over-refund should be 4xx, got {status_code}"
                );
                eprintln!("[scenario-04] over-refund rejected: {status_code}");
            }
            Err(e) => panic!("expected ApiError for over-refund, got: {e}"),
            Ok(r) => panic!("over-refund should fail, got refund id={}", r.id),
        }

        cleanup_payment_method(&client, &pm.id).await;
        cleanup_customer(&client, &customer.id).await;
    }
}
