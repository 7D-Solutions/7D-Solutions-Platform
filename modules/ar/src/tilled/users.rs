use super::error::TilledError;
use super::types::{ListResponse, User};
use super::TilledClient;
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize)]
pub struct CreateUserRequest {
    pub email: String,
    pub role: String,
    pub password: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct UpdateUserRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

/// Response from impersonating a user.
#[derive(Debug, Clone, Deserialize)]
pub struct UserImpersonation {
    #[serde(default)]
    pub access_token: Option<String>,
    #[serde(default)]
    pub token_type: Option<String>,
    #[serde(default)]
    pub expires_in: Option<i64>,
    #[serde(default)]
    pub user: Option<User>,
}

impl TilledClient {
    /// Create a user in the current account scope.
    pub async fn create_user(
        &self,
        email: String,
        role: String,
        password: String,
        name: Option<String>,
    ) -> Result<User, TilledError> {
        let request = CreateUserRequest {
            email,
            role,
            password,
            name,
        };
        self.post("/v1/users", &request).await
    }

    /// List users in the current account scope.
    pub async fn list_users(&self) -> Result<ListResponse<User>, TilledError> {
        self.get("/v1/users", None).await
    }

    /// Get a user by ID.
    pub async fn get_user(&self, user_id: &str) -> Result<User, TilledError> {
        let path = format!("/v1/users/{user_id}");
        self.get(&path, None).await
    }

    /// Update a user by ID.
    pub async fn update_user(
        &self,
        user_id: &str,
        request: UpdateUserRequest,
    ) -> Result<User, TilledError> {
        let path = format!("/v1/users/{user_id}");
        self.patch(&path, &request).await
    }

    /// Impersonate a user — returns an access token for acting as that user.
    pub async fn impersonate_user(
        &self,
        user_id: &str,
    ) -> Result<UserImpersonation, TilledError> {
        let path = format!("/v1/users/{user_id}/impersonate");
        self.post(&path, &serde_json::json!({})).await
    }

    /// Reset a user's MFA enrollments.
    /// Requires `verification_details` explaining why MFA is being reset.
    /// May return 204 empty body on success.
    pub async fn reset_user_mfa(
        &self,
        user_id: &str,
        verification_details: &str,
    ) -> Result<(), TilledError> {
        let path = format!("/v1/users/{user_id}/reset-mfa");
        let url = format!("{}{}", self.config.base_path, path);
        let response = self
            .http_client
            .post(&url)
            .headers(self.build_auth_headers()?)
            .json(&serde_json::json!({ "verification_details": verification_details }))
            .send()
            .await
            .map_err(|e| TilledError::HttpError(e.to_string()))?;

        if response.status().is_success() {
            Ok(())
        } else {
            let status_code = response.status().as_u16();
            let message = response
                .text()
                .await
                .unwrap_or_else(|_| "Unable to read error response".to_string());
            Err(TilledError::ApiError {
                status_code,
                message,
            })
        }
    }

    /// Unlock a locked user account.
    /// May return 204 empty body on success.
    pub async fn unlock_user(&self, user_id: &str) -> Result<(), TilledError> {
        let path = format!("/v1/users/{user_id}/unlock");
        let url = format!("{}{}", self.config.base_path, path);
        let response = self
            .http_client
            .post(&url)
            .headers(self.build_auth_headers()?)
            .json(&serde_json::json!({}))
            .send()
            .await
            .map_err(|e| TilledError::HttpError(e.to_string()))?;

        if response.status().is_success() {
            Ok(())
        } else {
            let status_code = response.status().as_u16();
            let message = response
                .text()
                .await
                .unwrap_or_else(|_| "Unable to read error response".to_string());
            Err(TilledError::ApiError {
                status_code,
                message,
            })
        }
    }

    /// Delete a user by ID.
    /// Tilled may return 204 with empty body for successful deletes.
    pub async fn delete_user(&self, user_id: &str) -> Result<(), TilledError> {
        let path = format!("/v1/users/{user_id}");
        let url = format!("{}{}", self.config.base_path, path);
        let response = self
            .http_client
            .delete(&url)
            .headers(self.build_auth_headers()?)
            .send()
            .await
            .map_err(|e| TilledError::HttpError(e.to_string()))?;

        if response.status().is_success() {
            Ok(())
        } else {
            let status_code = response.status().as_u16();
            let message = response
                .text()
                .await
                .unwrap_or_else(|_| "Unable to read error response".to_string());
            Err(TilledError::ApiError {
                status_code,
                message,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{CreateUserRequest, UpdateUserRequest};

    #[test]
    fn create_user_payload_serializes_expected_fields() {
        let payload = CreateUserRequest {
            email: "user@example.com".to_string(),
            role: "merchant_admin".to_string(),
            password: "Test1234".to_string(),
            name: Some("User Test".to_string()),
        };

        let value = serde_json::to_value(payload).unwrap();
        assert_eq!(value.get("email").unwrap(), "user@example.com");
        assert_eq!(value.get("role").unwrap(), "merchant_admin");
        assert_eq!(value.get("password").unwrap(), "Test1234");
        assert_eq!(value.get("name").unwrap(), "User Test");
    }

    #[test]
    fn update_user_payload_omits_none_fields() {
        let payload = UpdateUserRequest { name: None };
        let value = serde_json::to_value(payload).unwrap();
        assert!(value.get("name").is_none());

        let payload = UpdateUserRequest {
            name: Some("Updated User".to_string()),
        };
        let value = serde_json::to_value(payload).unwrap();
        assert_eq!(value.get("name").unwrap(), "Updated User");
    }
}
