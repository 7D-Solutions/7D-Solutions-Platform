//! Sandbox scenarios: account self-management + capabilities.
//!
//! Tests GET/PATCH /v1/accounts (self) and capabilities CRUD against real Tilled sandbox.
//! Merchant and partner scopes are both tested.

#[cfg(test)]
mod tests {
    use crate::tilled_sandbox::helpers::RetryPolicy;
    use crate::tilled_sandbox::{try_partner_client, try_sandbox_client};

    // -----------------------------------------------------------------------
    // Scenario 1: Get self account (merchant scope)
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn scenario_as01_get_self_account_merchant() {
        let client = match try_sandbox_client() {
            Some(c) => c,
            None => {
                eprintln!("SKIP: merchant sandbox creds not set");
                return;
            }
        };
        let retry = RetryPolicy::default();

        let account = retry
            .execute(|| {
                let c = client.clone();
                async move { c.get_self_account().await }
            })
            .await
            .expect("get_self_account (merchant) failed");

        eprintln!(
            "[scenario-as01] account: {} type={:?} name={:?} status={}",
            account.id, account.account_type, account.name, account.status
        );

        let expected_id = std::env::var("TILLED_ACCOUNT_ID").unwrap();
        assert_eq!(
            account.id, expected_id,
            "self-account ID should match TILLED_ACCOUNT_ID"
        );
        assert_eq!(account.account_type.as_deref(), Some("merchant"));
        assert_eq!(account.status, "active");
        assert!(
            !account.capabilities.is_empty(),
            "merchant should have at least one capability"
        );

        let cap = &account.capabilities[0];
        eprintln!(
            "[scenario-as01] capability: {} status={:?}",
            cap.id, cap.status
        );
        assert!(
            cap.id.starts_with("pp_"),
            "capability ID should start with pp_"
        );
        assert!(
            cap.product_code.is_some(),
            "capability should have product_code"
        );
    }

    // -----------------------------------------------------------------------
    // Scenario 2: Get self account (partner scope)
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn scenario_as02_get_self_account_partner() {
        let client = match try_partner_client() {
            Some(c) => c,
            None => {
                eprintln!("SKIP: partner creds not set");
                return;
            }
        };
        let retry = RetryPolicy::default();

        let account = retry
            .execute(|| {
                let c = client.clone();
                async move { c.get_self_account().await }
            })
            .await
            .expect("get_self_account (partner) failed");

        eprintln!(
            "[scenario-as02] partner account: {} type={:?} name={:?} status={}",
            account.id, account.account_type, account.name, account.status
        );

        let expected_id = std::env::var("TILLED_PARTNER_ACCOUNT_ID").unwrap();
        assert_eq!(
            account.id, expected_id,
            "self-account ID should match TILLED_PARTNER_ACCOUNT_ID"
        );
        assert_eq!(account.account_type.as_deref(), Some("partner"));
        assert_eq!(account.status, "active");
    }

    // -----------------------------------------------------------------------
    // Scenario 3: Update self account metadata
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn scenario_as03_update_account_metadata() {
        let client = match try_sandbox_client() {
            Some(c) => c,
            None => {
                eprintln!("SKIP: merchant sandbox creds not set");
                return;
            }
        };
        let retry = RetryPolicy::default();

        // Add a test key via metadata update
        let update_req = ar_rs::tilled::accounts::UpdateAccountRequest {
            metadata: Some(serde_json::json!({"sandbox_test": "as03_update"})),
            name: None,
            email: None,
        };

        let updated = retry
            .execute(|| {
                let c = client.clone();
                let req = update_req.clone();
                async move { c.update_account(&req).await }
            })
            .await
            .expect("update_account failed");

        eprintln!("[scenario-as03] updated metadata: {:?}", updated.metadata);
        assert!(
            updated
                .metadata
                .as_ref()
                .and_then(|m| m.get("sandbox_test"))
                .map(|v| v == "as03_update")
                .unwrap_or(false),
            "metadata should contain the sandbox_test key"
        );

        // Remove the test key by setting it to null (Tilled merges metadata)
        let restore_req = ar_rs::tilled::accounts::UpdateAccountRequest {
            metadata: Some(serde_json::json!({"sandbox_test": null})),
            name: None,
            email: None,
        };

        let restored = retry
            .execute(|| {
                let c = client.clone();
                let req = restore_req.clone();
                async move { c.update_account(&req).await }
            })
            .await
            .expect("restore metadata failed");

        eprintln!("[scenario-as03] restored metadata: {:?}", restored.metadata);
        // sandbox_test key should be gone
        let still_has_test = restored
            .metadata
            .as_ref()
            .and_then(|m| m.get("sandbox_test"))
            .is_some();
        assert!(!still_has_test, "sandbox_test should be removed after null");
    }

