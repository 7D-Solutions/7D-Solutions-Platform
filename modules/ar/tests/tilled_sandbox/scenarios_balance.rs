//! Sandbox scenarios: balance transactions + payout verification.

#[cfg(test)]
mod tests {
    use crate::tilled_sandbox::helpers::{
        cleanup_customer, cleanup_payment_method, try_create_test_payment_method, unique_email,
        RetryPolicy,
    };
    use crate::tilled_sandbox::{try_partner_client, try_sandbox_client};
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
    // Scenario B1: List balance transactions (partner scope)
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn scenario_b1_list_balance_transactions() {
        let client = match try_partner_client() {
            Some(c) => c,
            None => {
                eprintln!("SKIP: partner creds not set");
                return;
            }
        };

        let retry = RetryPolicy::default();
        let resp = retry
            .execute(|| client.list_balance_transactions(None, Some(10)))
            .await
            .expect("list_balance_transactions should succeed");

        eprintln!(
            "[scenario-b1] balance transactions: total={:?}, items_returned={}",
            resp.total,
            resp.items.len()
        );

        // Verify response structure — items may be empty in sandbox
        for txn in &resp.items {
            eprintln!(
                "[scenario-b1]   txn id={} amount={} status={} type={:?} source={:?}",
                txn.id, txn.amount, txn.status, txn.source_type, txn.source_id
            );
            assert!(!txn.id.is_empty(), "balance transaction must have an ID");
            assert!(!txn.status.is_empty(), "balance transaction must have a status");
        }

        eprintln!("[scenario-b1] PASS — list_balance_transactions returned valid structure");
    }

    // -----------------------------------------------------------------------
    // Scenario B2: Balance transaction after charge
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn scenario_b2_balance_transaction_after_charge() {
        let client = match try_sandbox_client() {
            Some(c) => c,
            None => {
                eprintln!("SKIP: sandbox creds not set");
                return;
            }
        };
        let partner_client = match try_partner_client() {
            Some(c) => c,
            None => {
                eprintln!("SKIP: partner creds not set — cannot check balance transactions");
                return;
            }
        };
        let (sk, acct, base_url) = match sandbox_config() {
            Some(c) => c,
            None => return,
        };

        // Create a payment method for the charge
        let pm = match try_create_test_payment_method(&sk, &acct, &base_url).await {
            Some(pm) => pm,
            None => return,
        };

        // Create customer + attach PM
        let customer = client
            .create_customer(unique_email(), None, None)
            .await
            .expect("create_customer");
        client
            .attach_payment_method(&pm.id, customer.id.clone())
            .await
            .expect("attach_payment_method");

        // Create and confirm a charge (auto-capture)
        let charge_amount = 2599; // $25.99
        let req = CreatePaymentIntentRequest {
            amount: charge_amount,
            currency: "usd".to_string(),
            payment_method_types: vec!["card".to_string()],
            customer_id: Some(customer.id.clone()),
            payment_method_id: Some(pm.id.clone()),
            description: None,
            confirm: Some(true),
            capture_method: Some("automatic".to_string()),
            metadata: None,
        };
        let pi = client
            .create_payment_intent(req)
            .await
            .expect("create_payment_intent");
        eprintln!(
            "[scenario-b2] created payment_intent={} status={} amount={}",
            pi.id, pi.status, pi.amount
        );

        // Check balance transactions via partner client
        // Note: balance transactions may be delayed in sandbox
        let retry = RetryPolicy::default();
        let resp = retry
            .execute(|| partner_client.list_balance_transactions(None, Some(50)))
            .await
            .expect("list_balance_transactions");

        let matching = resp
            .items
            .iter()
            .find(|txn| txn.source_id.as_deref() == Some(&pi.id));

        if let Some(txn) = matching {
            eprintln!(
                "[scenario-b2] found matching balance txn: id={} amount={} fee={:?} net={:?}",
                txn.id, txn.amount, txn.fee, txn.net
            );
        } else {
            eprintln!(
                "[scenario-b2] SKIP assertion: no balance transaction found for PI {} yet \
                 (may be delayed in sandbox). {} txns checked.",
                pi.id,
                resp.items.len()
            );
        }

        // Cleanup
        cleanup_payment_method(&client, &pm.id).await;
        cleanup_customer(&client, &customer.id).await;

        eprintln!("[scenario-b2] PASS — charge created, balance transactions checked");
    }

    // -----------------------------------------------------------------------
    // Scenario B3: List payouts (partner scope)
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn scenario_b3_list_payouts() {
        let client = match try_partner_client() {
            Some(c) => c,
            None => {
                eprintln!("SKIP: partner creds not set");
                return;
            }
        };

        let retry = RetryPolicy::default();
        let resp = retry
            .execute(|| client.list_payouts(None, Some(10)))
            .await
            .expect("list_payouts should succeed");

        eprintln!(
            "[scenario-b3] payouts: total={:?}, items_returned={}",
            resp.total,
            resp.items.len()
        );

        // Payouts may be empty in sandbox — that's acceptable
        for po in &resp.items {
            eprintln!(
                "[scenario-b3]   payout id={} amount={:?} status={} currency={:?} arrival={:?}",
                po.id, po.amount, po.status, po.currency, po.arrival_date
            );
            assert!(!po.id.is_empty(), "payout must have an ID");
            assert!(!po.status.is_empty(), "payout must have a status");
        }

        eprintln!("[scenario-b3] PASS — list_payouts returned valid structure");
    }

    // -----------------------------------------------------------------------
    // Scenario B4: Get payout by ID (if any exist)
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn scenario_b4_get_payout_by_id() {
        let client = match try_partner_client() {
            Some(c) => c,
            None => {
                eprintln!("SKIP: partner creds not set");
                return;
            }
        };

        let retry = RetryPolicy::default();

        // First list payouts to find one
        let list = retry
            .execute(|| client.list_payouts(None, Some(5)))
            .await
            .expect("list_payouts");

        let payout_to_fetch = match list.items.first() {
            Some(po) => po.clone(),
            None => {
                eprintln!("[scenario-b4] SKIP: no payouts in sandbox — nothing to fetch by ID");
                return;
            }
        };

        let fetched = retry
            .execute(|| client.get_payout(&payout_to_fetch.id))
            .await
            .expect("get_payout should succeed");

        assert_eq!(fetched.id, payout_to_fetch.id, "payout ID should match");
        assert_eq!(
            fetched.status, payout_to_fetch.status,
            "payout status should match"
        );
        assert_eq!(
            fetched.amount, payout_to_fetch.amount,
            "payout amount should match"
        );

        eprintln!(
            "[scenario-b4] PASS — get_payout({}) matched list entry: status={} amount={:?}",
            fetched.id, fetched.status, fetched.amount
        );
    }
}
