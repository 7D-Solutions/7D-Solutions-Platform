pub mod types;

pub use types::*;

use platform_sdk::{build_query_url, parse_response, ClientError, PlatformClient, VerifiedClaims};
use uuid::Uuid;

pub struct ShopFloorGatesClient {
    client: PlatformClient,
}

impl ShopFloorGatesClient {
    pub fn new(client: PlatformClient) -> Self {
        Self { client }
    }

    // ── Holds ─────────────────────────────────────────────────────────────────

    pub async fn place_hold(
        &self,
        claims: &VerifiedClaims,
        body: &PlaceHoldRequest,
    ) -> Result<TravelerHold, ClientError> {
        let resp = self
            .client
            .post("/api/sfg/holds", body, claims)
            .await
            .map_err(ClientError::Network)?;
        parse_response(resp).await
    }

    pub async fn list_holds(
        &self,
        claims: &VerifiedClaims,
        query: &ListHoldsQuery,
    ) -> Result<Vec<TravelerHold>, ClientError> {
        let url = build_query_url("/api/sfg/holds", query)?;
        let resp = self
            .client
            .get(&url, claims)
            .await
            .map_err(ClientError::Network)?;
        parse_response(resp).await
    }

    pub async fn get_hold(
        &self,
        claims: &VerifiedClaims,
        hold_id: Uuid,
    ) -> Result<TravelerHold, ClientError> {
        let url = format!("/api/sfg/holds/{}", hold_id);
        let resp = self
            .client
            .get(&url, claims)
            .await
            .map_err(ClientError::Network)?;
        parse_response(resp).await
    }

    pub async fn release_hold(
        &self,
        claims: &VerifiedClaims,
        hold_id: Uuid,
        body: &ReleaseHoldRequest,
    ) -> Result<TravelerHold, ClientError> {
        let url = format!("/api/sfg/holds/{}/release", hold_id);
        let resp = self
            .client
            .post(&url, body, claims)
            .await
            .map_err(ClientError::Network)?;
        parse_response(resp).await
    }

    pub async fn cancel_hold(
        &self,
        claims: &VerifiedClaims,
        hold_id: Uuid,
        body: &CancelHoldRequest,
    ) -> Result<TravelerHold, ClientError> {
        let url = format!("/api/sfg/holds/{}/cancel", hold_id);
        let resp = self
            .client
            .post(&url, body, claims)
            .await
            .map_err(ClientError::Network)?;
        parse_response(resp).await
    }

    // ── Handoffs ──────────────────────────────────────────────────────────────

    pub async fn initiate_handoff(
        &self,
        claims: &VerifiedClaims,
        body: &InitiateHandoffRequest,
    ) -> Result<OperationHandoff, ClientError> {
        let resp = self
            .client
            .post("/api/sfg/handoffs", body, claims)
            .await
            .map_err(ClientError::Network)?;
        parse_response(resp).await
    }

    pub async fn list_handoffs(
        &self,
        claims: &VerifiedClaims,
        query: &ListHandoffsQuery,
    ) -> Result<Vec<OperationHandoff>, ClientError> {
        let url = build_query_url("/api/sfg/handoffs", query)?;
        let resp = self
            .client
            .get(&url, claims)
            .await
            .map_err(ClientError::Network)?;
        parse_response(resp).await
    }

    pub async fn get_handoff(
        &self,
        claims: &VerifiedClaims,
        handoff_id: Uuid,
    ) -> Result<OperationHandoff, ClientError> {
        let url = format!("/api/sfg/handoffs/{}", handoff_id);
        let resp = self
            .client
            .get(&url, claims)
            .await
            .map_err(ClientError::Network)?;
        parse_response(resp).await
    }

    pub async fn accept_handoff(
        &self,
        claims: &VerifiedClaims,
        handoff_id: Uuid,
        body: &AcceptHandoffRequest,
    ) -> Result<OperationHandoff, ClientError> {
        let url = format!("/api/sfg/handoffs/{}/accept", handoff_id);
        let resp = self
            .client
            .post(&url, body, claims)
            .await
            .map_err(ClientError::Network)?;
        parse_response(resp).await
    }

    pub async fn reject_handoff(
        &self,
        claims: &VerifiedClaims,
        handoff_id: Uuid,
        body: &RejectHandoffRequest,
    ) -> Result<OperationHandoff, ClientError> {
        let url = format!("/api/sfg/handoffs/{}/reject", handoff_id);
        let resp = self
            .client
            .post(&url, body, claims)
            .await
            .map_err(ClientError::Network)?;
        parse_response(resp).await
    }

    pub async fn cancel_handoff(
        &self,
        claims: &VerifiedClaims,
        handoff_id: Uuid,
        body: &CancelHandoffRequest,
    ) -> Result<OperationHandoff, ClientError> {
        let url = format!("/api/sfg/handoffs/{}/cancel", handoff_id);
        let resp = self
            .client
            .post(&url, body, claims)
            .await
            .map_err(ClientError::Network)?;
        parse_response(resp).await
    }

