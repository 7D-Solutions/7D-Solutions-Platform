//! Sandbox scenarios: payments lifecycle — decline, auto-capture + refund,
//! manual partial capture, multiple charges.

#[cfg(test)]
mod tests {
    use crate::tilled_sandbox::helpers::{
        cleanup_customer, cleanup_payment_method, try_create_test_payment_method, unique_email,
        RetryPolicy,
    };
    use crate::tilled_sandbox::try_sandbox_client;
    use ar_rs::tilled::payment_intent::CreatePaymentIntentRequest;

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
    // Scenario P1: Decline simulation (insufficient funds)
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn scenario_p1_decline_insufficient_funds() {
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
                    c.create_customer(e, Some("Decline Test".to_string()), None)
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

        // Attach PM to customer (with retry for sandbox 404 race)
        let mut attached = false;
        for attempt in 1..=3 {
            match client
                .attach_payment_method(&pm.id, customer.id.clone())
                .await
            {
                Ok(_) => {
                    attached = true;
                    break;
                }
                Err(ar_rs::tilled::error::TilledError::ApiError {
                    status_code: 404, ..
                }) if attempt < 3 => {
                    eprintln!("[scenario-p1] attach 404 on attempt {attempt}, retrying...");
                    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                }
                Err(e) => panic!("attach failed: {e}"),
            }
        }
        assert!(attached, "attach_payment_method failed after retries");

        // Amount 777701 = insufficient funds decline in Tilled sandbox
        let result = client
            .create_payment_intent(CreatePaymentIntentRequest {
                amount: 777701,
                currency: "usd".to_string(),
                payment_method_types: vec!["card".to_string()],
                customer_id: Some(customer.id.clone()),
                payment_method_id: Some(pm.id.clone()),
                description: Some("scenario-p1 decline simulation".to_string()),
                metadata: None,
                confirm: Some(true),
                capture_method: None,
            })
            .await;

        match result {
            Err(ar_rs::tilled::error::TilledError::ApiError {
                status_code,
                message,
            }) => {
                eprintln!("[scenario-p1] decline correctly rejected: {status_code} — {message}");
                assert!(
                    (400..500).contains(&status_code),
                    "expected 4xx decline, got {status_code}"
                );
            }
            Ok(pi) => {
                // Some sandbox implementations return a PI with failed status
                eprintln!(
                    "[scenario-p1] PI returned with status={} (amount={})",
                    pi.status, pi.amount
                );
                assert!(
                    pi.status == "failed"
                        || pi.status == "requires_payment_method"
                        || pi.status == "canceled",
                    "expected failed/requires_payment_method/canceled, got: {}",
                    pi.status
                );
            }
            Err(e) => panic!("unexpected error type on decline: {e}"),
        }

        cleanup_payment_method(&client, &pm.id).await;
        cleanup_customer(&client, &customer.id).await;
    }

    // -----------------------------------------------------------------------
    // Scenario P2: Auto-capture charge + full refund
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn scenario_p2_auto_capture_and_full_refund() {
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
                    c.create_customer(e, Some("AutoCapture Test".to_string()), None)
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

        // Auto-captured charge: $49.99
        let charge = retry
            .execute(|| {
                let c = client.clone();
                let cust_id = customer.id.clone();
                let pm_id = pm.id.clone();
                async move {
                    c.create_charge(cust_id, pm_id, 4999, None, None, None)
                        .await
                }
            })
            .await
            .expect("create_charge failed");

        eprintln!(
            "[scenario-p2] charge: {} status={} amount={:?}",
            charge.id, charge.status, charge.amount
        );
        assert_eq!(charge.status, "succeeded");
        assert_eq!(charge.amount, Some(4999));

        // Full refund with reason
        let refund = match retry
            .execute(|| {
                let c = client.clone();
                let pi_id = charge.id.clone();
                async move {
                    c.create_refund(
                        pi_id,
                        4999,
                        None,
                        Some("requested_by_customer".into()),
                        None,
                    )
                    .await
                }
            })
            .await
        {
            Ok(refund) => refund,
            Err(ar_rs::tilled::error::TilledError::ApiError {
                status_code: 400,
                message,
            }) if message.contains("batched yet") => {
                eprintln!("[scenario-p2] SKIP refund: charge not batched yet");
                cleanup_payment_method(&client, &pm.id).await;
                cleanup_customer(&client, &customer.id).await;
                return;
            }
            Err(e) => panic!("create_refund failed: {e}"),
        };

        eprintln!(
            "[scenario-p2] refund: {} amount={} status={}",
            refund.id, refund.amount, refund.status
        );
        assert_eq!(refund.amount, 4999, "full refund amount must match charge");
        assert!(!refund.id.is_empty());

        cleanup_payment_method(&client, &pm.id).await;
        cleanup_customer(&client, &customer.id).await;
    }

