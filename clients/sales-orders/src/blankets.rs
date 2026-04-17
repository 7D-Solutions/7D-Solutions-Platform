//! Typed HTTP client for Blanket Order endpoints.

use crate::types::*;
use platform_sdk::{parse_response, ClientError, PlatformClient, VerifiedClaims};
use uuid::Uuid;

pub struct BlanketsClient {
    pub(crate) client: PlatformClient,
}

impl BlanketsClient {
    pub fn new(client: PlatformClient) -> Self {
        Self { client }
    }

    /// POST `/api/so/blankets`
    pub async fn create_blanket(
        &self,
        claims: &VerifiedClaims,
        body: &CreateBlanketRequest,
    ) -> Result<BlanketOrder, ClientError> {
        let resp = self
            .client
            .post("/api/so/blankets", claims, body)
            .await
            .map_err(ClientError::Network)?;
        parse_response(resp).await
    }

    /// GET `/api/so/blankets/{blanket_id}`
    pub async fn get_blanket(
        &self,
        claims: &VerifiedClaims,
        blanket_id: Uuid,
    ) -> Result<BlanketOrder, ClientError> {
        let path = format!("/api/so/blankets/{}", blanket_id);
        let resp = self
            .client
            .get(&path, claims)
            .await
            .map_err(ClientError::Network)?;
        parse_response(resp).await
    }

    /// POST `/api/so/blankets/{blanket_id}/activate`
    pub async fn activate_blanket(
        &self,
        claims: &VerifiedClaims,
        blanket_id: Uuid,
    ) -> Result<BlanketOrder, ClientError> {
        let path = format!("/api/so/blankets/{}/activate", blanket_id);
        let resp = self
            .client
            .post(&path, claims, &serde_json::json!({}))
            .await
            .map_err(ClientError::Network)?;
        parse_response(resp).await
    }

    /// POST `/api/so/blankets/{blanket_id}/releases`
    pub async fn create_release(
        &self,
        claims: &VerifiedClaims,
        blanket_id: Uuid,
        body: &CreateReleaseRequest,
    ) -> Result<BlanketOrderRelease, ClientError> {
        let path = format!("/api/so/blankets/{}/releases", blanket_id);
        let resp = self
            .client
            .post(&path, claims, body)
            .await
            .map_err(ClientError::Network)?;
        parse_response(resp).await
    }
}
