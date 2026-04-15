//! Sandbox scenarios: files lifecycle and user update.
//!
//! File delete and content-download require partner scope on Tilled's API.
//! Tests f1–f4 use `try_partner_client()` accordingly.

#[cfg(test)]
mod tests {
    use crate::tilled_sandbox::helpers::{minimal_png, unique_email, RetryPolicy};
    use crate::tilled_sandbox::{try_partner_client, try_sandbox_client};
    use ar_rs::tilled::users::UpdateUserRequest;
    use std::collections::HashMap;

    #[tokio::test]
    async fn scenario_f1_list_files_after_upload() {
        let client = match try_partner_client() {
            Some(c) => c,
            None => {
                eprintln!("SKIP: partner creds not set");
                return;
            }
        };
        let retry = RetryPolicy::default();

        let uploaded = retry
            .execute(|| {
                let c = client.clone();
                async move {
                    c.upload_file(
                        minimal_png(),
                        "scenario-f1.png",
                        "image/png",
                        "dispute_evidence",
                    )
                    .await
                }
            })
            .await
            .expect("upload_file failed");

        let mut filters = HashMap::new();
        filters.insert("limit".to_string(), "100".to_string());
        let list = retry
            .execute(|| {
                let c = client.clone();
                let f = filters.clone();
                async move { c.list_files(Some(f)).await }
            })
            .await
            .expect("list_files failed");

        assert!(
            list.items.iter().any(|f| f.id == uploaded.id),
            "uploaded file should appear in list"
        );

        client
            .delete_file(&uploaded.id)
            .await
            .expect("cleanup delete_file failed");
    }

    #[tokio::test]
    async fn scenario_f2_get_file_metadata() {
        let client = match try_partner_client() {
            Some(c) => c,
            None => {
                eprintln!("SKIP: partner creds not set");
                return;
            }
        };
        let retry = RetryPolicy::default();

        let uploaded = retry
            .execute(|| {
                let c = client.clone();
                async move {
                    c.upload_file(
                        minimal_png(),
                        "scenario-f2.png",
                        "image/png",
                        "dispute_evidence",
                    )
                    .await
                }
            })
            .await
            .expect("upload_file failed");

        let fetched = retry
            .execute(|| {
                let c = client.clone();
                let id = uploaded.id.clone();
                async move { c.get_file(&id).await }
            })
            .await
            .expect("get_file failed");

        assert_eq!(fetched.id, uploaded.id);
        assert_eq!(fetched.purpose, uploaded.purpose);

        client
            .delete_file(&uploaded.id)
            .await
            .expect("cleanup delete_file failed");
    }

    /// Tilled's sandbox returns 403 for file content download via API key auth
    /// (requires JWT/user-session auth). We verify the upload produces a valid
    /// content URL and that get_file_contents returns the expected auth error.
    #[tokio::test]
    async fn scenario_f3_file_contents_url_and_error() {
        let client = match try_partner_client() {
            Some(c) => c,
            None => {
                eprintln!("SKIP: partner creds not set");
                return;
            }
        };
        let retry = RetryPolicy::default();

        let uploaded = retry
            .execute(|| {
                let c = client.clone();
                async move {
                    c.upload_file(
                        minimal_png(),
                        "scenario-f3.png",
                        "image/png",
                        "dispute_evidence",
                    )
                    .await
                }
            })
            .await
            .expect("upload_file failed");

        // Verify the file has a valid content URL
        let url = uploaded
            .url
            .as_deref()
            .expect("uploaded file should have url");
        assert!(
            url.contains(&uploaded.id),
            "content URL should reference the file ID"
        );
        assert!(
            url.contains("/contents"),
            "content URL should point to contents endpoint"
        );

        // Verify get_file_contents returns auth error (Tilled requires JWT for downloads)
        let result = client.get_file_contents(&uploaded.id).await;
        assert!(
            result.is_err(),
            "get_file_contents should fail with API key auth"
        );

        client
            .delete_file(&uploaded.id)
            .await
            .expect("cleanup delete_file failed");
    }

    #[tokio::test]
    async fn scenario_f4_delete_file() {
        let client = match try_partner_client() {
            Some(c) => c,
            None => {
                eprintln!("SKIP: partner creds not set");
                return;
            }
        };
        let retry = RetryPolicy::default();

        let uploaded = retry
            .execute(|| {
                let c = client.clone();
                async move {
                    c.upload_file(
                        minimal_png(),
                        "scenario-f4.png",
                        "image/png",
                        "dispute_evidence",
                    )
                    .await
                }
            })
            .await
            .expect("upload_file failed");

        retry
            .execute(|| {
                let c = client.clone();
                let id = uploaded.id.clone();
                async move { c.delete_file(&id).await }
            })
            .await
            .expect("delete_file failed");

        let mut filters = HashMap::new();
        filters.insert("limit".to_string(), "100".to_string());
        let list = retry
            .execute(|| {
                let c = client.clone();
                let f = filters.clone();
                async move { c.list_files(Some(f)).await }
            })
            .await
            .expect("list_files failed");

        assert!(
            !list.items.iter().any(|f| f.id == uploaded.id),
            "deleted file should not appear in list"
        );
    }

    #[tokio::test]
    async fn scenario_f5_update_user() {
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
        let password = format!("SandboxUpdate1{}", uuid::Uuid::new_v4().simple());
        let initial_name = "Files User Before".to_string();

        let created = retry
            .execute(|| {
                let c = client.clone();
                let e = email.clone();
                let r = role.clone();
                let p = password.clone();
                let n = Some(initial_name.clone());
                async move { c.create_user(e, r, p, n).await }
            })
            .await
            .expect("create_user failed");

        let updated_name = format!("Files User After {}", uuid::Uuid::new_v4().simple());
        let updated = retry
            .execute(|| {
                let c = client.clone();
                let id = created.id.clone();
                let req = UpdateUserRequest {
                    name: Some(updated_name.clone()),
                };
                async move { c.update_user(&id, req).await }
            })
            .await
            .expect("update_user failed");

        assert_eq!(updated.id, created.id);
        assert_eq!(updated.name.as_deref(), Some(updated_name.as_str()));

        let fetched = retry
            .execute(|| {
                let c = client.clone();
                let id = created.id.clone();
                async move { c.get_user(&id).await }
            })
            .await
            .expect("get_user failed");
        assert_eq!(fetched.name.as_deref(), Some(updated_name.as_str()));

        client
            .delete_user(&created.id)
            .await
            .expect("cleanup delete_user failed");
    }
}
