//! Typed HTTP client for Sales Orders endpoints.

use crate::types::*;
use platform_sdk::{build_query_url, parse_response, ClientError, PlatformClient, VerifiedClaims};
use uuid::Uuid;

pub struct SalesOrdersClient {
    pub(crate) client: PlatformClient,
}

impl SalesOrdersClient {
    pub fn new(client: PlatformClient) -> Self {
        Self { client }
    }

    /// POST `/api/so/orders`
    pub async fn create_order(
        &self,
        claims: &VerifiedClaims,
        body: &CreateOrderRequest,
    ) -> Result<SalesOrder, ClientError> {
        let resp = self
            .client
            .post("/api/so/orders", body, claims)
            .await
            .map_err(ClientError::Network)?;
        parse_response(resp).await
    }

    /// GET `/api/so/orders`
    pub async fn list_orders(
        &self,
        claims: &VerifiedClaims,
        query: &ListOrdersQuery,
    ) -> Result<Vec<SalesOrder>, ClientError> {
        let url = build_query_url("/api/so/orders", query)?;
        let resp = self
            .client
            .get(&url, claims)
            .await
            .map_err(ClientError::Network)?;
        parse_response(resp).await
    }

    /// GET `/api/so/orders/{order_id}`
    pub async fn get_order(
        &self,
        claims: &VerifiedClaims,
        order_id: Uuid,
    ) -> Result<SalesOrderWithLines, ClientError> {
        let path = format!("/api/so/orders/{}", order_id);
        let resp = self
            .client
            .get(&path, claims)
            .await
            .map_err(ClientError::Network)?;
        parse_response(resp).await
    }

    /// PUT `/api/so/orders/{order_id}`
    pub async fn update_order(
        &self,
        claims: &VerifiedClaims,
        order_id: Uuid,
        body: &UpdateOrderRequest,
    ) -> Result<SalesOrder, ClientError> {
        let path = format!("/api/so/orders/{}", order_id);
        let resp = self
            .client
            .put(&path, body, claims)
            .await
            .map_err(ClientError::Network)?;
        parse_response(resp).await
    }

    /// POST `/api/so/orders/{order_id}/book`
    pub async fn book_order(
        &self,
        claims: &VerifiedClaims,
        order_id: Uuid,
    ) -> Result<SalesOrder, ClientError> {
        let path = format!("/api/so/orders/{}/book", order_id);
        let resp = self
            .client
            .post(&path, &serde_json::json!({}), claims)
            .await
            .map_err(ClientError::Network)?;
        parse_response(resp).await
    }

    /// POST `/api/so/orders/{order_id}/cancel`
    pub async fn cancel_order(
        &self,
        claims: &VerifiedClaims,
        order_id: Uuid,
    ) -> Result<SalesOrder, ClientError> {
        let path = format!("/api/so/orders/{}/cancel", order_id);
        let resp = self
            .client
            .post(&path, &serde_json::json!({}), claims)
            .await
            .map_err(ClientError::Network)?;
        parse_response(resp).await
    }

    /// POST `/api/so/orders/{order_id}/lines`
    pub async fn add_line(
        &self,
        claims: &VerifiedClaims,
        order_id: Uuid,
        body: &CreateOrderLineRequest,
    ) -> Result<SalesOrder, ClientError> {
        let path = format!("/api/so/orders/{}/lines", order_id);
        let resp = self
            .client
            .post(&path, body, claims)
            .await
            .map_err(ClientError::Network)?;
        parse_response(resp).await
    }
}
