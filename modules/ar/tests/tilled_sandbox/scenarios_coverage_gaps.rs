//! Sandbox scenarios: coverage gap tests for 6 untested client methods.
//!
//! Gap 1: update_customer
//! Gap 2: confirm_payment_intent (standalone, not create+confirm=true)
//! Gap 3: get_payment_method
//! Gap 4: list_refunds (queries existing sandbox refunds)
//! Gap 5: get_dispute
//! Gap 6: list_connected_accounts (replaces nonexistent get_account endpoint)

#[cfg(test)]
mod tests {
    use crate::tilled_sandbox::helpers::{
        cleanup_customer, cleanup_payment_method, try_create_test_payment_method, unique_email,
        RetryPolicy,
    };
    use crate::tilled_sandbox::{try_partner_client, try_sandbox_client};
    use ar_rs::tilled::customer::UpdateCustomerRequest;
    use ar_rs::tilled::payment_intent::CreatePaymentIntentRequest;
    use ar_rs::tilled::types::Dispute;
    use ar_rs::tilled::TilledClient;
    use std::time::{Duration, Instant};

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
    // Gap 1: update_customer
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn gap_01_update_customer() {
        let client = match try_sandbox_client() {
            Some(c) => c,
            None => {
                eprintln!("SKIP: sandbox creds not set");
                return;
            }
        };
        let retry = RetryPolicy::default();

        let email = unique_email();
        let customer = retry
            .execute(|| {
                let c = client.clone();
                let e = email.clone();
                async move {
                    c.create_customer(e, Some("Original Name".to_string()), None)
                        .await
                }
            })
            .await
            .expect("create_customer failed");

        eprintln!("[gap-01] created customer: {}", customer.id);
        assert_eq!(customer.first_name.as_deref(), Some("Original Name"));

        // Update name and last_name
        let updated = retry
            .execute(|| {
                let c = client.clone();
                let id = customer.id.clone();
                let u = UpdateCustomerRequest {
                    email: None,
                    first_name: Some("Updated First".to_string()),
                    last_name: Some("Updated Last".to_string()),
                    metadata: None,
                };
                async move { c.update_customer(&id, u).await }
            })
            .await
            .expect("update_customer failed");

        eprintln!(
            "[gap-01] updated: first={:?} last={:?}",
            updated.first_name, updated.last_name
        );
        assert_eq!(updated.first_name.as_deref(), Some("Updated First"));
        assert_eq!(updated.last_name.as_deref(), Some("Updated Last"));

        // Verify via GET
        let fetched = retry
            .execute(|| {
                let c = client.clone();
                let id = customer.id.clone();
                async move { c.get_customer(&id).await }
            })
            .await
            .expect("get_customer failed");

        assert_eq!(fetched.first_name.as_deref(), Some("Updated First"));
        assert_eq!(fetched.last_name.as_deref(), Some("Updated Last"));

        cleanup_customer(&client, &customer.id).await;
    }

    // -----------------------------------------------------------------------
    // Gap 2: confirm_payment_intent (standalone)
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn gap_02_confirm_payment_intent_standalone() {
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
                    c.create_customer(e, Some("Confirm Test".to_string()), None)
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

        // Create PI with confirm=false
        let pi = retry
            .execute(|| {
                let c = client.clone();
                let req = CreatePaymentIntentRequest {
                    amount: 1500,
                    currency: "usd".to_string(),
                    payment_method_types: vec!["card".to_string()],
                    customer_id: Some(customer.id.clone()),
                    payment_method_id: Some(pm.id.clone()),
                    description: Some("gap-02 confirm standalone".to_string()),
                    metadata: None,
                    confirm: Some(false),
                    capture_method: None,
                };
                async move { c.create_payment_intent(req).await }
            })
            .await
            .expect("create_payment_intent failed");

        eprintln!("[gap-02] PI created: {} status={}", pi.id, pi.status);
        assert!(
            pi.status == "requires_confirmation" || pi.status == "requires_payment_method",
            "expected requires_confirmation or requires_payment_method, got {}",
            pi.status
        );

        // Confirm standalone
        let confirmed = retry
            .execute(|| {
                let c = client.clone();
                let pi_id = pi.id.clone();
                let pm_id = pm.id.clone();
                async move { c.confirm_payment_intent(&pi_id, Some(pm_id)).await }
            })
            .await
            .expect("confirm_payment_intent failed");

        eprintln!(
            "[gap-02] confirmed: {} status={}",
            confirmed.id, confirmed.status
        );
        assert!(
            confirmed.status == "succeeded" || confirmed.status == "requires_capture",
            "expected succeeded or requires_capture after confirm, got {}",
            confirmed.status
        );

        cleanup_payment_method(&client, &pm.id).await;
        cleanup_customer(&client, &customer.id).await;
    }

