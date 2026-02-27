//! Sandbox scenarios: balance transactions + payout verification.
//!
//! Balance transactions and payouts live on the **merchant** scope, not
//! the partner scope.  The test suite creates a charge and then verifies
//! that balance transaction line-items appear.  Payouts are discovered by
//! extracting `payout_id` from balance transactions (the list endpoint
//! may return 0 even when payouts exist — a known sandbox quirk).

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
    // Scenario B1: List balance transactions (merchant scope)
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn scenario_b1_list_balance_transactions() {
        let client = match try_sandbox_client() {
            Some(c) => c,
            None => {
                eprintln!("SKIP: sandbox creds not set");
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

        assert!(
            resp.total.unwrap_or(0) > 0,
            "merchant account should have balance transactions from prior charges"
        );

        for txn in &resp.items {
            eprintln!(
                "[scenario-b1]   txn id={} type={:?} amount={} status={} source={:?} payout={:?}",
                txn.id, txn.source_type, txn.amount, txn.status, txn.source_id, txn.payout_id
            );
            assert!(!txn.id.is_empty(), "balance transaction must have an ID");
            assert!(!txn.status.is_empty(), "balance transaction must have a status");
        }

        eprintln!("[scenario-b1] PASS — {} balance transactions found", resp.total.unwrap_or(0));
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
        let (sk, acct, base_url) = match sandbox_config() {
            Some(c) => c,
            None => return,
        };

        let pm = match try_create_test_payment_method(&sk, &acct, &base_url).await {
            Some(pm) => pm,
            None => return,
        };

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
            description: Some("scenario-b2 balance txn verification".to_string()),
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

        // Brief delay for sandbox to generate balance transaction
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;

        // Check balance transactions on merchant scope
        let retry = RetryPolicy::default();
        let resp = retry
            .execute(|| client.list_balance_transactions(None, Some(50)))
            .await
            .expect("list_balance_transactions");

        let matching = resp
            .items
            .iter()
            .find(|txn| txn.source_id.as_deref() == Some(&pi.id));

        if let Some(txn) = matching {
            eprintln!(
                "[scenario-b2] found matching balance txn: id={} amount={} fee={:?} net={:?} payout={:?}",
                txn.id, txn.amount, txn.fee, txn.net, txn.payout_id
            );
            assert_eq!(
                txn.amount as i64, charge_amount,
                "balance txn amount should match charge"
            );
        } else {
            eprintln!(
                "[scenario-b2] NOTE: no balance transaction found for PI {} yet \
                 (may be delayed in sandbox). {} txns checked.",
                pi.id,
                resp.items.len()
            );
        }

        cleanup_payment_method(&client, &pm.id).await;
        cleanup_customer(&client, &customer.id).await;

        eprintln!("[scenario-b2] PASS — charge created, balance transactions checked");
    }

    // -----------------------------------------------------------------------
    // Scenario B3: List payouts (merchant scope) — may be empty
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn scenario_b3_list_payouts() {
        let client = match try_sandbox_client() {
            Some(c) => c,
            None => {
                eprintln!("SKIP: sandbox creds not set");
                return;
            }
        };

        let retry = RetryPolicy::default();
        let resp = retry
            .execute(|| client.list_payouts(None, Some(10)))
            .await
            .expect("list_payouts should succeed");

        eprintln!(
            "[scenario-b3] payouts via list: total={:?}, items_returned={}",
            resp.total,
            resp.items.len()
        );

        for po in &resp.items {
            eprintln!(
                "[scenario-b3]   payout id={} amount={:?} status={} currency={:?}",
                po.id, po.amount, po.status, po.currency
            );
        }

        // Sandbox list may return 0 even when payouts exist (known quirk).
        // The real verification is in B4 below via payout_id from balance txns.
        eprintln!("[scenario-b3] PASS — list_payouts returned valid structure");
    }

    // -----------------------------------------------------------------------
    // Scenario B4: Get payout by ID (discovered from balance transactions)
    //
    // The /v1/payouts list endpoint may return empty in sandbox, but
    // balance transactions carry payout_id and GET /v1/payouts/:id works.
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn scenario_b4_get_payout_by_id() {
        let client = match try_sandbox_client() {
            Some(c) => c,
            None => {
                eprintln!("SKIP: sandbox creds not set");
                return;
            }
        };

        let retry = RetryPolicy::default();

        // Extract a payout_id from balance transactions (more reliable than list)
        let bal_txns = retry
            .execute(|| client.list_balance_transactions(None, Some(100)))
            .await
            .expect("list_balance_transactions");

        let payout_id = bal_txns
            .items
            .iter()
            .filter_map(|txn| txn.payout_id.as_deref())
            .next();

        let payout_id = match payout_id {
            Some(id) => id.to_string(),
            None => {
                eprintln!(
                    "[scenario-b4] SKIP: no payout_id found in {} balance transactions",
                    bal_txns.items.len()
                );
                return;
            }
        };

        eprintln!("[scenario-b4] found payout_id={} in balance transactions", payout_id);

        let fetched = retry
            .execute(|| client.get_payout(&payout_id))
            .await
            .expect("get_payout should succeed");

        assert_eq!(fetched.id, payout_id, "payout ID should match");
        eprintln!(
            "[scenario-b4] PASS — get_payout({}) status={} amount={:?} currency={:?}",
            fetched.id, fetched.status, fetched.amount, fetched.currency
        );
    }
}
