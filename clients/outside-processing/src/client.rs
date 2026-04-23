use platform_sdk::{parse_response, ClientError, PlatformClient, VerifiedClaims};
use uuid::Uuid;

use crate::types::*;

pub struct OutsideProcessingClient {
    client: PlatformClient,
}

impl OutsideProcessingClient {
    pub fn new(client: PlatformClient) -> Self {
        Self { client }
    }

    // ── Orders ────────────────────────────────────────────────────────────────

    pub async fn create_order(
        &self,
        claims: &VerifiedClaims,
        body: &CreateOpOrderRequest,
    ) -> Result<OpOrder, ClientError> {
        let resp = self
            .client
            .post("/api/outside-processing/orders", body, claims)
            .await
            .map_err(ClientError::Network)?;
        parse_response(resp).await
    }

    pub async fn get_order(
        &self,
        claims: &VerifiedClaims,
        order_id: Uuid,
    ) -> Result<OpOrderDetail, ClientError> {
        let resp = self
            .client
            .get(
                &format!("/api/outside-processing/orders/{}", order_id),
                claims,
            )
            .await
            .map_err(ClientError::Network)?;
        parse_response(resp).await
    }

    pub async fn list_orders(&self, claims: &VerifiedClaims) -> Result<Vec<OpOrder>, ClientError> {
        let resp = self
            .client
            .get("/api/outside-processing/orders", claims)
            .await
            .map_err(ClientError::Network)?;
        parse_response(resp).await
    }

    pub async fn update_order(
        &self,
        claims: &VerifiedClaims,
        order_id: Uuid,
        body: &UpdateOpOrderRequest,
    ) -> Result<OpOrder, ClientError> {
        let resp = self
            .client
            .put(
                &format!("/api/outside-processing/orders/{}", order_id),
                body,
                claims,
            )
            .await
            .map_err(ClientError::Network)?;
        parse_response(resp).await
    }

    pub async fn issue_order(
        &self,
        claims: &VerifiedClaims,
        order_id: Uuid,
        body: &IssueOpOrderRequest,
    ) -> Result<OpOrder, ClientError> {
        let resp = self
            .client
            .post(
                &format!("/api/outside-processing/orders/{}/issue", order_id),
                body,
                claims,
            )
            .await
            .map_err(ClientError::Network)?;
        parse_response(resp).await
    }

    pub async fn cancel_order(
        &self,
        claims: &VerifiedClaims,
        order_id: Uuid,
        body: &CancelOpOrderRequest,
    ) -> Result<OpOrder, ClientError> {
        let resp = self
            .client
            .post(
                &format!("/api/outside-processing/orders/{}/cancel", order_id),
                body,
                claims,
            )
            .await
            .map_err(ClientError::Network)?;
        parse_response(resp).await
    }

    pub async fn close_order(
        &self,
        claims: &VerifiedClaims,
        order_id: Uuid,
    ) -> Result<OpOrder, ClientError> {
        let body = serde_json::json!({});
        let resp = self
            .client
            .post(
                &format!("/api/outside-processing/orders/{}/close", order_id),
                &body,
                claims,
            )
            .await
            .map_err(ClientError::Network)?;
        parse_response(resp).await
    }

    // ── Ship Events ───────────────────────────────────────────────────────────

    pub async fn create_ship_event(
        &self,
        claims: &VerifiedClaims,
        order_id: Uuid,
        body: &CreateShipEventRequest,
    ) -> Result<OpShipEvent, ClientError> {
        let resp = self
            .client
            .post(
                &format!("/api/outside-processing/orders/{}/ship-events", order_id),
                body,
                claims,
            )
            .await
            .map_err(ClientError::Network)?;
        parse_response(resp).await
    }

    pub async fn list_ship_events(
        &self,
        claims: &VerifiedClaims,
        order_id: Uuid,
    ) -> Result<Vec<OpShipEvent>, ClientError> {
        let resp = self
            .client
            .get(
                &format!("/api/outside-processing/orders/{}/ship-events", order_id),
                claims,
            )
            .await
            .map_err(ClientError::Network)?;
        parse_response(resp).await
    }

    // ── Return Events ─────────────────────────────────────────────────────────

    pub async fn create_return_event(
        &self,
        claims: &VerifiedClaims,
        order_id: Uuid,
        body: &CreateReturnEventRequest,
    ) -> Result<OpReturnEvent, ClientError> {
        let resp = self
            .client
            .post(
                &format!("/api/outside-processing/orders/{}/return-events", order_id),
                body,
                claims,
            )
            .await
            .map_err(ClientError::Network)?;
        parse_response(resp).await
    }