    // -----------------------------------------------------------------------
    // Gap 3: get_payment_method
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn gap_03_get_payment_method() {
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
                    c.create_customer(e, Some("Get PM Test".to_string()), None)
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

        // Get payment method by ID
        let fetched = retry
            .execute(|| {
                let c = client.clone();
                let pm_id = pm.id.clone();
                async move { c.get_payment_method(&pm_id).await }
            })
            .await
            .expect("get_payment_method failed");

        eprintln!(
            "[gap-03] fetched PM: {} type={} card={:?}",
            fetched.id, fetched.payment_type, fetched.card
        );
        assert_eq!(fetched.id, pm.id);
        assert_eq!(fetched.payment_type, "card");

        let card = fetched.card.expect("card details should be present");
        assert_eq!(card.last4, "1111");
        assert_eq!(card.exp_month, 12);
        assert_eq!(card.exp_year, 2030);
        assert!(!card.brand.is_empty(), "brand should be non-empty");

        cleanup_payment_method(&client, &pm.id).await;
        cleanup_customer(&client, &customer.id).await;
    }

    // -----------------------------------------------------------------------
    // Gap 4: list_refunds — query existing refunds in sandbox (no charge needed)
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn gap_04_list_refunds() {
        let client = match try_sandbox_client() {
            Some(c) => c,
            None => {
                eprintln!("SKIP: sandbox creds not set");
                return;
            }
        };
        let retry = RetryPolicy::default();

        // List existing refunds — sandbox already has refunds from previous test runs.
        // This avoids the "charge not batched yet" skip that plagued the old approach.
        let list = retry
            .execute(|| {
                let c = client.clone();
                async move { c.list_refunds(None).await }
            })
            .await
            .expect("list_refunds failed");

        eprintln!("[gap-04] list_refunds returned {} items", list.items.len());
        assert!(
            !list.items.is_empty(),
            "sandbox should have existing refunds from previous test runs"
        );

        // Validate structure of each refund
        for r in &list.items {
            assert!(!r.id.is_empty(), "refund id must be non-empty");
            assert!(r.amount > 0, "refund amount must be positive, got {}", r.amount);
            assert!(!r.status.is_empty(), "refund status must be non-empty");
        }

        eprintln!(
            "[gap-04] PASS: {} refunds validated (first: id={} amount={} status={})",
            list.items.len(),
            list.items[0].id,
            list.items[0].amount,
            list.items[0].status,
        );
    }

    // -----------------------------------------------------------------------
    // Gap 5: get_dispute
    // -----------------------------------------------------------------------

    const DISPUTE_TRIGGER_AMOUNT: i64 = 777_799;

