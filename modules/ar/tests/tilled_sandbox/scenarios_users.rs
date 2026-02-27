//! Sandbox scenarios: users per merchant (create, list, get, delete).

#[cfg(test)]
mod tests {
    use crate::tilled_sandbox::helpers::{unique_email, RetryPolicy};
    use crate::tilled_sandbox::try_sandbox_client;

    #[tokio::test]
    async fn scenario_u1_users_create_list_get_delete() {
        let client = match try_sandbox_client() {
            Some(c) => c,
            None => {
                eprintln!("SKIP: sandbox creds not set");
                return;
            }
        };
        let retry = RetryPolicy::default();

        let email = unique_email();
        let role = "merchant_admin".to_string();
        let name = Some("Sandbox User".to_string());

        let created = retry
            .execute(|| {
                let c = client.clone();
                let e = email.clone();
                let r = role.clone();
                let n = name.clone();
                async move { c.create_user(e, r, n).await }
            })
            .await
            .expect("create_user failed");

        eprintln!(
            "[scenario-u1] created user: {} email={:?} role={:?}",
            created.id, created.email, created.role
        );
        assert!(!created.id.is_empty());
        assert_eq!(created.email.as_deref(), Some(email.as_str()));
        assert_eq!(created.role.as_deref(), Some(role.as_str()));

        let list = retry
            .execute(|| {
                let c = client.clone();
                async move { c.list_users().await }
            })
            .await
            .expect("list_users failed");
        let listed = list.items.iter().find(|u| u.id == created.id);
        assert!(listed.is_some(), "created user should appear in list");

        let fetched = retry
            .execute(|| {
                let c = client.clone();
                let id = created.id.clone();
                async move { c.get_user(&id).await }
            })
            .await
            .expect("get_user failed");
        assert_eq!(fetched.id, created.id);
        assert_eq!(fetched.email.as_deref(), Some(email.as_str()));

        retry
            .execute(|| {
                let c = client.clone();
                let id = created.id.clone();
                async move { c.delete_user(&id).await }
            })
            .await
            .expect("delete_user failed");
        eprintln!("[scenario-u1] deleted user: {}", created.id);

        let list_after = retry
            .execute(|| {
                let c = client.clone();
                async move { c.list_users().await }
            })
            .await
            .expect("list_users after delete failed");
        let still_present = list_after.items.iter().any(|u| u.id == created.id);
        assert!(
            !still_present,
            "deleted user should no longer appear in list"
        );
    }
}
