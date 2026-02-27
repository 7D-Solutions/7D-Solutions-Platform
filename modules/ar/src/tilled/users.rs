use super::error::TilledError;
use super::types::{ListResponse, User};
use super::TilledClient;
use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct CreateUserRequest {
    pub email: String,
    pub role: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

impl TilledClient {
    /// Create a user in the current account scope.
    pub async fn create_user(
        &self,
        email: String,
        role: String,
        name: Option<String>,
    ) -> Result<User, TilledError> {
        let request = CreateUserRequest { email, role, name };
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
    use super::CreateUserRequest;

    #[test]
    fn create_user_payload_serializes_expected_fields() {
        let payload = CreateUserRequest {
            email: "user@example.com".to_string(),
            role: "merchant_admin".to_string(),
            name: Some("User Test".to_string()),
        };

        let value = serde_json::to_value(payload).unwrap();
        assert_eq!(value.get("email").unwrap(), "user@example.com");
        assert_eq!(value.get("role").unwrap(), "merchant_admin");
        assert_eq!(value.get("name").unwrap(), "User Test");
    }
}