    // -----------------------------------------------------------------------
    // Scenario P3: Manual capture with partial amount
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn scenario_p3_manual_capture_partial_amount() {
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
                    c.create_customer(e, Some("Partial Capture Test".to_string()), None)
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

        // Attach with retry for sandbox 404 race
        let mut attached = false;
        for attempt in 1..=3 {
            match client
                .attach_payment_method(&pm.id, customer.id.clone())
                .await
            {
                Ok(_) => {
                    attached = true;
                    break;
                }
                Err(ar_rs::tilled::error::TilledError::ApiError {
                    status_code: 404, ..
                }) if attempt < 3 => {
                    eprintln!("[scenario-p3] attach 404 on attempt {attempt}, retrying...");
                    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                }
                Err(e) => panic!("attach failed: {e}"),
            }
        }
        assert!(attached, "attach_payment_method failed after retries");

        // Create PI with manual capture, $100.00 authorized
        let pi = retry
            .execute(|| {
                let c = client.clone();
                let req = CreatePaymentIntentRequest {
                    amount: 10000,
                    currency: "usd".to_string(),
                    payment_method_types: vec!["card".to_string()],
                    customer_id: Some(customer.id.clone()),
                    payment_method_id: Some(pm.id.clone()),
                    description: Some("scenario-p3 partial capture".to_string()),
                    metadata: None,
                    confirm: Some(true),
                    capture_method: Some("manual".to_string()),
                };
                async move { c.create_payment_intent(req).await }
            })
            .await
            .expect("create_payment_intent failed");

        eprintln!(
            "[scenario-p3] PI: {} status={} amount={}",
            pi.id, pi.status, pi.amount
        );
        assert_eq!(pi.status, "requires_capture");
        assert_eq!(pi.amount, 10000);

        // Capture partial amount: $60.00 of $100.00
        let captured = retry
            .execute(|| {
                let c = client.clone();
                let pi_id = pi.id.clone();
                async move { c.capture_payment_intent(&pi_id, Some(6000)).await }
            })
            .await
            .expect("capture_payment_intent failed");

        eprintln!(
            "[scenario-p3] captured: {} status={}",
            captured.id, captured.status
        );
        assert_eq!(captured.status, "succeeded");

        cleanup_payment_method(&client, &pm.id).await;
        cleanup_customer(&client, &customer.id).await;
    }

    // -----------------------------------------------------------------------
    // Scenario P4: Multiple charges on same customer
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn scenario_p4_multiple_charges_same_customer() {
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
                    c.create_customer(e, Some("Multi Charge Test".to_string()), None)
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

        // Create 3 charges with different amounts
        let amounts = [1500_i64, 2500, 3500]; // $15, $25, $35
        let mut charge_ids = Vec::new();

        for (i, &amount) in amounts.iter().enumerate() {
            let charge = retry
                .execute(|| {
                    let c = client.clone();
                    let cust_id = customer.id.clone();
                    let pm_id = pm.id.clone();
                    let desc = format!("scenario-p4 charge {}", i + 1);
                    async move {
                        c.create_charge(cust_id, pm_id, amount, None, Some(desc), None)
                            .await
                    }
                })
                .await
                .unwrap_or_else(|e| panic!("create_charge {} failed: {e}", i + 1));

            eprintln!(
                "[scenario-p4] charge {}: {} status={} amount={:?}",
                i + 1,
                charge.id,
                charge.status,
                charge.amount
            );
            assert_eq!(charge.status, "succeeded");
            assert_eq!(charge.amount, Some(amount));
            charge_ids.push(charge.id);
        }

        // Verify all 3 have distinct IDs
        assert_eq!(charge_ids.len(), 3);
        let unique: std::collections::HashSet<&String> = charge_ids.iter().collect();
        assert_eq!(
            unique.len(),
            3,
            "all 3 charges must have distinct IDs: {:?}",
            charge_ids
        );

        cleanup_payment_method(&client, &pm.id).await;
        cleanup_customer(&client, &customer.id).await;
    }
}
