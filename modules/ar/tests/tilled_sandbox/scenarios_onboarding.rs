//! Sandbox tests for onboarding, auth links, and merchant applications.
//! All operate on PARTNER scope via `try_partner_client()`.

use super::helpers::RetryPolicy;

/// Helper: require a partner-scope sandbox client or skip.
macro_rules! require_partner {
    () => {
        match $crate::tilled_sandbox::try_partner_client() {
            Some(c) => c,
            None => {
                eprintln!(
                    "SKIP: TILLED_PARTNER_ACCOUNT_ID not set — \
                     partner-scope test skipped."
                );
                return;
            }
        }
    };
}

/// Helper: get the merchant account ID from env.
fn merchant_account_id() -> Option<String> {
    std::env::var("TILLED_ACCOUNT_ID")
        .ok()
        .filter(|s| !s.is_empty())
}

/// Helper: get first user ID on partner account for auth-link tests.
async fn first_partner_user_id(client: &ar_rs::tilled::TilledClient) -> Option<String> {
    match client.list_users().await {
        Ok(list) => list.items.into_iter().next().map(|u| u.id),
        Err(e) => {
            eprintln!("could not list partner users: {e}");
            None
        }
    }
}

// ── Scenario 1: get_onboarding ──────────────────────────────────────

#[tokio::test]
async fn get_onboarding_returns_application() {
    // Onboarding GET uses the merchant's own account scope
    let client = crate::require_sandbox!();
    let retry = RetryPolicy::default();

    let result = retry
        .execute(|| async { client.get_onboarding().await })
        .await;

    match result {
        Ok(app) => {
            // The demo merchant has a legal entity
            assert!(
                app.legal_entity.is_some(),
                "expected legal_entity in onboarding response"
            );
            eprintln!("OK: get_onboarding returned application with legal_entity");
        }
        Err(ar_rs::tilled::error::TilledError::ApiError {
            status_code: 404, ..
        }) => {
            eprintln!(
                "OK: get_onboarding returned 404 — account has no onboarding app \
                 (already fully onboarded). This is acceptable."
            );
        }
        Err(e) => panic!("get_onboarding failed: {e}"),
    }
}

// ── Scenario 2: create_auth_link ────────────────────────────────────

#[tokio::test]
async fn create_auth_link_returns_url() {
    let client = require_partner!();
    let retry = RetryPolicy::default();

    let user_id = match first_partner_user_id(&client).await {
        Some(id) => id,
        None => {
            eprintln!(
                "SKIP: no users found on partner account — \
                 cannot create auth link without user_id"
            );
            return;
        }
    };

    let result = retry
        .execute(|| async {
            client
                .create_auth_link(user_id.clone(), "1d".to_string(), None, None)
                .await
        })
        .await;

    match result {
        Ok(link) => {
            assert!(!link.id.is_empty(), "auth link should have an id");
            assert!(link.url.is_some(), "auth link should have a url");
            let url = link.url.as_deref().unwrap();
            assert!(
                url.contains("tilled.com"),
                "auth link URL should point to tilled.com, got: {url}"
            );
            assert_eq!(link.redeemed, Some(false));
            eprintln!("OK: create_auth_link returned link id={}", link.id);
        }
        Err(e) => panic!("create_auth_link failed: {e}"),
    }
}

// ── Scenario 3: get_merchant_application ────────────────────────────

#[tokio::test]
async fn get_merchant_application_returns_data() {
    let client = require_partner!();
    let retry = RetryPolicy::default();

    let acct_id = match merchant_account_id() {
        Some(id) => id,
        None => {
            eprintln!("SKIP: TILLED_ACCOUNT_ID not set");
            return;
        }
    };

    let result = retry
        .execute(|| async { client.get_merchant_application(&acct_id).await })
        .await;

    match result {
        Ok(app) => {
            // Already-submitted merchant should still return application data
            assert!(
                app.legal_entity.is_some() || app.tos_acceptance.is_some(),
                "expected at least legal_entity or tos_acceptance in response"
            );
            eprintln!("OK: get_merchant_application returned data for {acct_id}");
        }
        Err(ar_rs::tilled::error::TilledError::ApiError {
            status_code: 404,
            ref message,
        }) => {
            eprintln!(
                "OK: get_merchant_application returned 404 for {acct_id}: {message}. \
                 Account may not have an application."
            );
        }
        Err(ar_rs::tilled::error::TilledError::ApiError {
            status_code: 403,
            ref message,
        }) if message.contains("already been submitted") => {
            eprintln!(
                "OK: get_merchant_application returned 403 for {acct_id}: {message}. \
                 Application already submitted — sandbox limitation."
            );
        }
        Err(e) => panic!("get_merchant_application failed: {e}"),
    }
}

// ── Scenario 4: onboarding lifecycle ────────────────────────────────

#[tokio::test]
async fn onboarding_lifecycle_get_app_and_auth_link() {
    let partner_client = require_partner!();
    let merchant_client = crate::require_sandbox!();
    let retry = RetryPolicy::default();

    // Step 1: Get onboarding via merchant scope
    let onboarding = retry
        .execute(|| async { merchant_client.get_onboarding().await })
        .await;

    match &onboarding {
        Ok(app) => {
            eprintln!(
                "lifecycle: got onboarding, tos_acceptance={:?}, validation_errors={:?}",
                app.tos_acceptance,
                app.validation_errors.as_ref().map(|v| v.len())
            );
        }
        Err(ar_rs::tilled::error::TilledError::ApiError {
            status_code: 404, ..
        }) => {
            eprintln!("lifecycle: merchant already fully onboarded (404). Continuing.");
        }
        Err(e) => panic!("lifecycle: get_onboarding failed: {e}"),
    }

    // Step 2: Get merchant application via partner scope
    let acct_id = match merchant_account_id() {
        Some(id) => id,
        None => {
            eprintln!("lifecycle: TILLED_ACCOUNT_ID not set, skipping app+link steps");
            return;
        }
    };

    let app_result = retry
        .execute(|| async { partner_client.get_merchant_application(&acct_id).await })
        .await;

    match &app_result {
        Ok(app) => {
            eprintln!(
                "lifecycle: got merchant application, has_legal_entity={}",
                app.legal_entity.is_some()
            );
        }
        Err(ar_rs::tilled::error::TilledError::ApiError {
            status_code: 404, ..
        }) => {
            eprintln!("lifecycle: no merchant application found (404). Continuing.");
        }
        Err(ar_rs::tilled::error::TilledError::ApiError {
            status_code: 403, ..
        }) => {
            eprintln!("lifecycle: merchant application already submitted (403). Continuing.");
        }
        Err(e) => panic!("lifecycle: get_merchant_application failed: {e}"),
    }

    // Step 3: Create auth link via partner scope
    let user_id = match first_partner_user_id(&partner_client).await {
        Some(id) => id,
        None => {
            eprintln!("lifecycle: no partner users, skipping auth link step");
            return;
        }
    };

    let link = retry
        .execute(|| async {
            partner_client
                .create_auth_link(
                    user_id.clone(),
                    "1d".to_string(),
                    Some("/onboarding".to_string()),
                    None,
                )
                .await
        })
        .await
        .expect("lifecycle: create_auth_link should succeed");

    assert!(!link.id.is_empty());
    assert!(link.url.is_some());
    eprintln!(
        "lifecycle: created auth link id={}, url={}",
        link.id,
        link.url.as_deref().unwrap_or("(none)")
    );
}
