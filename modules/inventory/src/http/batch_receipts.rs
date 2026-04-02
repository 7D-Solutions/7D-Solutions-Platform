use axum::{extract::State, http::StatusCode, response::IntoResponse, Json};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::{
    domain::receipt_service::{self, ReceiptRequest, ReceiptResult},
    AppState,
};

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct BatchReceiptRequest {
    pub receipts: Vec<ReceiptRequest>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct BatchReceiptResponse {
    pub results: Vec<BatchReceiptItemResult>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
#[serde(untagged)] // Allows for different types in the same Vec
pub enum BatchReceiptItemResult {
    Success(Box<ReceiptResult>),
    Error {
        #[serde(rename = "itemId")]
        item_id: Uuid,
        #[serde(rename = "errorMessage")]
        error_message: String,
    },
}

/// HTTP handler for batch stock receipts.
///
/// This endpoint allows submitting multiple stock receipt requests in a single call.
/// Each individual receipt will be processed independently, and the response will
/// contain a list of results (success or error) for each submitted receipt.
#[utoipa::path(
    post,
    path = "/api/inventory/batch-receipts",
    tag = "Receipts",
    request_body = BatchReceiptRequest,
    responses(
        (status = 200, description = "Batch receipt results", body = BatchReceiptResponse),
    ),
    security(("bearer" = [])),
)]
pub async fn post_batch_receipts(
    State(app_state): State<AppState>,
    Json(req): Json<BatchReceiptRequest>,
) -> impl IntoResponse {
    tracing::info!(
        "Received batch receipt request with {} items",
        req.receipts.len()
    );

    let mut results = Vec::with_capacity(req.receipts.len());

    for receipt_req in req.receipts {
        let item_id = receipt_req.item_id;
        match receipt_service::process_receipt(&app_state.pool, &receipt_req, None).await {
            Ok((result, _is_replay)) => {
                results.push(BatchReceiptItemResult::Success(Box::new(result)));
            }
            Err(e) => {
                tracing::error!(
                    "Error processing batch receipt for item {}: {:?}",
                    item_id,
                    e
                );
                results.push(BatchReceiptItemResult::Error {
                    item_id,
                    error_message: e.to_string(),
                });
            }
        }
    }

    (StatusCode::OK, Json(BatchReceiptResponse { results })).into_response()
}