    // ── Verifications ─────────────────────────────────────────────────────────

    pub async fn create_verification(
        &self,
        claims: &VerifiedClaims,
        body: &CreateVerificationRequest,
    ) -> Result<OperationStartVerification, ClientError> {
        let resp = self
            .client
            .post("/api/sfg/verifications", body, claims)
            .await
            .map_err(ClientError::Network)?;
        parse_response(resp).await
    }

    pub async fn list_verifications(
        &self,
        claims: &VerifiedClaims,
        query: &ListVerificationsQuery,
    ) -> Result<Vec<OperationStartVerification>, ClientError> {
        let url = build_query_url("/api/sfg/verifications", query)?;
        let resp = self
            .client
            .get(&url, claims)
            .await
            .map_err(ClientError::Network)?;
        parse_response(resp).await
    }

    pub async fn get_verification(
        &self,
        claims: &VerifiedClaims,
        verification_id: Uuid,
    ) -> Result<OperationStartVerification, ClientError> {
        let url = format!("/api/sfg/verifications/{}", verification_id);
        let resp = self
            .client
            .get(&url, claims)
            .await
            .map_err(ClientError::Network)?;
        parse_response(resp).await
    }

    pub async fn operator_confirm(
        &self,
        claims: &VerifiedClaims,
        verification_id: Uuid,
        body: &OperatorConfirmRequest,
    ) -> Result<OperationStartVerification, ClientError> {
        let url = format!(
            "/api/sfg/verifications/{}/operator-confirm",
            verification_id
        );
        let resp = self
            .client
            .post(&url, body, claims)
            .await
            .map_err(ClientError::Network)?;
        parse_response(resp).await
    }

    pub async fn verify(
        &self,
        claims: &VerifiedClaims,
        verification_id: Uuid,
        body: &VerifyRequest,
    ) -> Result<OperationStartVerification, ClientError> {
        let url = format!("/api/sfg/verifications/{}/verify", verification_id);
        let resp = self
            .client
            .post(&url, body, claims)
            .await
            .map_err(ClientError::Network)?;
        parse_response(resp).await
    }

    pub async fn skip_verification(
        &self,
        claims: &VerifiedClaims,
        verification_id: Uuid,
        body: &SkipVerificationRequest,
    ) -> Result<OperationStartVerification, ClientError> {
        let url = format!("/api/sfg/verifications/{}/skip", verification_id);
        let resp = self
            .client
            .post(&url, body, claims)
            .await
            .map_err(ClientError::Network)?;
        parse_response(resp).await
    }

    // ── Signoffs ──────────────────────────────────────────────────────────────

    pub async fn record_signoff(
        &self,
        claims: &VerifiedClaims,
        body: &RecordSignoffRequest,
    ) -> Result<Signoff, ClientError> {
        let resp = self
            .client
            .post("/api/sfg/signoffs", body, claims)
            .await
            .map_err(ClientError::Network)?;
        parse_response(resp).await
    }

    pub async fn list_signoffs(
        &self,
        claims: &VerifiedClaims,
        query: &ListSignoffsQuery,
    ) -> Result<Vec<Signoff>, ClientError> {
        let url = build_query_url("/api/sfg/signoffs", query)?;
        let resp = self
            .client
            .get(&url, claims)
            .await
            .map_err(ClientError::Network)?;
        parse_response(resp).await
    }

    pub async fn get_signoff(
        &self,
        claims: &VerifiedClaims,
        signoff_id: Uuid,
    ) -> Result<Signoff, ClientError> {
        let url = format!("/api/sfg/signoffs/{}", signoff_id);
        let resp = self
            .client
            .get(&url, claims)
            .await
            .map_err(ClientError::Network)?;
        parse_response(resp).await
    }

    // ── Labels ────────────────────────────────────────────────────────────────

    pub async fn upsert_label(
        &self,
        claims: &VerifiedClaims,
        table: &str,
        body: &UpsertLabelRequest,
    ) -> Result<StatusLabel, ClientError> {
        let url = format!("/api/sfg/labels/{}", table);
        let resp = self
            .client
            .put(&url, body, claims)
            .await
            .map_err(ClientError::Network)?;
        parse_response(resp).await
    }

    pub async fn list_labels(
        &self,
        claims: &VerifiedClaims,
        table: &str,
    ) -> Result<Vec<StatusLabel>, ClientError> {
        let url = format!("/api/sfg/labels/{}", table);
        let resp = self
            .client
            .get(&url, claims)
            .await
            .map_err(ClientError::Network)?;
        parse_response(resp).await
    }

    pub async fn delete_label(
        &self,
        claims: &VerifiedClaims,
        table: &str,
        id: Uuid,
    ) -> Result<(), ClientError> {
        let url = format!("/api/sfg/labels/{}/{}", table, id);
        let resp = self
            .client
            .delete(&url, claims)
            .await
            .map_err(ClientError::Network)?;
        parse_response(resp).await
    }
}