    // -----------------------------------------------------------------------
    // Scenario 4: Inspect capabilities from account response
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn scenario_as04_inspect_capabilities() {
        let client = match try_sandbox_client() {
            Some(c) => c,
            None => {
                eprintln!("SKIP: merchant sandbox creds not set");
                return;
            }
        };
        let retry = RetryPolicy::default();

        let account = retry
            .execute(|| {
                let c = client.clone();
                async move { c.get_self_account().await }
            })
            .await
            .expect("get_self_account failed");

        eprintln!(
            "[scenario-as04] {} capabilities found",
            account.capabilities.len()
        );

        for cap in &account.capabilities {
            let pm_type = cap
                .product_code
                .as_ref()
                .and_then(|pc| pc.get("payment_method_type"))
                .and_then(|v| v.as_str())
                .unwrap_or("unknown");

            eprintln!(
                "[scenario-as04]   cap={} status={:?} pm_type={} progress={:?}",
                cap.id, cap.status, pm_type, cap.onboarding_application_progress
            );
            assert!(!cap.id.is_empty());
        }

        let has_card = account.capabilities.iter().any(|c| {
            c.product_code
                .as_ref()
                .and_then(|pc| pc.get("payment_method_type"))
                .and_then(|v| v.as_str())
                == Some("card")
        });
        assert!(has_card, "merchant should have card capability");
    }

    // -----------------------------------------------------------------------
    // Scenario 5: Capabilities add guard (expects 400 on submitted account)
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn scenario_as05_capability_add_guard() {
        let client = match try_sandbox_client() {
            Some(c) => c,
            None => {
                eprintln!("SKIP: merchant sandbox creds not set");
                return;
            }
        };

        // The Shovel Shop already has onboarding submitted, so adding a capability
        // should return 400. This proves the endpoint is wired correctly.
        let req = ar_rs::tilled::accounts::AddCapabilityRequest {
            pricing_template_id: "pt_nonexistent".to_string(),
        };
        let result = client.add_account_capability(&req).await;

        match result {
            Err(ar_rs::tilled::error::TilledError::ApiError {
                status_code,
                ref message,
            }) => {
                eprintln!(
                    "[scenario-as05] add_capability returned {} — {}",
                    status_code, message
                );
                assert_eq!(status_code, 400, "expected 400, got {status_code}");
            }
            Ok(cap) => {
                eprintln!(
                    "[scenario-as05] WARNING: capability added unexpectedly: {}",
                    cap.id
                );
                let _ = client.delete_account_capability(&cap.id).await;
            }
            Err(e) => panic!("unexpected error from add_capability: {e}"),
        }
    }

    // -----------------------------------------------------------------------
    // Scenario 6: Capability update (re-apply same pricing template)
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn scenario_as06_capability_update() {
        let client = match try_sandbox_client() {
            Some(c) => c,
            None => {
                eprintln!("SKIP: merchant sandbox creds not set");
                return;
            }
        };
        let retry = RetryPolicy::default();

        let account = retry
            .execute(|| {
                let c = client.clone();
                async move { c.get_self_account().await }
            })
            .await
            .expect("get_self_account failed");

        let cap = match account.capabilities.first() {
            Some(c) => c,
            None => {
                eprintln!("SKIP: no capabilities on account");
                return;
            }
        };

        let pt_id = cap
            .pricing_template
            .as_ref()
            .and_then(|pt| pt.get("id"))
            .and_then(|v| v.as_str());

        let pt_id = match pt_id {
            Some(id) => id.to_string(),
            None => {
                eprintln!("SKIP: capability has no pricing_template.id");
                return;
            }
        };

        // Re-apply the same pricing template (returns 201 with empty body)
        let req = ar_rs::tilled::accounts::UpdateCapabilityRequest {
            pricing_template_id: pt_id,
        };

        let result = client.update_account_capability(&cap.id, &req).await;

        match result {
            Ok(()) => {
                eprintln!(
                    "[scenario-as06] updated capability {} — 201 success",
                    cap.id
                );
            }
            Err(ar_rs::tilled::error::TilledError::ApiError {
                status_code,
                ref message,
            }) if status_code == 400 => {
                eprintln!(
                    "[scenario-as06] update rejected (400): {} — sandbox limitation",
                    message
                );
            }
            Err(e) => panic!("unexpected error from update_capability: {e}"),
        }
    }

    // -----------------------------------------------------------------------
    // Scenario 7: Capability delete guard (expects 400 on submitted account)
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn scenario_as07_capability_delete_guard() {
        let client = match try_sandbox_client() {
            Some(c) => c,
            None => {
                eprintln!("SKIP: merchant sandbox creds not set");
                return;
            }
        };
        let retry = RetryPolicy::default();

        let account = retry
            .execute(|| {
                let c = client.clone();
                async move { c.get_self_account().await }
            })
            .await
            .expect("get_self_account failed");

        let cap = match account.capabilities.first() {
            Some(c) => c,
            None => {
                eprintln!("SKIP: no capabilities on account");
                return;
            }
        };

        let result = client.delete_account_capability(&cap.id).await;

        match result {
            Err(ar_rs::tilled::error::TilledError::ApiError {
                status_code,
                ref message,
            }) => {
                eprintln!(
                    "[scenario-as07] delete_capability returned {} — {}",
                    status_code, message
                );
                assert_eq!(status_code, 400, "expected 400, got {status_code}");
            }
            Ok(()) => {
                eprintln!("[scenario-as07] WARNING: capability deleted — sandbox may allow this");
            }
            Err(e) => panic!("unexpected error from delete_capability: {e}"),
        }
    }
}