    async fn wait_for_dispute(
        client: &TilledClient,
        charge_id: &str,
        max_wait_secs: u64,
    ) -> Option<Dispute> {
        let deadline = Instant::now() + Duration::from_secs(max_wait_secs);
        loop {
            match client.list_disputes(None).await {
                Ok(list) => {
                    if let Some(dispute) = list
                        .items
                        .into_iter()
                        .find(|d| d.charge_id.as_deref() == Some(charge_id))
                    {
                        return Some(dispute);
                    }
                }
                Err(e) => eprintln!("[gap-05] list_disputes poll error: {e}"),
            }

            if Instant::now() >= deadline {
                return None;
            }
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
    }

    #[tokio::test]
    async fn gap_05_get_dispute() {
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
                    c.create_customer(e, Some("Dispute Get Test".to_string()), None)
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

        let charge = retry
            .execute(|| {
                let c = client.clone();
                let cust_id = customer.id.clone();
                let pm_id = pm.id.clone();
                async move {
                    c.create_charge(
                        cust_id, pm_id, DISPUTE_TRIGGER_AMOUNT, None, None, None,
                    )
                    .await
                }
            })
            .await
            .expect("create_charge failed");

        assert_eq!(charge.status, "succeeded");

        let ch_id = match &charge.charge_id {
            Some(id) => id.clone(),
            None => {
                eprintln!(
                    "SKIP: no charge_id in response for PI {} — cannot match disputes",
                    charge.id
                );
                cleanup_payment_method(&client, &pm.id).await;
                cleanup_customer(&client, &customer.id).await;
                return;
            }
        };
        eprintln!("[gap-05] PI={} charge_id={}", charge.id, ch_id);
        tokio::time::sleep(Duration::from_secs(2)).await;

        let dispute = match wait_for_dispute(&client, &ch_id, 20).await {
            Some(d) => d,
            None => {
                eprintln!("SKIP: dispute did not appear within timeout for charge {}", ch_id);
                cleanup_payment_method(&client, &pm.id).await;
                cleanup_customer(&client, &customer.id).await;
                return;
            }
        };

        eprintln!(
            "[gap-05] dispute found via list: {} status={}",
            dispute.id, dispute.status
        );

        // Now test get_dispute by ID
        let fetched = retry
            .execute(|| {
                let c = client.clone();
                let did = dispute.id.clone();
                async move { c.get_dispute(&did).await }
            })
            .await
            .expect("get_dispute failed");

        eprintln!(
            "[gap-05] get_dispute: {} status={} amount={:?} reason={:?}",
            fetched.id, fetched.status, fetched.amount, fetched.reason_description
        );
        assert_eq!(fetched.id, dispute.id);
        assert!(!fetched.status.is_empty());
        assert_eq!(fetched.charge_id.as_deref(), Some(ch_id.as_str()));

        cleanup_payment_method(&client, &pm.id).await;
        cleanup_customer(&client, &customer.id).await;
    }

    // -----------------------------------------------------------------------
    // Gap 6: list_connected_accounts (replaces get_account — that endpoint
    // returns 404 at all scopes in Tilled's API)
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn gap_06_list_connected_accounts() {
        let client = match try_partner_client() {
            Some(c) => c,
            None => {
                eprintln!("SKIP: TILLED_PARTNER_ACCOUNT_ID not set");
                return;
            }
        };
        let retry = RetryPolicy::default();

        let list = retry
            .execute(|| {
                let c = client.clone();
                async move { c.list_connected_accounts(None, Some(10)).await }
            })
            .await
            .expect("list_connected_accounts failed");

        eprintln!(
            "[gap-06] list_connected_accounts returned {} items",
            list.items.len()
        );
        assert!(
            !list.items.is_empty(),
            "partner account should have connected merchants"
        );

        // Validate structure of each account
        for acct in &list.items {
            assert!(!acct.id.is_empty(), "account id must be non-empty");
            assert!(
                acct.id.starts_with("acct_"),
                "account id should start with acct_, got {}",
                acct.id
            );
            assert!(!acct.status.is_empty(), "account status must be non-empty");
        }

        eprintln!(
            "[gap-06] PASS: {} connected accounts validated (first: id={} name={:?} status={})",
            list.items.len(),
            list.items[0].id,
            list.items[0].name,
            list.items[0].status,
        );
    }
}
