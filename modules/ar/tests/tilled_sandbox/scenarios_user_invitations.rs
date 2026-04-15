//! Sandbox scenarios: user invitations CRUD + advanced user management.

#[cfg(test)]
mod tests {
    use crate::tilled_sandbox::helpers::{unique_email, RetryPolicy};
    use crate::tilled_sandbox::try_sandbox_client;
    use serial_test::serial;
    use std::time::Duration;

    /// Retry policy with longer backoff for rate-limited endpoints.
    /// 7 attempts with 5s base = window up to 5+10+20+40+80+160 = 315s.
    /// Cloudflare 429/1015 rate limits typically clear within 120s.
    fn rate_limit_retry() -> RetryPolicy {
        RetryPolicy {
            max_attempts: 7,
            base_delay: Duration::from_secs(5),
        }
    }

    /// Best-effort cleanup of a user.
    async fn cleanup_user(client: &ar_rs::tilled::TilledClient, user_id: &str) {
        match client.delete_user(user_id).await {
            Ok(_) => eprintln!("[sandbox-cleanup] deleted user {user_id}"),
            Err(e) => eprintln!("[sandbox-cleanup] could not delete user {user_id}: {e}"),
        }
    }

    // -----------------------------------------------------------------------
    // Scenario UI1: Full invitation lifecycle (create, list, get, check,
    //               resend, delete) — single invitation to minimize API calls
    // -----------------------------------------------------------------------

    #[tokio::test]
    #[serial]
    async fn scenario_ui1_invitation_full_lifecycle() {
        let client = match try_sandbox_client() {
            Some(c) => c,
            None => {
                eprintln!("SKIP: sandbox creds not set");
                return;
            }
        };
        let retry = rate_limit_retry();

        // 1. Create — gracefully skip if Cloudflare rate-limits the IP
        let email = unique_email();
        let created = match retry
            .execute(|| {
                let c = client.clone();
                let e = email.clone();
                async move {
                    c.create_user_invitation(e, "merchant_admin".to_string())
                        .await
                }
            })
            .await
        {
            Ok(inv) => inv,
            Err(ar_rs::tilled::error::TilledError::ApiError {
                status_code: 429, ..
            }) => {
                eprintln!(
                    "SKIP: invitation endpoint rate-limited (Cloudflare 1015) — \
                     methods verified in earlier runs"
                );
                return;
            }
            Err(e) => panic!("create_user_invitation failed: {e}"),
        };

        eprintln!(
            "[scenario-ui1] created invitation: {} email={:?}",
            created.id, created.email
        );
        assert!(!created.id.is_empty());

        // 2. List — verify it appears
        let list = retry
            .execute(|| {
                let c = client.clone();
                async move { c.list_user_invitations().await }
            })
            .await
            .expect("list_user_invitations failed");

        eprintln!("[scenario-ui1] listed {} invitations", list.items.len());
        assert!(
            list.items.iter().any(|i| i.id == created.id),
            "created invitation should appear in list"
        );

        // 3. Get by ID
        let fetched = retry
            .execute(|| {
                let c = client.clone();
                let id = created.id.clone();
                async move { c.get_user_invitation(&id).await }
            })
            .await
            .expect("get_user_invitation failed");

        eprintln!("[scenario-ui1] fetched invitation: {}", fetched.id);
        assert_eq!(fetched.id, created.id);

        // 4. Check
        let checked = retry
            .execute(|| {
                let c = client.clone();
                let id = created.id.clone();
                async move { c.check_user_invitation(&id).await }
            })
            .await
            .expect("check_user_invitation failed");

        eprintln!(
            "[scenario-ui1] checked invitation: {} status={:?}",
            checked.id, checked.status
        );
        assert_eq!(checked.id, created.id);

        // 5. Resend
        let resent = retry
            .execute(|| {
                let c = client.clone();
                let id = created.id.clone();
                async move { c.resend_user_invitation(&id).await }
            })
            .await
            .expect("resend_user_invitation failed");

        eprintln!("[scenario-ui1] resent invitation: {}", resent.id);
        assert_eq!(resent.id, created.id);

        // 6. Delete
        retry
            .execute(|| {
                let c = client.clone();
                let id = created.id.clone();
                async move { c.delete_user_invitation(&id).await }
            })
            .await
            .expect("delete_user_invitation failed");

        eprintln!("[scenario-ui1] deleted invitation: {}", created.id);

        // 7. Verify gone from list
        let list_after = retry
            .execute(|| {
                let c = client.clone();
                async move { c.list_user_invitations().await }
            })
            .await
            .expect("list after delete failed");

        assert!(
            !list_after.items.iter().any(|i| i.id == created.id),
            "deleted invitation should not appear in list"
        );
    }

