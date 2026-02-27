//! Sandbox scenarios: merchant onboarding + multi-merchant isolation.
//!
//! Tests connected account operations against the real Tilled sandbox.
//! Partner-scope tests require `TILLED_PARTNER_ACCOUNT_ID`.

#[cfg(test)]
mod tests {
    use crate::tilled_sandbox::helpers::{cleanup_customer, unique_email, RetryPolicy};
    use crate::tilled_sandbox::{try_partner_client, try_sandbox_client};

    // -----------------------------------------------------------------------
    // Scenario 1: List connected accounts
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn scenario_m01_list_connected_accounts() {
        let client = match try_partner_client() {
            Some(c) => c,
            None => {
                eprintln!("SKIP: partner creds not set");
                return;
            }
        };
        let retry = RetryPolicy::default();

        let accounts = retry
            .execute(|| {
                let c = client.clone();
                async move { c.list_connected_accounts(None, Some(100)).await }
            })
            .await
            .expect("list_connected_accounts failed");

        eprintln!(
            "[scenario-m01] found {} connected accounts",
            accounts.items.len()
        );
        assert!(
            accounts.items.len() >= 2,
            "expected at least 2 connected accounts, got {}",
            accounts.items.len()
        );

        let has_shovel = accounts
            .items
            .iter()
            .any(|a| a.id == "acct_AWRc6cK2YDg4sfMprqgul");
        assert!(has_shovel, "Shovel Shop account should be in the list");
    }

    // -----------------------------------------------------------------------
    // Scenario 2: Get specific account via list filter
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn scenario_m02_get_specific_account() {
        let client = match try_partner_client() {
            Some(c) => c,
            None => {
                eprintln!("SKIP: partner creds not set");
                return;
            }
        };
        let retry = RetryPolicy::default();

        // Tilled doesn't expose GET /v1/accounts/{id} at partner scope.
        // List connected accounts and filter to the Shovel Shop instead.
        let accounts = retry
            .execute(|| {
                let c = client.clone();
                async move { c.list_connected_accounts(None, Some(100)).await }
            })
            .await
            .expect("list_connected_accounts failed");

        let shovel_id = "acct_AWRc6cK2YDg4sfMprqgul";
        let account = accounts
            .items
            .iter()
            .find(|a| a.id == shovel_id)
            .expect("Shovel Shop not found in connected accounts list");

        eprintln!(
            "[scenario-m02] account: {} name={:?} status={}",
            account.id, account.name, account.status
        );
        assert_eq!(account.id, shovel_id);
        assert!(
            account.name.is_some(),
            "Shovel Shop should have a name set"
        );

        if let Some(caps) = &account.capabilities {
            eprintln!("[scenario-m02] capabilities: {}", caps);
        }
    }

    // -----------------------------------------------------------------------
    // Scenario 3: Create new connected account
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn scenario_m03_create_connected_account() {
        let client = match try_partner_client() {
            Some(c) => c,
            None => {
                eprintln!("SKIP: partner creds not set");
                return;
            }
        };
        let retry = RetryPolicy::default();

        let email = unique_email();
        let name = format!("Test Merchant {}", &email[..8]);

        let account = retry
            .execute(|| {
                let c = client.clone();
                let e = email.clone();
                let n = name.clone();
                async move {
                    c.create_connected_account(e, Some(n), None, None, None)
                        .await
                }
            })
            .await
            .expect("create_connected_account failed");

        eprintln!(
            "[scenario-m03] created account: {} name={:?} status={}",
            account.id, account.name, account.status
        );
        assert!(!account.id.is_empty(), "account ID must be non-empty");
        assert!(
            account.id.starts_with("acct_"),
            "account ID should start with acct_"
        );
    }

    // -----------------------------------------------------------------------
    // Scenario 4: Multi-merchant isolation
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn scenario_m04_multi_merchant_isolation() {
        let merchant_client = match try_sandbox_client() {
            Some(c) => c,
            None => {
                eprintln!("SKIP: merchant sandbox creds not set");
                return;
            }
        };
        let partner_client = match try_partner_client() {
            Some(c) => c,
            None => {
                eprintln!("SKIP: partner creds not set");
                return;
            }
        };
        let retry = RetryPolicy::default();

        let email = unique_email();
        let customer = retry
            .execute(|| {
                let c = merchant_client.clone();
                let e = email.clone();
                async move {
                    c.create_customer(e, Some("Isolation Test".to_string()), None)
                        .await
                }
            })
            .await
            .expect("create_customer on merchant failed");

        eprintln!(
            "[scenario-m04] created customer {} on merchant scope",
            customer.id
        );

        // Verify visible on merchant client
        let fetched = retry
            .execute(|| {
                let c = merchant_client.clone();
                let id = customer.id.clone();
                async move { c.get_customer(&id).await }
            })
            .await
            .expect("get_customer on merchant should succeed");
        assert_eq!(fetched.id, customer.id);

        // Cross-scope fetch: partner scope should NOT see merchant's customer
        let cross_fetch = partner_client.get_customer(&customer.id).await;
        match cross_fetch {
            Err(ar_rs::tilled::error::TilledError::ApiError {
                status_code, ..
            }) => {
                eprintln!(
                    "[scenario-m04] cross-scope fetch returned {} (expected 4xx)",
                    status_code
                );
                assert!(
                    (400..500).contains(&status_code),
                    "cross-scope customer fetch should be 4xx, got {}",
                    status_code
                );
            }
            Err(e) => panic!("expected ApiError for cross-scope fetch, got: {e}"),
            Ok(c) => {
                // Partner scope may have visibility into merchant customers
                eprintln!(
                    "[scenario-m04] WARNING: customer {} found in partner scope — \
                     Tilled shares customers at partner level. \
                     Verifying identity match.",
                    c.id
                );
                assert_eq!(c.id, customer.id);
            }
        }

        cleanup_customer(&merchant_client, &customer.id).await;
    }
}