    pub async fn list_return_events(
        &self,
        claims: &VerifiedClaims,
        order_id: Uuid,
    ) -> Result<Vec<OpReturnEvent>, ClientError> {
        let resp = self
            .client
            .get(
                &format!("/api/outside-processing/orders/{}/return-events", order_id),
                claims,
            )
            .await
            .map_err(ClientError::Network)?;
        parse_response(resp).await
    }

    // ── Reviews ───────────────────────────────────────────────────────────────

    pub async fn create_review(
        &self,
        claims: &VerifiedClaims,
        order_id: Uuid,
        body: &CreateReviewRequest,
    ) -> Result<OpVendorReview, ClientError> {
        let resp = self
            .client
            .post(
                &format!("/api/outside-processing/orders/{}/reviews", order_id),
                body,
                claims,
            )
            .await
            .map_err(ClientError::Network)?;
        parse_response(resp).await
    }

    pub async fn list_reviews(
        &self,
        claims: &VerifiedClaims,
        order_id: Uuid,
    ) -> Result<Vec<OpVendorReview>, ClientError> {
        let resp = self
            .client
            .get(
                &format!("/api/outside-processing/orders/{}/reviews", order_id),
                claims,
            )
            .await
            .map_err(ClientError::Network)?;
        parse_response(resp).await
    }

    // ── Re-Identifications ────────────────────────────────────────────────────

    pub async fn create_re_identification(
        &self,
        claims: &VerifiedClaims,
        order_id: Uuid,
        body: &CreateReIdentificationRequest,
    ) -> Result<OpReIdentification, ClientError> {
        let resp = self
            .client
            .post(
                &format!(
                    "/api/outside-processing/orders/{}/re-identifications",
                    order_id
                ),
                body,
                claims,
            )
            .await
            .map_err(ClientError::Network)?;
        parse_response(resp).await
    }

    pub async fn list_re_identifications(
        &self,
        claims: &VerifiedClaims,
        order_id: Uuid,
    ) -> Result<Vec<OpReIdentification>, ClientError> {
        let resp = self
            .client
            .get(
                &format!(
                    "/api/outside-processing/orders/{}/re-identifications",
                    order_id
                ),
                claims,
            )
            .await
            .map_err(ClientError::Network)?;
        parse_response(resp).await
    }

    // ── Labels ────────────────────────────────────────────────────────────────

    pub async fn list_status_labels(
        &self,
        claims: &VerifiedClaims,
    ) -> Result<Vec<OpStatusLabel>, ClientError> {
        let resp = self
            .client
            .get("/api/outside-processing/status-labels", claims)
            .await
            .map_err(ClientError::Network)?;
        parse_response(resp).await
    }

    pub async fn upsert_status_label(
        &self,
        claims: &VerifiedClaims,
        canonical_status: &str,
        body: &UpsertStatusLabelRequest,
    ) -> Result<OpStatusLabel, ClientError> {
        let resp = self
            .client
            .put(
                &format!("/api/outside-processing/status-labels/{}", canonical_status),
                body,
                claims,
            )
            .await
            .map_err(ClientError::Network)?;
        parse_response(resp).await
    }

    pub async fn list_service_type_labels(
        &self,
        claims: &VerifiedClaims,
    ) -> Result<Vec<OpServiceTypeLabel>, ClientError> {
        let resp = self
            .client
            .get("/api/outside-processing/service-type-labels", claims)
            .await
            .map_err(ClientError::Network)?;
        parse_response(resp).await
    }

    pub async fn upsert_service_type_label(
        &self,
        claims: &VerifiedClaims,
        service_type: &str,
        body: &UpsertServiceTypeLabelRequest,
    ) -> Result<OpServiceTypeLabel, ClientError> {
        let resp = self
            .client
            .put(
                &format!(
                    "/api/outside-processing/service-type-labels/{}",
                    service_type
                ),
                body,
                claims,
            )
            .await
            .map_err(ClientError::Network)?;
        parse_response(resp).await
    }
}

impl platform_sdk::PlatformService for OutsideProcessingClient {
    const SERVICE_NAME: &'static str = "outside-processing";
    fn from_platform_client(client: PlatformClient) -> Self {
        Self::new(client)
    }
}
