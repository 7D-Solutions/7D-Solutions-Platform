use super::error::TilledError;
use super::types::ListResponse;
use super::TilledClient;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize)]
pub struct UserInvitation {
    pub id: String,
    #[serde(default)]
    pub email: Option<String>,
    #[serde(default)]
    pub role: Option<String>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub created_at: Option<String>,
    #[serde(default)]
    pub updated_at: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct CreateUserInvitationRequest {
    pub email: String,
    pub role: String,
}

impl TilledClient {
    /// Create a user invitation.
    pub async fn create_user_invitation(
        &self,
        email: String,
        role: String,
    ) -> Result<UserInvitation, TilledError> {
        let request = CreateUserInvitationRequest { email, role };
        self.post("/v1/user-invitations", &request).await
    }

    /// List all user invitations.
    pub async fn list_user_invitations(&self) -> Result<ListResponse<UserInvitation>, TilledError> {
        self.get("/v1/user-invitations", None).await
    }

    /// Get a user invitation by ID.
    pub async fn get_user_invitation(
        &self,
        invitation_id: &str,
    ) -> Result<UserInvitation, TilledError> {
        let path = format!("/v1/user-invitations/{invitation_id}");
        self.get(&path, None).await
    }

    /// Delete a user invitation.
    pub async fn delete_user_invitation(&self, invitation_id: &str) -> Result<(), TilledError> {
        let path = format!("/v1/user-invitations/{invitation_id}");
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

    /// Resend a user invitation.
    pub async fn resend_user_invitation(
        &self,
        invitation_id: &str,
    ) -> Result<UserInvitation, TilledError> {
        let path = format!("/v1/user-invitations/{invitation_id}/resend");
        self.post(&path, &serde_json::json!({})).await
    }

    /// Check a user invitation by ID (public endpoint).
    pub async fn check_user_invitation(
        &self,
        invitation_id: &str,
    ) -> Result<UserInvitation, TilledError> {
        let path = format!("/v1/user-invitations/check/{invitation_id}");
        self.get(&path, None).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_user_invitation_payload_serializes() {
        let payload = CreateUserInvitationRequest {
            email: "invite@example.com".to_string(),
            role: "merchant_admin".to_string(),
        };
        let value = serde_json::to_value(payload).expect("test fixture");
        assert_eq!(value.get("email").expect("test fixture"), "invite@example.com");
        assert_eq!(value.get("role").expect("test fixture"), "merchant_admin");
    }

    #[test]
    fn user_invitation_deserializes_with_optional_fields() {
        let value = serde_json::json!({
            "id": "ui_123",
            "email": "invite@example.com",
            "role": "merchant_admin",
            "status": "pending"
        });
        let invite: UserInvitation = serde_json::from_value(value).expect("test fixture");
        assert_eq!(invite.id, "ui_123");
        assert_eq!(invite.email.as_deref(), Some("invite@example.com"));
        assert_eq!(invite.status.as_deref(), Some("pending"));
    }

    #[test]
    fn user_invitation_deserializes_minimal() {
        let value = serde_json::json!({ "id": "ui_456" });
        let invite: UserInvitation = serde_json::from_value(value).expect("test fixture");
        assert_eq!(invite.id, "ui_456");
        assert!(invite.email.is_none());
    }
}