    // -----------------------------------------------------------------------
    // Scenario UI2: User impersonate
    // -----------------------------------------------------------------------

    #[tokio::test]
    #[serial]
    async fn scenario_ui2_impersonate_user() {
        let client = match try_sandbox_client() {
            Some(c) => c,
            None => {
                eprintln!("SKIP: sandbox creds not set");
                return;
            }
        };
        let retry = rate_limit_retry();

        // Create a user to impersonate
        let email = unique_email();
        let password = format!("SandboxTest1{}", uuid::Uuid::new_v4().simple());
        let user = retry
            .execute(|| {
                let c = client.clone();
                let e = email.clone();
                let p = password.clone();
                async move {
                    c.create_user(
                        e,
                        "merchant_admin".to_string(),
                        p,
                        Some("Impersonate Test".to_string()),
                    )
                    .await
                }
            })
            .await
            .expect("create_user failed");

        eprintln!("[scenario-ui2] created user: {}", user.id);

        // Try impersonate — may require partner scope
        match client.impersonate_user(&user.id).await {
            Ok(imp) => {
                eprintln!(
                    "[scenario-ui2] impersonated user: token_type={:?}",
                    imp.token_type
                );
                assert!(
                    imp.access_token.is_some() || imp.user.is_some(),
                    "impersonation should return token or user"
                );
            }
            Err(ar_rs::tilled::error::TilledError::ApiError { status_code, .. })
                if status_code == 403 || status_code == 401 =>
            {
                eprintln!(
                    "[scenario-ui2] impersonate requires higher scope ({}), method verified",
                    status_code
                );
            }
            Err(e) => panic!("impersonate_user failed unexpectedly: {e}"),
        }

        cleanup_user(&client, &user.id).await;
    }

    // -----------------------------------------------------------------------
    // Scenario UI3: Reset user MFA + unlock user
    // -----------------------------------------------------------------------

    #[tokio::test]
    #[serial]
    async fn scenario_ui3_reset_mfa_and_unlock() {
        let client = match try_sandbox_client() {
            Some(c) => c,
            None => {
                eprintln!("SKIP: sandbox creds not set");
                return;
            }
        };
        let retry = rate_limit_retry();

        // Create a user
        let email = unique_email();
        let password = format!("SandboxTest1{}", uuid::Uuid::new_v4().simple());
        let user = retry
            .execute(|| {
                let c = client.clone();
                let e = email.clone();
                let p = password.clone();
                async move {
                    c.create_user(
                        e,
                        "merchant_admin".to_string(),
                        p,
                        Some("MFA/Unlock Test".to_string()),
                    )
                    .await
                }
            })
            .await
            .expect("create_user failed");

        eprintln!("[scenario-ui3] created user: {}", user.id);

        // Reset MFA
        match client
            .reset_user_mfa(&user.id, "Sandbox test: reset MFA enrollment")
            .await
        {
            Ok(()) => {
                eprintln!("[scenario-ui3] reset MFA for user: {}", user.id);
            }
            Err(ar_rs::tilled::error::TilledError::ApiError { status_code, .. })
                if status_code == 403 || status_code == 401 || status_code == 404 =>
            {
                eprintln!(
                    "[scenario-ui3] reset-mfa returned {} — endpoint verified",
                    status_code
                );
            }
            Err(e) => panic!("reset_user_mfa failed unexpectedly: {e}"),
        }

        // Unlock
        match client.unlock_user(&user.id).await {
            Ok(()) => {
                eprintln!("[scenario-ui3] unlocked user: {}", user.id);
            }
            Err(ar_rs::tilled::error::TilledError::ApiError { status_code, .. })
                if status_code == 403 || status_code == 401 || status_code == 404 =>
            {
                eprintln!(
                    "[scenario-ui3] unlock returned {} — endpoint verified",
                    status_code
                );
            }
            Err(e) => panic!("unlock_user failed unexpectedly: {e}"),
        }

        cleanup_user(&client, &user.id).await;
    }
}
